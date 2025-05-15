use std::{fs, process::ExitStatus};

use anyhow::Context;
use rusqlite::OptionalExtension;
use time::OffsetDateTime;
use tracing::{instrument, trace};
use walkdir::WalkDir;

mod cache;

#[instrument]
pub async fn build(argv: &[String]) -> anyhow::Result<ExitStatus> {
    // Get current working directory.
    let workspace_path = std::env::current_dir().context("could not get current directory")?;

    // Initialize the workspace cache.
    //
    // TODO: All of these failures should be non-fatal and should not block us
    // from shelling out to `cargo build`.
    let mut workspace_cache = cache::WorkspaceCache::new(&workspace_path)
        .context("could not initialize workspace cache")?;

    // Record this invocation.
    let tx = workspace_cache
        .metadb
        .transaction()
        .context("could not start cache transaction")?;
    let exit_status = {
        let invocation_id = tx
            .query_row(
                "INSERT INTO invocation (argv, start_time) VALUES (?1, ?2) RETURNING invocation_id",
                (argv.join(" "), OffsetDateTime::now_utc()),
                |row| row.get::<_, i64>(0),
            )
            .context("could not record hurry invocation in cache")?;
        trace!(?invocation_id, "recorded invocation");

        // Record the source files used in this invocation.
        //
        // FIXME: Relying on the source files to be in `src/` is a convention.
        // In theory, we should actually be shelling out to `rustc`'s crate
        // loader to understand the actual module inclusion logic. We may also
        // need to intercept or replicate cargo's extern flag-passing behavior.
        //
        // TODO: Should we parallelize this? Will `jwalk` improve performance
        // here?
        let check_source_file = &mut tx
            .prepare("SELECT source_file_id FROM source_file WHERE b3sum = ?1")
            .context("could not prepare source file check")?;
        let insert_source_file = &mut tx
            .prepare(
                "INSERT INTO source_file (b3sum) VALUES (?1) ON CONFLICT DO NOTHING RETURNING source_file_id",
            )
            .context("could not prepare source file insert")?;
        let insert_invocation_source_file = &mut tx
            .prepare(
                "INSERT INTO invocation_source_file (invocation_id, source_file_id, path, mtime) VALUES (?1, ?2, ?3, ?4)",
            )
            .context("could not prepare source file invocation insert")?;
        for entry in WalkDir::new(workspace_path.join("src")) {
            let entry = entry.context("could not walk source directory")?;
            if entry.file_type().is_file() {
                let source_path = entry.path();
                let source_mtime: OffsetDateTime = entry
                    .metadata()
                    .context("could not get file metadata")?
                    .modified()
                    .context("could not get file mtime")?
                    .into();
                // TODO: Improve performance here? `blake3` provides both
                // streaming and parallel APIs.
                let source_b3sum = {
                    let source_bytes =
                        fs::read(source_path).context("could not read source file")?;
                    blake3::hash(&source_bytes).to_hex().to_string()
                };
                let source_file_id = match check_source_file
                    .query_row((&source_b3sum,), |row| row.get::<_, i64>(0))
                    .optional()
                    .context("could not check source file")?
                {
                    Some(rid) => rid,
                    None => insert_source_file
                        .query_row((&source_b3sum,), |row| row.get::<_, i64>(0))
                        .context("could not insert source file")?,
                };
                // TODO: If paths don't often change, should we optimize this
                // with delta encoding or something similar?
                insert_invocation_source_file
                    .execute((
                        invocation_id,
                        source_file_id,
                        source_path
                            .strip_prefix(&workspace_path)
                            .unwrap()
                            .display()
                            .to_string(),
                        source_mtime,
                    ))
                    .context("could not record source file invocation")?;
            }
        }

        // Execute the build.
        let exit_status = exec(&argv).await.context("could not execute build")?;

        // Record the build artifacts.
        let check_artifact = &mut tx
            .prepare("SELECT artifact_id FROM artifact WHERE b3sum = ?1")
            .context("could not prepare artifact check")?;
        let insert_artifact = &mut tx
            .prepare(
                "INSERT INTO artifact (b3sum) VALUES (?1) ON CONFLICT DO NOTHING RETURNING artifact_id",
            )
            .context("could not prepare artifact insert")?;
        let insert_invocation_artifact = &mut tx
            .prepare(
                "INSERT INTO invocation_artifact (invocation_id, artifact_id, path, mtime) VALUES (?1, ?2, ?3, ?4)",
            )
            .context("could not prepare artifact invocation insert")?;
        for entry in WalkDir::new(&workspace_cache.workspace_target_path) {
            let entry = entry.context("could not walk target directory")?;
            if entry.file_type().is_file() {
                let target_path = entry.path();
                let target_mtime: OffsetDateTime = entry
                    .metadata()
                    .context("could not get file metadata")?
                    .modified()
                    .context("could not get file mtime")?
                    .into();
                // TODO: Improve performance here? `blake3` provides both
                // streaming and parallel APIs.
                let target_bytes = fs::read(target_path)
                    .context(format!("could not read artifact {}", target_path.display()))?;
                let target_b3sum = blake3::hash(&target_bytes).to_hex().to_string();
                trace!(?target_path, ?target_mtime, ?target_b3sum, "read artifact");

                let target_file_id = match check_artifact
                    .query_row((&target_b3sum,), |row| row.get::<_, i64>(0))
                    .optional()
                    .context("could not check artifact")?
                {
                    Some(rid) => rid,
                    None => {
                        // For build artifacts that are new, save them to the
                        // CAS.
                        fs::write(workspace_cache.cas_path.join(&target_b3sum), &target_bytes)
                            .context("could not save artifact to CAS")?;

                        // Record the build artifact.
                        insert_artifact
                            .query_row((&target_b3sum,), |row| row.get::<_, i64>(0))
                            .context("could not insert artifact")?
                    }
                };
                // TODO: If paths don't often change, should we optimize this
                // with delta encoding or something similar?
                insert_invocation_artifact
                    .execute((
                        invocation_id,
                        target_file_id,
                        target_path
                            .strip_prefix(&workspace_cache.workspace_cache_path)
                            .unwrap()
                            .display()
                            .to_string(),
                        target_mtime,
                    ))
                    .context("could not record artifact invocation")?;
            }
        }

        exit_status
    };

    // Finalize database interactions.
    tx.commit().context("could not commit cache transaction")?;
    match workspace_cache.metadb.close() {
        Ok(_) => {}
        // TODO: Retry closing more times?
        Err((_, e)) => Err(e).context("could not close database")?,
    }

    Ok(exit_status)
}

#[instrument]
pub async fn exec(argv: &[String]) -> anyhow::Result<ExitStatus> {
    let mut cmd = std::process::Command::new("cargo");
    cmd.args(argv);
    Ok(cmd
        .spawn()
        .context("could not spawn cargo")?
        .wait()
        .context("could complete cargo execution")?)
}
