use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use clients::courier::v1 as courier;
use color_eyre::{
    Result,
    eyre::{self, OptionExt as _},
};
use derive_more::Debug;
use serde::{Deserialize, Serialize};
use tap::Pipe as _;
use tracing::instrument;

use crate::{
    cargo::{DepInfo, Fingerprint, UnitPlanInfo, Workspace},
    fs,
    path::{AbsFilePath, JoinWith as _, RelFilePath, TryJoinWith as _},
};

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BuildScriptCompilationUnitPlan {
    pub info: UnitPlanInfo,

    /// The path to the build script's main entrypoint source file. This is
    /// usually `build.rs` within the package's source code, but can vary if
    /// the package author sets `package.build` in the package's
    /// `Cargo.toml`, which changes the build script's name[^1].
    ///
    /// This is parsed from the rustc invocation arguments in the unit's
    /// build plan invocation.
    ///
    /// This is used to rewrite the build script compilation's fingerprint
    /// on restore.
    ///
    /// [^1]: https://doc.rust-lang.org/cargo/reference/manifest.html#the-build-field
    pub src_path: AbsFilePath,
}

impl BuildScriptCompilationUnitPlan {
    fn entrypoint_module_name(&self) -> Result<String> {
        let src_path_filename = self
            .src_path
            .file_name_str_lossy()
            .ok_or_eyre("build script source path has no name")?;
        Ok(src_path_filename
            .strip_suffix(".rs")
            .ok_or_eyre("build script source path has no `.rs` extension")?
            .to_string())
    }

    /// Build scripts always compile to a single program and a renamed hard
    /// link to the same program.
    ///
    /// This is parsed from the `outputs` and `links` paths in the unit's
    /// build plan invocation.
    ///
    /// These file paths must have their mtimes modified to be later than
    /// the fingerprint's invoked timestamp for the unit to be marked fresh.
    pub fn program_file(&self) -> Result<RelFilePath> {
        self.info.build_dir()?.try_join_file(format!(
            "build_script_{}-{}",
            self.entrypoint_module_name()?.replace("-", "_"),
            self.info.unit_hash
        ))
    }

    pub fn linked_program_file(&self) -> Result<RelFilePath> {
        self.info
            .build_dir()?
            .try_join_file(format!("build-script-{}", self.entrypoint_module_name()?))
    }

    pub fn dep_info_file(&self) -> Result<RelFilePath> {
        self.info.build_dir()?.try_join_file(format!(
            "build_script_{}-{}.d",
            self.entrypoint_module_name()?.replace("-", "_"),
            self.info.unit_hash
        ))
    }

    pub fn encoded_dep_info_file(&self) -> Result<RelFilePath> {
        self.info.fingerprint_dir()?.try_join_file(format!(
            "dep-build-script-build-script-{}",
            self.entrypoint_module_name()?
        ))
    }

    pub fn fingerprint_json_file(&self) -> Result<RelFilePath> {
        self.info.fingerprint_dir()?.try_join_file(format!(
            "build-script-build-script-{}.json",
            self.entrypoint_module_name()?
        ))
    }

    pub fn fingerprint_hash_file(&self) -> Result<RelFilePath> {
        self.info.fingerprint_dir()?.try_join_file(format!(
            "build-script-build-script-{}",
            self.entrypoint_module_name()?
        ))
    }

