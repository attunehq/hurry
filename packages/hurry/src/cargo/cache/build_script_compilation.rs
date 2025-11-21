use std::{collections::HashMap, path::PathBuf, sync::Arc};

use color_eyre::{
    Result,
    eyre::{OptionExt as _, bail},
};
use derive_more::Debug;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    cargo::{BuildScriptCompilationUnitPlan, DepInfo, Fingerprint, Workspace, fingerprint},
    fs,
    path::JoinWith as _,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildScriptCompiledFiles {
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
    /// This fingerprint is stored in `.fingerprint`, and is used to derive the
    /// timestamp, fingerprint hash file, and fingerprint JSON file.
    pub fingerprint: Fingerprint,
    /// This `EncodedDepInfo` (i.e. Cargo dep-info) file is stored in
    /// `.fingerprint`, and is directly saved and restored.
    pub encoded_dep_info_file: Vec<u8>,
}

impl BuildScriptCompiledFiles {
    pub async fn read(ws: &Workspace, unit: &BuildScriptCompilationUnitPlan) -> Result<Self> {
        let profile_dir = ws.unit_profile_dir(&unit.info);

        let compiled_program =
            fs::must_read_buffered(&profile_dir.join(unit.program_file()?)).await?;

        let dep_info_file = DepInfo::from_file(
            ws,
            &unit.info.target_arch,
            &profile_dir.join(&unit.dep_info_file()?),
        )
        .await?;

        let encoded_dep_info_file =
            fs::must_read_buffered(&profile_dir.join(&unit.encoded_dep_info_file()?)).await?;

        let fingerprint = {
            let fingerprint_json =
                fs::must_read_buffered_utf8(&profile_dir.join(&unit.fingerprint_json_file()?))
                    .await?;
            let fingerprint: Fingerprint = serde_json::from_str(&fingerprint_json)?;

            let fingerprint_hash =
                fs::must_read_buffered_utf8(&profile_dir.join(&unit.fingerprint_hash_file()?))
                    .await?;

            // Sanity check that the fingerprint hashes match.
            if fingerprint.fingerprint_hash() != fingerprint_hash {
                bail!("fingerprint hash mismatch");
            }

            fingerprint
        };

        Ok(Self {
            compiled_program,
            dep_info_file,
            fingerprint,
            encoded_dep_info_file,
        })
    }

    #[allow(unused, reason = "documents how to restore in-memory unit")]
    pub async fn restore(
        self,
        ws: &Workspace,
        dep_fingerprints: &mut HashMap<u64, Arc<Fingerprint>>,
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
            self.dep_info_file
                .reconstruct(ws, &unit_plan.info.target_arch)?,
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
        unit_plan: &BuildScriptCompilationUnitPlan,
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
