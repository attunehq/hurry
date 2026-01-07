use std::collections::HashMap;

use cargo_metadata::TargetKind;
use color_eyre::{Result, eyre, eyre::OptionExt as _};
use serde::Deserialize;

use crate::{
    cargo::{CargoCompileMode, RustcTarget, UnitHash},
    path::{AbsDirPath, AbsFilePath},
};

#[derive(Clone, Eq, PartialEq, Debug, Deserialize)]
pub struct BuildPlan {
    pub invocations: Vec<BuildPlanInvocation>,
    pub inputs: Vec<String>,
}

// Note that these fields are all undocumented. To see their definition, see
// https://github.com/rust-lang/cargo/blob/0436f86288a4d9bce1c712c4eea5b05eb82682b9/src/cargo/core/compiler/build_plan.rs#L21-L34
#[derive(Clone, Eq, PartialEq, Debug, Deserialize)]
pub struct BuildPlanInvocation {
    pub package_name: String,
    pub package_version: String,
    pub target_kind: Vec<cargo_metadata::TargetKind>,
    #[serde(rename = "kind")]
    pub target_arch: RustcTarget,
    pub compile_mode: CargoCompileMode,
    pub deps: Vec<usize>,
    pub outputs: Vec<String>,
    // Note that this map is a link of built artifacts to hardlinks on the
    // filesystem (that are used to alias the built artifacts). This does NOT
    // enumerate libraries being linked in.
    pub links: HashMap<String, String>,
    pub program: String,
    // These are usually `rustc` arguments, but not always! For example, build
    // script execution units' arguments are not technically `rustc` arguments
    // (although in practice they appear to always be empty).
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: String,
}

impl BuildPlanInvocation {
    /// Returns the unit hash for this build plan invocation.
    ///
    /// Returns `None` for invocation types we don't cache (binaries,
    /// unsupported target kinds). Returns `Some(hash)` for library crates,
    /// build script compilations, and build script executions.
    ///
    /// This is used to build an index→hash mapping before creating units, so
    /// that dep indices can be resolved to UnitHash values.
    pub fn unit_hash(&self) -> Result<Option<UnitHash>> {
        if self.target_kind == [TargetKind::CustomBuild] {
            match self.compile_mode {
                CargoCompileMode::Build => {
                    // Parse unit hash from output filename like
                    // `build_script_{name}-{hash}`
                    let output = self
                        .outputs
                        .iter()
                        .find(|o| !o.ends_with(".dwp") && !o.ends_with(".dSYM"))
                        .ok_or_eyre("build script compilation has no outputs")?;
                    let path = AbsFilePath::try_from(output.as_str())?;
                    let filename = path
                        .file_name_str_lossy()
                        .ok_or_eyre("program file has no name")?;
                    let hash = filename
                        .rsplit_once('-')
                        .ok_or_eyre("program file has no unit hash")?
                        .1;
                    Ok(Some(UnitHash::from(hash.to_string())))
                }
                CargoCompileMode::RunCustomBuild => {
                    // Parse unit hash from OUT_DIR path like `.../build/{pkg}-{hash}/out`
                    let out_dir = self
                        .env
                        .get("OUT_DIR")
                        .ok_or_eyre("build script execution should set OUT_DIR")?;
                    let out_dir = AbsDirPath::try_from(out_dir.as_str())?;
                    let unit_dir = out_dir.parent().ok_or_eyre("OUT_DIR should have parent")?;
                    let dir_name = unit_dir
                        .file_name_str_lossy()
                        .ok_or_eyre("build script execution directory should have name")?;
                    let hash = dir_name
                        .rsplit_once('-')
                        .ok_or_eyre("build script execution directory should have unit hash")?
                        .1;
                    Ok(Some(UnitHash::from(hash.to_string())))
                }
                _ => Ok(None),
            }
        } else if self.target_kind == [TargetKind::Bin] {
            // Binaries are not cached
            Ok(None)
        } else if self.target_kind.contains(&TargetKind::Lib)
            || self.target_kind.contains(&TargetKind::RLib)
            || self.target_kind.contains(&TargetKind::CDyLib)
            || self.target_kind.contains(&TargetKind::ProcMacro)
        {
            // Parse unit hash from output filename like `lib{name}-{hash}.rlib`
            let output = self
                .outputs
                .iter()
                .find(|o| !o.ends_with(".dwp") && !o.ends_with(".dSYM"))
                .ok_or_eyre("library crate has no outputs")?;
            let path = AbsFilePath::try_from(output.as_str())?;
            let filename = path
                .file_name()
                .ok_or_eyre("no filename")?
                .to_string_lossy();
            let filename = filename.split_once('.').ok_or_eyre("no extension")?.0;
            let hash = filename
                .rsplit_once('-')
                .ok_or_else(|| {
                    eyre::eyre!(
                        "no unit hash suffix in filename: {filename} (outputs: {:?})",
                        self.outputs
                    )
                })?
                .1;
            Ok(Some(UnitHash::from(hash.to_string())))
        } else {
            // Unknown target kind - don't cache
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use color_eyre::{Result, Section as _, SectionExt as _, eyre::Context as _};

    use super::*;

    #[test]
    fn parse_build_plan_smoke() -> Result<()> {
        let _ = color_eyre::install();

        let output = std::process::Command::new("cargo")
            .args(["build", "--build-plan", "-Z", "unstable-options"])
            .env("RUSTC_BOOTSTRAP", "1")
            .output()
            .expect("execute cargo build-plan");

        assert!(
            output.status.success(),
            "cargo build-plan failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let build_plan = serde_json::from_slice::<BuildPlan>(&output.stdout)
            .with_section(|| {
                String::from_utf8_lossy(&output.stdout)
                    .to_string()
                    .header("Build Plan:")
            })
            .context("parse build plan JSON")?;

        assert!(
            !build_plan.invocations.is_empty(),
            "build plan should have invocations"
        );

        Ok(())
    }
}
