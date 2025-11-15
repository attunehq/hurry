use std::collections::HashMap;

use serde::Deserialize;

use crate::cargo::{RustcArguments, unit_graph::CargoCompileMode};

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
    pub kind: Option<String>,
    pub compile_mode: CargoCompileMode,
    pub deps: Vec<usize>,
    pub outputs: Vec<String>,
    // Note that this map is a link of built artifacts to hardlinks on the
    // filesystem (that are used to alias the built artifacts). This does NOT
    // enumerate libraries being linked in.
    pub links: HashMap<String, String>,
    pub program: String,
    pub args: RustcArguments,
    pub env: HashMap<String, String>,
    pub cwd: String,
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
