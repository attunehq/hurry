use std::collections::HashMap;

use serde::Deserialize;

use crate::cargo::unit_graph::CargoCompileMode;

#[derive(Debug, Deserialize)]
pub struct BuildPlan {
    invocations: Vec<BuildPlanInvocation>,
    inputs: Vec<String>,
}

// Note that these fields are all undocumented. To see their definition, see
// https://github.com/rust-lang/cargo/blob/0436f86288a4d9bce1c712c4eea5b05eb82682b9/src/cargo/core/compiler/build_plan.rs#L21-L34
#[derive(Debug, Deserialize)]
pub struct BuildPlanInvocation {
    package_name: String,
    package_version: String,
    target_kind: Vec<cargo_metadata::TargetKind>,
    kind: Option<String>,
    compile_mode: CargoCompileMode,
    deps: Vec<usize>,
    outputs: Vec<String>,
    links: HashMap<String, String>,
    program: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: String,
}
