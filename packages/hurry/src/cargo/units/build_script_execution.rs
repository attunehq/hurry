use std::{collections::HashMap, sync::Arc, time::SystemTime};

use clients::courier::v1 as courier;
use color_eyre::{
    Result,
    eyre::{self, OptionExt as _, bail},
};
use derive_more::Debug;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use tap::Pipe as _;
use tracing::debug;

use crate::{
    cargo::{BuildScriptOutput, Fingerprint, QualifiedPath, SavedFile, UnitPlanInfo, Workspace},
    fs, mk_rel_dir, mk_rel_file,
    path::{JoinWith as _, RelDirPath, RelFilePath, TryJoinWith as _},
};

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BuildScriptExecutionUnitPlan {
    // Note that we don't save src_path for build script execution because this
    // field is always set to `""` in the fingerprint for build script execution
    // units[^1].
    //
    // [^1]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/compiler/fingerprint/mod.rs#L1665
    pub info: UnitPlanInfo,

    /// The entrypoint module name of the compiled build script program after
    /// linkage (i.e. using the original build script name, which is what Cargo
    /// uses to name the execution unit files).
    pub build_script_program_name: String,
}

impl BuildScriptExecutionUnitPlan {
    pub fn fingerprint_json_file(&self) -> Result<RelFilePath> {
        self.info.fingerprint_dir()?.try_join_file(format!(
            "run-build-script-{}.json",
            self.build_script_program_name
        ))
    }

    pub fn fingerprint_hash_file(&self) -> Result<RelFilePath> {
        self.info.fingerprint_dir()?.try_join_file(format!(
            "run-build-script-{}",
            self.build_script_program_name
        ))
    }

    pub fn out_dir(&self) -> Result<RelDirPath> {
        Ok(self.info.build_dir()?.join(mk_rel_dir!("out")))
    }

    pub fn stdout_file(&self) -> Result<RelFilePath> {
        Ok(self.info.build_dir()?.join(mk_rel_file!("output")))
    }

    pub fn stderr_file(&self) -> Result<RelFilePath> {
        Ok(self.info.build_dir()?.join(mk_rel_file!("stderr")))
    }

    pub fn root_output_file(&self) -> Result<RelFilePath> {
        Ok(self.info.build_dir()?.join(mk_rel_file!("root-output")))
    }