    pub async fn read(&self, ws: &Workspace) -> Result<BuildScriptCompiledFiles> {
        let profile_dir = ws.unit_profile_dir(&self.info);

        let compiled_program =
            fs::must_read_buffered(&profile_dir.join(self.program_file()?)).await?;

        let dep_info_file = DepInfo::from_file(
            ws,
            &self.info.target_arch,
            &profile_dir.join(&self.dep_info_file()?),
        )
        .await?;

        let encoded_dep_info_file =
            fs::must_read_buffered(&profile_dir.join(&self.encoded_dep_info_file()?)).await?;

        let fingerprint = self.read_fingerprint(ws).await?;

        Ok(BuildScriptCompiledFiles {
            compiled_program,
            dep_info_file,
            fingerprint,
            encoded_dep_info_file,
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
            // Set compiled program and hard link mtime.
            async { fs::set_mtime(&profile_dir.join(self.program_file()?), mtime).await },
            async { fs::set_mtime(&profile_dir.join(self.linked_program_file()?), mtime).await },
            // Set dep info file mtime.
            async { fs::set_mtime(&profile_dir.join(self.dep_info_file()?), mtime).await },
            // Set encoded dep info file mtime.
            async { fs::set_mtime(&profile_dir.join(self.encoded_dep_info_file()?), mtime).await },
            // Set fingerprint file mtimes.
            async { fs::set_mtime(&profile_dir.join(self.fingerprint_json_file()?), mtime).await },
            async { fs::set_mtime(&profile_dir.join(self.fingerprint_hash_file()?), mtime).await },
        )?;

        Ok(())
    }
}

impl TryFrom<BuildScriptCompilationUnitPlan> for courier::BuildScriptCompilationUnitPlan {
    type Error = eyre::Report;

    fn try_from(value: BuildScriptCompilationUnitPlan) -> Result<Self> {
        Self::builder()
            .info(value.info)
            .src_path(serde_json::to_string(&value.src_path)?)
            .build()
            .pipe(Ok)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildScriptCompiledFiles {
    /// This fingerprint is stored in `.fingerprint`, and is used to derive the
    /// timestamp, fingerprint hash file, and fingerprint JSON file.
    pub fingerprint: Fingerprint,
    /// This field contains the contents of the compiled build script program at
    /// `build_script_{build_script_entrypoint}-{build_script_compilation_unit_hash}`
    /// and hard linked at `build-script-{build_script_entrypoint}`.
    ///
    /// We need both of these files: the hard link is the file that's actually
    /// executed in the build plan, but the full path with the unit hash is the
    /// file that's tracked by the fingerprint.
    pub compiled_program: Vec<u8>,
    /// This is the path to the rustc dep-info file in the build directory.
    pub dep_info_file: DepInfo,
    /// This `EncodedDepInfo` (i.e. Cargo dep-info) file is stored in
    /// `.fingerprint`, and is directly saved and restored.
    pub encoded_dep_info_file: Vec<u8>,
}

impl BuildScriptCompiledFiles {
    #[allow(unused, reason = "documents how to restore in-memory unit")]
    pub async fn restore(
        self,
        ws: &Workspace,
        dep_fingerprints: &mut HashMap<u64, Fingerprint>,
        unit_plan: &BuildScriptCompilationUnitPlan,
    ) -> Result<()> {
        let profile_dir = ws.unit_profile_dir(&unit_plan.info);

        // Restore compiled build script program.
        let program_file = profile_dir.join(unit_plan.program_file()?);
        fs::write(&program_file, self.compiled_program).await?;
        fs::set_executable(&program_file, true).await?;
        fs::hard_link(
            &program_file,
            &profile_dir.join(unit_plan.linked_program_file()?),
        )
        .await?;

        // Restore encoded Cargo dep-info file.
        fs::write(
            &profile_dir.join(&unit_plan.encoded_dep_info_file()?),
            self.encoded_dep_info_file,
        )
        .await?;

        // Reconstruct and restore rustc dep-info file.
        fs::write(
            &profile_dir.join(&unit_plan.dep_info_file()?),
            self.dep_info_file.reconstruct(ws, &unit_plan.info),
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
        unit_plan: &BuildScriptCompilationUnitPlan,
    ) -> Result<()> {
        // Rewrite the fingerprint.
        let rewritten =
            fingerprint.rewrite(Some(PathBuf::from(&unit_plan.src_path)), dep_fingerprints)?;
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
