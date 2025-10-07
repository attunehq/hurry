use serde::Deserialize;

/// A UnitGraph represents the output of `cargo build --unit-graph`. This output
/// is documented here[^1] and defined in source code here[^2].
///
/// [^1]: https://doc.rust-lang.org/cargo/reference/unstable.html#unit-graph
/// [^2]: https://github.com/rust-lang/cargo/blob/c24e1064277fe51ab72011e2612e556ac56addf7/src/cargo/core/compiler/unit_graph.rs#L43-L48
#[derive(Debug, Deserialize)]
pub struct UnitGraph {
    pub version: u64,
    pub units: Vec<UnitGraphUnit>,
    pub roots: Vec<usize>,
}

#[derive(Debug, Deserialize)]
pub struct UnitGraphUnit {
    pub pkg_id: String,
    pub target: cargo_metadata::Target,
    pub profile: UnitGraphProfile,
    pub platform: Option<String>,
    pub mode: CargoCompileMode,
    pub features: Vec<String>,
    #[serde(skip)]
    pub is_std: bool,
    pub dependencies: Vec<UnitGraphDependency>,
}

#[derive(Debug, Deserialize)]
pub struct UnitGraphProfile {
    pub name: String,
    pub opt_level: String,
    pub lto: String,
    pub codegen_units: Option<u64>,
    pub debuginfo: Option<u64>,
    pub debug_assertions: bool,
    pub overflow_checks: bool,
    pub rpath: bool,
    pub incremental: bool,
    pub panic: UnitGraphProfilePanicStrategy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnitGraphProfilePanicStrategy {
    Unwind,
    Abort,
}

#[derive(Debug, Deserialize)]
pub struct UnitGraphDependency {
    pub index: usize,
    pub extern_crate_name: String,
    pub public: bool,
    pub noprelude: bool,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CargoCompileMode {
    Test,
    Build,
    Check,
    Doc,
    Doctest,
    Docscrape,
    RunCustomBuild,
}
