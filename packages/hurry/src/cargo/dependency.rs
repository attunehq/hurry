use std::collections::HashSet;

use bon::Builder;
use derive_more::{Debug, Display};

/// A Dependency identifies a dependency package in a Cargo build.
///
/// Note that these are not sufficient for keying compiled artifacts! This type
/// is designed to be constructible from `cargo metadata` output, which is
/// resolved before build invocation (and therefore doesn't know things like
/// which optimizations and features are enabled). This only identifies the
/// package and version being used.
///
/// For keying compiled artifacts, use the `DependencyBuild` type, which
/// includes the actual information used to compile the dependency.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Builder)]
#[display("{package_name}@{version}")]
pub struct Dependency {
    /// The name of the dependency.
    #[builder(into)]
    pub package_name: String,

    /// The name of the dependency's library crate target.
    ///
    /// This is often the same as the name of the dependency package. However,
    /// it can be different in two cases:
    /// 1. Users can customize this name in their `Cargo.toml`[^1]. For example,
    ///    the package `xml-rs` names its library crate `xml`, which compiles to
    ///    an rlib called `libxml`[^2] and the package `build-rs` used to name
    ///    its library crate `build`[^3].
    /// 2. In packages whose names are not valid Rust identifiers (in
    ///    particular, packages with hyphens in their names), Cargo will
    ///    automatically convert the library crate name into a Rust identifier
    ///    by converting hyphens into underscores[^1].
    ///
    /// [^1]: https://doc.rust-lang.org/cargo/reference/cargo-targets.html#the-name-field
    /// [^2]: https://github.com/kornelski/xml-rs/blob/9ce8c90821a7ea1d3cb82753caab88482788a1d0/Cargo.toml#L2
    /// [^3]: https://github.com/rust-lang/cargo/blob/6655e485135d1c339864b4e4f4147cb60144ec48/Cargo.toml#L13
    #[builder(into)]
    pub lib_name: String,

    /// The version of the dependency.
    #[builder(into)]
    pub version: String,
}

/// A DependencyBuild is a specific build of a dependency crate, including other
/// build-identifying information such as target, profile, and features.
#[derive(Clone, Eq, PartialEq, Debug, Builder)]
pub struct DependencyBuild {
    pub package: Dependency,

    pub dependencies: Vec<DependencyBuild>,

    /// The target triple for which the dependency
    /// is being or has been built.
    ///
    /// Examples:
    /// ```not_rust
    /// aarch64-apple-darwin
    /// x86_64-unknown-linux-gnu
    /// ```
    #[builder(into)]
    pub target: String,

    pub profile: Optimizations,

    pub features: HashSet<String>,

    #[builder(into)]
    pub edition: String,

    #[builder(into)]
    pub extra_filename: String,
}

#[derive(Clone, Eq, PartialEq, Debug, Builder)]
pub struct Optimizations {
    #[builder(into)]
    pub opt_level: String,
    #[builder(into)]
    pub debuginfo: String,
    pub debug_assertions: bool,
    pub overflow_checks: bool,
    pub test: bool,
}
