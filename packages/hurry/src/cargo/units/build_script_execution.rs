use std::{collections::HashMap, time::SystemTime};

use clients::courier::v1 as courier;
use color_eyre::{Result, eyre};
use derive_more::Debug;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use tap::Pipe as _;
use tracing::instrument;

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
                let path = QualifiedPath::parse_abs(ws, &self.info.target_arch, file.as_ref());
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

        let fingerprint = self.read_fingerprint(ws).await?;

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

    pub async fn read_fingerprint(&self, ws: &Workspace) -> Result<Fingerprint> {
        let profile_dir = ws.unit_profile_dir(&self.info);
        Fingerprint::read(
            profile_dir.join(&self.fingerprint_json_file()?),
            profile_dir.join(&self.fingerprint_hash_file()?),
        )
        .await
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
    pub fingerprint: Fingerprint,
    pub out_dir_files: Vec<SavedFile>,
    pub stdout: BuildScriptOutput,
    pub stderr: Vec<u8>,
}

impl BuildScriptOutputFiles {
    #[allow(unused, reason = "documents how to restore in-memory unit")]
    pub async fn restore(
        self,
        ws: &Workspace,
        dep_fingerprints: &mut HashMap<u64, Fingerprint>,
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

    #[instrument(skip(ws, dep_fingerprints, fingerprint))]
    pub async fn restore_fingerprint(
        ws: &Workspace,
        dep_fingerprints: &mut HashMap<u64, Fingerprint>,
        fingerprint: Fingerprint,
        unit_plan: &BuildScriptExecutionUnitPlan,
    ) -> Result<()> {
        // Rewrite the fingerprint.
        let rewritten = fingerprint.rewrite(None, dep_fingerprints)?;
        let fingerprint_hash = rewritten.fingerprint_hash();

        // Write the reconstructed fingerprint.
        let profile_dir = ws.unit_profile_dir(&unit_plan.info);
        fs::write(
            &profile_dir.join(&unit_plan.fingerprint_hash_file()?),
            fingerprint_hash,
        )
        .await?;
        fs::write(
            &profile_dir.join(&unit_plan.fingerprint_json_file()?),
            serde_json::to_vec(&rewritten)?,
        )
        .await?;

        Ok(())
    }
}
