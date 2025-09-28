use serde::Deserialize;

/// A UnitGraph represents the output of `cargo build --unit-graph`. This output
/// is documented here[^1] and defined in source code here[^2].
///
/// [^1]: https://doc.rust-lang.org/cargo/reference/unstable.html#unit-graph
/// [^2]: https://github.com/rust-lang/cargo/blob/c24e1064277fe51ab72011e2612e556ac56addf7/src/cargo/core/compiler/unit_graph.rs#L43-L48
#[derive(Debug, Deserialize)]
pub struct UnitGraph {
    version: u64,
    units: Vec<UnitGraphUnit>,
    roots: Vec<usize>,
}

#[derive(Debug, Deserialize)]
pub struct UnitGraphUnit {
    pkg_id: String,
    target: cargo_metadata::Target,
    profile: UnitGraphProfile,
    platform: Option<String>,
    mode: CargoCompileMode,
    features: Vec<String>,
    #[serde(skip)]
    is_std: bool,
    dependencies: Vec<UnitGraphDependency>,
}

#[derive(Debug, Deserialize)]
pub struct UnitGraphProfile {
    name: String,
    opt_level: String,
    lto: String,
    codegen_units: Option<u64>,
    debuginfo: Option<u64>,
    debug_assertions: bool,
    overflow_checks: bool,
    rpath: bool,
    incremental: bool,
    panic: UnitGraphProfilePanicStrategy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitGraphProfilePanicStrategy {
    Unwind,
    Abort,
}

#[derive(Debug, Deserialize)]
pub struct UnitGraphDependency {
    index: usize,
    extern_crate_name: String,
    public: bool,
    noprelude: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CargoCompileMode {
    Test,
    Build,
    Check,
    Doc,
    Doctest,
    Docscrape,
    #[serde(rename = "run-custom-build")]
    RunCustomBuild,
}
