use std::{collections::HashMap, path::PathBuf, sync::Arc, time::SystemTime};

use clients::courier::v1 as courier;
use color_eyre::{
    Result,
    eyre::{self, OptionExt as _, bail},
};
use derive_more::Debug;
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use tap::Pipe as _;
use tracing::debug;

use crate::{
    cargo::{DepInfo, Fingerprint, QualifiedPath, SavedFile, UnitPlanInfo, Workspace, fingerprint},
    fs,
    path::{AbsFilePath, JoinWith as _, RelFilePath, TryJoinWith as _},
};

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct LibraryCrateUnitPlan {
    pub info: UnitPlanInfo,
    pub src_path: AbsFilePath,
    pub outputs: Vec<AbsFilePath>,
}

impl LibraryCrateUnitPlan {
    pub fn dep_info_file(&self) -> Result<RelFilePath> {
        self.info.deps_dir()?.try_join_file(format!(
            "{}-{}.d",
            self.info.crate_name, self.info.unit_hash
        ))
    }

    pub fn encoded_dep_info_file(&self) -> Result<RelFilePath> {
        self.info
            .fingerprint_dir()?
            .try_join_file(format!("dep-lib-{}", self.info.crate_name))
    }

    pub fn fingerprint_json_file(&self) -> Result<RelFilePath> {
        self.info
            .fingerprint_dir()?
            .try_join_file(format!("lib-{}.json", self.info.crate_name))
    }

    pub fn fingerprint_hash_file(&self) -> Result<RelFilePath> {
        self.info
            .fingerprint_dir()?
            .try_join_file(format!("lib-{}", self.info.crate_name))
    }

    pub async fn read(&self, ws: &Workspace) -> Result<LibraryFiles> {
        let profile_dir = ws.unit_profile_dir(&self.info);

        // There should only be 1-3 files here, it's a very small number.
        let output_files = {
            let mut output_files = Vec::new();
            for output_file_path in &self.outputs {
                let path =
                    QualifiedPath::parse(ws, &self.info.target_arch, output_file_path.as_ref())
                        .await?;
                let contents = fs::must_read_buffered(output_file_path).await?;
                let executable = fs::is_executable(output_file_path.as_std_path()).await;
                output_files.push(SavedFile {
                    path,
                    contents,
                    executable,
                });
            }
            output_files
        };

        let dep_info_file = DepInfo::from_file(
            ws,
            &self.info.target_arch,
            &profile_dir.join(&self.dep_info_file()?),
        )
        .await?;

        let encoded_dep_info_file =
            fs::must_read_buffered(&profile_dir.join(&self.encoded_dep_info_file()?)).await?;

        let fingerprint = {
            let fingerprint_json =
                fs::must_read_buffered_utf8(&profile_dir.join(&self.fingerprint_json_file()?))
                    .await?;
            let fingerprint: Fingerprint = serde_json::from_str(&fingerprint_json)?;

            let fingerprint_hash =
                fs::must_read_buffered_utf8(&profile_dir.join(&self.fingerprint_hash_file()?))
                    .await?;

            // Sanity check that the fingerprint hashes match.
            if fingerprint.fingerprint_hash() != fingerprint_hash {
                bail!("fingerprint hash mismatch");
            }

            fingerprint
        };

        Ok(LibraryFiles {
            output_files,
            dep_info_file,
            fingerprint,
            encoded_dep_info_file,
        })
    }

    /// Set the mtime for all output files of this unit. This function assumes
    /// these files are present on disk, and will return an error if they are
    /// not.
    pub async fn touch(&self, ws: &Workspace, mtime: SystemTime) -> Result<()> {
        let profile_dir = ws.unit_profile_dir(&self.info);

        tokio::try_join!(
            // Set output file mtimes.
            async {
                for path in &self.outputs {
                    fs::set_mtime(path, mtime).await?;
                }
                Ok(())
            },
            // Set dep info file mtime.
            async { fs::set_mtime(&profile_dir.join(&self.dep_info_file()?), mtime).await },
            // Set encoded dep info file mtime.
            async { fs::set_mtime(&profile_dir.join(&self.encoded_dep_info_file()?), mtime).await },
            // Set fingerprint file mtimes.
            async { fs::set_mtime(&profile_dir.join(&self.fingerprint_json_file()?), mtime).await },
            async { fs::set_mtime(&profile_dir.join(&self.fingerprint_hash_file()?), mtime).await },
        )?;

        Ok(())
    }
}