    pub async fn read(&self, ws: &Workspace) -> Result<BuildScriptOutputFiles> {
        let profile_dir = ws.unit_profile_dir(&self.info);

        let stdout = BuildScriptOutput::from_file(
            ws,
            &self.info.target_arch,
            &profile_dir.join(&self.stdout_file()?),
        )
        .await?;
        let stderr = fs::must_read_buffered(&profile_dir.join(&self.stderr_file()?)).await?;
        let out_dir_files = {
            let files = fs::walk_files(&profile_dir.join(&self.out_dir()?))
                .try_collect::<Vec<_>>()
                .await?;
            let mut out_dir_files = Vec::new();
            for file in files {
                let path = QualifiedPath::parse(ws, &self.info.target_arch, file.as_ref()).await?;
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
                fs::must_read_buffered_utf8(&profile_dir.join(self.fingerprint_json_file()?))
                    .await?;
            let fingerprint: Fingerprint = serde_json::from_str(&fingerprint_json)?;

            let fingerprint_hash =
                fs::must_read_buffered_utf8(&profile_dir.join(self.fingerprint_hash_file()?))
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
        Ok(BuildScriptOutputFiles {
            out_dir_files,
            stdout,
            stderr,
            fingerprint,
        })
    }

    /// Set the mtime for all output files of this unit. This function assumes
    /// these files are present on disk, and will return an error if they are
    /// not.
    pub async fn touch(&self, ws: &Workspace, mtime: SystemTime) -> Result<()> {
        let profile_dir = ws.unit_profile_dir(&self.info);

        tokio::try_join!(
            // Touch the stdout file mtime.
            async { fs::set_mtime(&profile_dir.join(self.stdout_file()?), mtime).await },
            // Touch the stderr file mtime.
            async { fs::set_mtime(&profile_dir.join(self.stderr_file()?), mtime).await },
            // Touch every file in the OUT_DIR.
            async {
                let out_dir_files = fs::walk_files(&profile_dir.join(&self.out_dir()?))
                    .try_collect::<Vec<_>>()
                    .await?;
                for file in out_dir_files {
                    fs::set_mtime(&file, mtime).await?;
                }
                Ok(())
            },
            // Touch the root output file mtime.
            async { fs::set_mtime(&profile_dir.join(self.root_output_file()?), mtime).await },
            // Touch the fingerprint file mtimes.
            async { fs::set_mtime(&profile_dir.join(self.fingerprint_json_file()?), mtime).await },
            async { fs::set_mtime(&profile_dir.join(self.fingerprint_hash_file()?), mtime).await },
        )?;

        Ok(())
    }
}

impl TryFrom<BuildScriptExecutionUnitPlan> for courier::BuildScriptExecutionUnitPlan {
    type Error = eyre::Report;

    fn try_from(value: BuildScriptExecutionUnitPlan) -> Result<Self> {
        Self::builder()
            .info(value.info)
            .build_script_program_name(value.build_script_program_name)
            .build()
            .pipe(Ok)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildScriptOutputFiles {
    pub out_dir_files: Vec<SavedFile>,
    pub stdout: BuildScriptOutput,
    pub stderr: Vec<u8>,
    pub fingerprint: Fingerprint,
}

impl BuildScriptOutputFiles {
    #[allow(unused, reason = "documents how to restore in-memory unit")]
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
                .reconstruct(ws, &unit_plan.info)
                .try_into()?;
            fs::write(&path, saved_file.contents).await?;
            fs::set_executable(&path, saved_file.executable).await?;
        }

        // Reconstruct and restore build script STDOUT.
        fs::write(
            &profile_dir.join(&unit_plan.stdout_file()?),
            self.stdout.reconstruct(ws, &unit_plan.info),
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
        Self::restore_fingerprint(ws, dep_fingerprints, self.fingerprint, unit_plan).await?;

        Ok(())
    }

    pub async fn restore_fingerprint(
        ws: &Workspace,
        dep_fingerprints: &mut HashMap<u64, Arc<Fingerprint>>,
        mut fingerprint: Fingerprint,
        unit_plan: &BuildScriptExecutionUnitPlan,
    ) -> Result<()> {
        let profile_dir = ws.unit_profile_dir(&unit_plan.info);
        let old_fingerprint_hash = fingerprint.hash_u64();

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
        for dep in fingerprint.deps.iter_mut() {
            debug!(?dep, "rewriting fingerprint dep");
            let old_dep_fingerprint = dep.fingerprint.hash_u64();
            dep.fingerprint = dep_fingerprints
                .get(&old_dep_fingerprint)
                .ok_or_eyre("dependency fingerprint hash not found")?
                .clone();
        }
        debug!("rewrite fingerprint deps: done");

        // Clear and recalculate fingerprint hash.
        fingerprint.clear_memoized();
        let fingerprint_hash = fingerprint.fingerprint_hash();
        debug!(old = ?old_fingerprint_hash, new = ?fingerprint.hash_u64(), "rewritten fingerprint hash");

        // Finally, write the reconstructed fingerprint.
        fs::write(
            &profile_dir.join(&unit_plan.fingerprint_hash_file()?),
            fingerprint_hash,
        )
        .await?;
        fs::write(
            &profile_dir.join(&unit_plan.fingerprint_json_file()?),
            serde_json::to_vec(&fingerprint)?,
        )
        .await?;

        // Save unit fingerprint (for future dependents).
        dep_fingerprints.insert(old_fingerprint_hash, Arc::new(fingerprint));

        Ok(())
    }
}
