use std::{collections::HashMap, sync::Arc};

use color_eyre::{
    Result,
    eyre::{bail, eyre},
};
use derive_more::Debug;
use futures::TryStreamExt as _;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    cargo::{
        BuildScriptExecutionUnitPlan, BuildScriptOutput, Fingerprint, QualifiedPath, Workspace,
        cache::SavedFile,
    },
    fs,
    path::{AbsFilePath, JoinWith as _},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildScriptOutputFiles {
    out_dir_files: Vec<SavedFile>,
    stdout: BuildScriptOutput,
    stderr: Vec<u8>,
    fingerprint: Fingerprint,
}

impl BuildScriptOutputFiles {
    pub async fn read(ws: &Workspace, unit_plan: &BuildScriptExecutionUnitPlan) -> Result<Self> {
        let profile_dir = ws.unit_profile_dir(&unit_plan.info);

        let stdout = BuildScriptOutput::from_file(
            ws,
            &unit_plan.info.target_arch,
            &profile_dir.join(&unit_plan.stdout_file()?),
        )
        .await?;
        let stderr = fs::must_read_buffered(&profile_dir.join(&unit_plan.stderr_file()?)).await?;
        let out_dir_files = {
            let files = fs::walk_files(&profile_dir.join(&unit_plan.out_dir()?))
                .try_collect::<Vec<_>>()
                .await?;
            let mut out_dir_files = Vec::new();
            for file in files {
                let path =
                    QualifiedPath::parse(ws, &unit_plan.info.target_arch, file.as_ref()).await?;
                let executable = fs::is_executable(&file).await;
                let contents = fs::must_read_buffered(&file).await?;
                out_dir_files.push(SavedFile {
                    path,
                    executable,
                    contents,
                });
            }
            out_dir_files
        };

        let fingerprint = {
            let fingerprint_json =
                fs::must_read_buffered_utf8(&profile_dir.join(unit_plan.fingerprint_json_file()?))
                    .await?;
            let fingerprint: Fingerprint = serde_json::from_str(&fingerprint_json)?;

            let fingerprint_hash =
                fs::must_read_buffered_utf8(&profile_dir.join(unit_plan.fingerprint_hash_file()?))
                    .await?;

            // Sanity check that the fingerprint hashes match.
            if fingerprint.fingerprint_hash() != fingerprint_hash {
                bail!("fingerprint hash mismatch");
            }

            fingerprint
        };

        // Note that we don't save
        // `{profile_dir}/.fingerprint/{package_name}-{unit_hash}/root-output`
        // because it is fully reconstructible from the workspace and the unit
        // plan.
        Ok(Self {
            out_dir_files,
            stdout,
            stderr,
            fingerprint,
        })
    }

    pub async fn restore(
        self,
        ws: &Workspace,
        dep_fingerprints: &mut HashMap<u64, Arc<Fingerprint>>,
        unit_plan: &BuildScriptExecutionUnitPlan,
    ) -> Result<()> {
        let profile_dir = ws.unit_profile_dir(&unit_plan.info);

        // Restore OUT_DIR files.
        for saved_file in self.out_dir_files {
            let path = saved_file
                .path
                .reconstruct(ws, &unit_plan.info.target_arch)
                .map(AbsFilePath::try_from)??;
            fs::write(&path, saved_file.contents).await?;
            fs::set_executable(&path, saved_file.executable).await?;
        }

        // Reconstruct and restore build script STDOUT.
        fs::write(
            &profile_dir.join(&unit_plan.stdout_file()?),
            self.stdout.reconstruct(ws, &unit_plan.info.target_arch)?,
        )
        .await?;

        // Restore build script STDERR.
        fs::write(&profile_dir.join(&unit_plan.stderr_file()?), self.stderr).await?;

        // Generate `root-output` file.
        fs::write(
            &profile_dir.join(&unit_plan.root_output_file()?),
            unit_plan.out_dir()?.as_os_str().as_encoded_bytes(),
        )
        .await?;

        // Reconstruct and restore fingerprint.
        let mut saved_fingerprint = self.fingerprint;
        let old_fingerprint_hash = saved_fingerprint.hash_u64();

        // Rewrite the `deps` field. Note that we never need to rewrite
        // the `path` field for build script execution units, since it's
        // always unset[^1].
        //
        // We don't actually have enough information to synthesize our
        // own DepFingerprints (in particular, it would be very annoying
        // to derive `only_requires_rmeta` independently). But the old
        // fingerprint hashes are unique, and we know our old
        // fingerprint hash! So we save a map of the old fingerprint
        // hashes to the replacement fingerprint hashes, and use that to
        // look up the correct replacement fingerprint hash in future
        // DepFingerprints, leaving all other fields untouched.
        //
        // This works because we know the units are in dependency order,
        // so previous replacement fingerprint hashes will always have
        // already been calculated when we need them.
        //
        // [^1]: https://github.com/attunehq/cargo/blob/21f1bfe23aa3fafd6205b8e3368a499466336bb9/src/cargo/core/compiler/fingerprint/mod.rs#L1696
        debug!("rewrite fingerprint deps: start");
        for dep in saved_fingerprint.deps.iter_mut() {
            debug!(?dep, "rewriting fingerprint dep");
            let old_dep_fingerprint = dep.fingerprint.hash_u64();
            dep.fingerprint = dep_fingerprints
                .get(&old_dep_fingerprint)
                .ok_or_else(|| {
                    eyre!("dependency fingerprint hash not found").wrap_err(format!(
                        "rewriting fingerprint {} for unit {}",
                        old_dep_fingerprint, unit_plan.info.unit_hash
                    ))
                })?
                .clone();
        }
        debug!("rewrite fingerprint deps: done");

        // Clear and recalculate fingerprint hash.
        saved_fingerprint.clear_memoized();
        let fingerprint_hash = saved_fingerprint.fingerprint_hash();
        debug!(old = ?old_fingerprint_hash, new = ?saved_fingerprint.hash_u64(), "rewritten fingerprint hash");

        // Finally, write the reconstructed fingerprint.
        fs::write(
            &profile_dir.join(&unit_plan.fingerprint_hash_file()?),
            fingerprint_hash,
        )
        .await?;
        fs::write(
            &profile_dir.join(&unit_plan.fingerprint_json_file()?),
            serde_json::to_vec(&saved_fingerprint)?,
        )
        .await?;

        // Save unit fingerprint (for future dependents).
        dep_fingerprints.insert(old_fingerprint_hash, Arc::new(saved_fingerprint));

        Ok(())
    }
}