impl TryFrom<LibraryCrateUnitPlan> for courier::LibraryCrateUnitPlan {
    type Error = eyre::Report;

    fn try_from(value: LibraryCrateUnitPlan) -> Result<Self> {
        Self::builder()
            .info(value.info)
            .src_path(serde_json::to_string(&value.src_path)?)
            .outputs(
                value
                    .outputs
                    .into_iter()
                    .map(|p| Result::<_>::Ok(serde_json::to_string(&p)?.into()))
                    .try_collect::<_, Vec<_>, _>()?,
            )
            .build()
            .pipe(Ok)
    }
}

/// Libraries are usually associated with 7 files:
///
/// - 2 output files (an `.rmeta` and an `.rlib`)
/// - 1 rustc dep-info (`.d`) file in the `deps` folder
/// - 4 files in the fingerprint directory
///   - An `EncodedDepInfo` file
///   - A fingerprint hash
///   - A fingerprint JSON
///   - An invoked timestamp
///
/// Of these files, the fingerprint hash, fingerprint JSON, and invoked
/// timestamp are all reconstructed from fingerprint information during
/// restoration.
#[derive(Debug, Serialize, Deserialize)]
pub struct LibraryFiles {
    /// These files come from the build plan's `outputs` field.
    // TODO: Can we specify this even more narrowly (e.g. with an `rmeta` and
    // `rlib` field)? I know there are other possible output files (e.g. `.so`
    // for proc macros on Linux and `.dylib` for something on macOS), but I
    // don't know what the enumerated list is.
    pub output_files: Vec<SavedFile>,
    /// This file is always at a known path in
    /// `deps/{package_name}-{unit_hash}.d`.
    pub dep_info_file: DepInfo,
    /// This information is parsed from the initial fingerprint created after
    /// the build, and is used to dynamically reconstruct fingerprints on
    /// restoration.
    pub fingerprint: Fingerprint,
    /// This file is always at a known path in
    /// `.fingerprint/{package_name}-{unit_hash}/dep-lib-{crate_name}`. It can
    /// be safely relocatably copied because the `EncodedDepInfo` struct only
    /// ever contains relative file path information (note that deps always have
    /// a `DepInfoPathType`, which is either `PackageRootRelative` or
    /// `BuildRootRelative`)[^1].
    ///
    /// [^1]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/core/compiler/fingerprint/dep_info.rs#L112
    pub encoded_dep_info_file: Vec<u8>,
}

impl LibraryFiles {
    #[allow(unused, reason = "documents how to restore in-memory unit")]
    pub async fn restore(
        self,
        ws: &Workspace,
        dep_fingerprints: &mut HashMap<u64, Arc<Fingerprint>>,
        unit_plan: &LibraryCrateUnitPlan,
    ) -> Result<()> {
        let profile_dir = ws.unit_profile_dir(&unit_plan.info);

        // Restore output files.
        for saved_file in self.output_files {
            let path = saved_file
                .path
                .reconstruct(ws, &unit_plan.info)
                .try_into()?;
            fs::write(&path, saved_file.contents).await?;
            fs::set_executable(&path, saved_file.executable).await?;
        }

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

    pub async fn restore_fingerprint(
        ws: &Workspace,
        dep_fingerprints: &mut HashMap<u64, Arc<Fingerprint>>,
        mut fingerprint: Fingerprint,
        unit_plan: &LibraryCrateUnitPlan,
    ) -> Result<()> {
        let profile_dir = ws.unit_profile_dir(&unit_plan.info);
        let old_fingerprint_hash = fingerprint.hash_u64();

        // First, rewrite the `path` field.
        fingerprint.path = fingerprint::util_hash_u64(PathBuf::from(&unit_plan.src_path));
        debug!(path = ?PathBuf::from(&unit_plan.src_path), path_hash = ?fingerprint.path, "rewritten fingerprint");

        // Then, rewrite the `deps` field.
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
