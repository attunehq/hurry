use std::{
    collections::HashSet,
    fs::{self, File},
    process::ExitStatus,
};

use anyhow::Context;
use rusqlite::OptionalExtension;
use time::OffsetDateTime;
use tracing::{debug, instrument, trace};
use walkdir::WalkDir;

mod cache;
mod workspace;

#[instrument(level = "debug")]
pub async fn build(argv: &[String]) -> anyhow::Result<ExitStatus> {
    // Load the current workspace.
    let workspace = workspace::Workspace::open()?;

    // Change the working directory to the workspace root. This makes a lot of
    // relative file path calculations nicer.
    std::env::set_current_dir(&workspace.metadata.workspace_root)
        .context("could not change working directory to workspace root")?;

    // Initialize the workspace cache.
    //
    // TODO: All of these failures should be non-fatal and should not block us
    // from shelling out to `cargo build`.
    let mut workspace_cache = cache::WorkspaceCache::new(&workspace.metadata.workspace_root)
        .context("could not initialize workspace cache")?;
    let workspace_path = &workspace.metadata.workspace_root;

    // Record this invocation.
    let tx = workspace_cache
        .metadb
        .transaction()
        .context("could not start cache transaction")?;
    let invocation_id = tx
        .query_row(
            "INSERT INTO invocation (argv, start_time) VALUES (?1, ?2) RETURNING invocation_id",
            (argv.join(" "), OffsetDateTime::now_utc()),
            |row| row.get::<_, i64>(0),
        )
        .context("could not record hurry invocation in cache")?;
    debug!(?invocation_id, "recorded invocation");

    // Record the source files used in this invocation.
    //
    // TODO: Would parallelization improve performance here?
    let cached_invocation_id_candidates = {
        let check_source_file = &mut tx
            .prepare("SELECT source_file_id FROM source_file WHERE b3sum = ?1")
            .context("could not prepare source file seen check")?;
        let source_file_invocations = &mut tx
            .prepare(r#"
                SELECT
                    invocation.invocation_id
                FROM source_file
                JOIN invocation_source_file ON source_file.source_file_id = invocation_source_file.source_file_id
                JOIN invocation ON invocation.invocation_id = invocation_source_file.invocation_id
                WHERE
                    invocation_source_file.path = ?1
                    AND source_file.b3sum = ?2
            "#)
            .context("could not prepare source file candidate check")?;
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

        // Record the source files used in this invocation.
        let mut cached_invocation_id_candidates = HashSet::new();
        let mut candidates_populated = false;

        for source_path in workspace.source_files() {
            trace!(?source_path, "processing source file");
            let source_path_relative = source_path
                .strip_prefix(&workspace_path)
                .unwrap()
                .to_string();
            let source_mtime: OffsetDateTime = fs::metadata(&source_path)
                .context("could not get file metadata")?
                .modified()
                .context("could not get file mtime")?
                .into();
            // TODO: Improve performance here? `blake3` provides both
            // streaming and parallel APIs.
            let source_b3sum_hash = {
                let source_bytes = fs::read(&source_path).context("could not read source file")?;
                blake3::hash(&source_bytes)
            };
            let source_b3sum = source_b3sum_hash.as_bytes().to_owned();
            trace!(
                ?source_path_relative,
                ?source_mtime,
                source_b3sum = ?source_b3sum_hash.to_hex().to_string(),
                "read source file",
            );

            let source_file_id = match check_source_file
                .query_row((&source_b3sum,), |row| row.get::<_, i64>(0))
                .optional()
                .context("could not check source file seen")?
            {
                Some(rid) => {
                    // If this source file has been seen before, check
                    // whether any of its cached invocations are compatible
                    // with the cached invocations of other source files.
                    let file_candidate_invocation_ids = source_file_invocations
                        .query_map((&source_path_relative, &source_b3sum), |row| {
                            row.get::<_, i64>(0)
                        })
                        .context("could not load source file invocations")?
                        .collect::<Result<Vec<i64>, _>>()
                        .context("could not read source file invocations")?
                        .into_iter()
                        .collect::<HashSet<_>>();
                    trace!(
                        ?file_candidate_invocation_ids,
                        "cached invocations containing this source file",
                    );
                    if !candidates_populated {
                        cached_invocation_id_candidates = file_candidate_invocation_ids;
                        candidates_populated = true;
                    } else if !cached_invocation_id_candidates.is_empty() {
                        cached_invocation_id_candidates = cached_invocation_id_candidates
                            .intersection(&file_candidate_invocation_ids)
                            .copied()
                            .collect();
                    }
                    trace!(
                        ?cached_invocation_id_candidates,
                        "remaining cached invocations after checking source file",
                    );

                    rid
                }
                None => {
                    // If this source file has never been seen before,
                    // record it into the database. Then clear the set of
                    // cached invocations, because there's no way we have a
                    // cached invocation.
                    candidates_populated = true;
                    cached_invocation_id_candidates.clear();

                    insert_source_file
                        .query_row((&source_b3sum,), |row| row.get::<_, i64>(0))
                        .context("could not insert source file")?
                }
            };
            // TODO: If paths don't often change, should we optimize this
            // with delta encoding or something similar?
            insert_invocation_source_file
                .execute((
                    &invocation_id,
                    &source_file_id,
                    &source_path_relative,
                    &source_mtime,
                ))
                .context("could not record source file invocation")?;
        }

        cached_invocation_id_candidates
    };

    // If we can, swap in a cached invocation.
    if !cached_invocation_id_candidates.is_empty() {
        // We swap in the earliest cached invocation, because that one is
        // guaranteed to be correct.
        //
        // Why are later cached invocations possibly incorrect? This happens
        // because we record the SOURCE FILES before we do the cache swap,
        // but we only record the BUILD ARTIFACTS after we have swapped and
        // done the build.
        //
        // So if you were to touch a source file and then rebuild, we would:
        //
        //   1. Record the TOUCHED mtime (which is wrong and does not match
        //      the
        //   2. Restore the ORIGINAL source files and build cache.
        //   3. Run the build.
        //   4. Record the build cache (which is the same as the original
        //      because of the cache hit).
        //
        // Notice that this causes a mismatch! We now have saved source
        // files on the _touched_ mtime with built artifacts keyed for the
        // _original_ mtime, so this follow-up cache is essentially tainted.
        //
        // Swapping in the earliest invocation is a workaround for this
        // problem.
        //
        // TODO: To fix it in the long term, we can record some more
        // metadata around whether we did a switch and around what the
        // "canonical" build cache for a source file set is, so we don't
        // need to worry about this anymore.
        let earliest_cached_invocation_id =
            cached_invocation_id_candidates.into_iter().min().unwrap();
        debug!(
            ?earliest_cached_invocation_id,
            "swapping in cached invocation",
        );

        // Modify source file mtimes.
        let invocation_source_files = &mut tx
            .prepare("SELECT path, mtime FROM invocation_source_file WHERE invocation_id = ?1")
            .context("could not prepare source file mtimes query")?;
        for source_file in invocation_source_files
            .query_map((&earliest_cached_invocation_id,), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, OffsetDateTime>(1)?))
            })
            .context("could not load source file mtimes")?
        {
            let (path, mtime) = source_file.context("could not load cached source file")?;
            let path = workspace_path.join(&path);
            debug!(?path, ?mtime, "setting source file mtime");
            let file = File::open(&path).context("could not open source file")?;
            file.set_modified(mtime.into())
                .context("could not restore source file mtime")?;
        }

        // Swap in built artifact cache.
        let invocation_artifacts = &mut tx
            .prepare(
                r#"
                    SELECT
                        invocation_artifact.path,
                        invocation_artifact.mtime,
                        LOWER(HEX(artifact.b3sum)) AS b3sum
                    FROM invocation_artifact
                    JOIN artifact ON artifact.artifact_id = invocation_artifact.artifact_id
                    WHERE invocation_artifact.invocation_id = ?1"#,
            )
            .context("could not prepare artifact metadata query")?;
        for artifact in invocation_artifacts
            .query_map((&earliest_cached_invocation_id,), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, OffsetDateTime>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .context("could not load cached artifact metadata")?
        {
            // TODO: Is there optimization here to avoid copying files that
            // we know don't need to change? We could remember hashes from
            // the first pass, or re-hash them on this pass.
            let (path, mtime, b3sum_hex) =
                artifact.context("could not load cached artifact metadata")?;
            let path = workspace_cache.workspace_cache_path.join(&path);
            trace!(?path, ?mtime, b3sum = ?b3sum_hex, "restoring cached artifact");

            if !fs::exists(&path).context("could not check restored artifact path")? {
                fs::create_dir_all(&path.parent().unwrap())
                    .context("could not create path to restored artifact")?;
            }
            let artifact_cas_path = workspace_cache.cas_path.join(&b3sum_hex);
            // TODO: Restore executable bit.
            fs::copy(&artifact_cas_path, &path).context(format!(
                "could not restore cached artifact from {:?} to {:?}",
                artifact_cas_path.display(),
                path.display()
            ))?;
            let file = File::open(&path).context("could not open restored artifact")?;
            file.set_modified(mtime.into())
                .context("could not restore artifact mtime")?;
        }
    }

    // Execute the build.
    let exit_status = exec(&argv).await.context("could not execute build")?;

    // If the build wasn't successful, abort.
    //
    // TODO: We should add handling around the case where we swapped in a cache
    // _and_ the build failed. This should never happen, but if it does happen,
    // it indicates that the cache has been corrupted and should be cleared.
    if !exit_status.success() {
        tx.rollback()
            .context("could not rollback cache transaction")?;
        return Ok(exit_status);
    }

    // Record the build artifacts.
    {
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
                let target_b3sum_hash = blake3::hash(&target_bytes);
                let target_b3sum = target_b3sum_hash.as_bytes().to_owned();
                let target_b3sum_hex = target_b3sum_hash.to_hex().to_string();
                trace!(
                    ?target_path,
                    ?target_mtime,
                    target_b3sum = ?target_b3sum_hex,
                    "read artifact"
                );

                let target_file_id = match check_artifact
                    .query_row((&target_b3sum,), |row| row.get::<_, i64>(0))
                    .optional()
                    .context("could not check artifact")?
                {
                    Some(rid) => rid,
                    None => {
                        // For build artifacts that are new, save them to the
                        // CAS.
                        fs::write(
                            workspace_cache.cas_path.join(&target_b3sum_hex),
                            &target_bytes,
                        )
                        .context("could not save artifact to CAS")?;

                        // Record the build artifact.
                        insert_artifact
                            .query_row((&target_b3sum,), |row| row.get::<_, i64>(0))
                            .context("could not insert artifact")?
                    }
                };
                // TODO: If paths don't often change, should we optimize this
                // with delta encoding or something similar?
                //
                // TODO: Record executable bit.
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
    }

    // Finalize database interactions.
    tx.execute(
        "UPDATE invocation SET end_time = ?1 WHERE invocation_id = ?2",
        (OffsetDateTime::now_utc(), invocation_id),
    )
    .context("could not set invocation end time")?;
    tx.commit().context("could not commit cache transaction")?;
    match workspace_cache.metadb.close() {
        Ok(_) => {}
        // TODO: Retry closing more times?
        Err((_, e)) => Err(e).context("could not close database")?,
    }

    Ok(exit_status)
}

#[instrument(level = "debug")]
pub async fn exec(argv: &[String]) -> anyhow::Result<ExitStatus> {
    let mut cmd = std::process::Command::new("cargo");
    cmd.args(argv);
    Ok(cmd
        .spawn()
        .context("could not spawn cargo")?
        .wait()
        .context("could complete cargo execution")?)
}
