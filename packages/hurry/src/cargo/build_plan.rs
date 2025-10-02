use std::collections::HashMap;

use serde::Deserialize;

use crate::cargo::unit_graph::CargoCompileMode;

#[derive(Debug, Deserialize)]
pub struct BuildPlan {
    pub invocations: Vec<BuildPlanInvocation>,
    pub inputs: Vec<String>,
}

// Note that these fields are all undocumented. To see their definition, see
// https://github.com/rust-lang/cargo/blob/0436f86288a4d9bce1c712c4eea5b05eb82682b9/src/cargo/core/compiler/build_plan.rs#L21-L34
#[derive(Debug, Deserialize)]
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
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: String,
}
