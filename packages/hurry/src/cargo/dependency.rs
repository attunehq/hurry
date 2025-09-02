use bon::{Builder, bon};
use derive_more::{Debug, Display};

use crate::hash::Blake3;

/// A Cargo dependency.
///
/// This isn't the full set of information about a dependency, but it's enough
/// to identify it uniquely within a workspace for the purposes of caching.
///
/// Each piece of data in this struct is used to build the "cache key"
/// for the dependency; the intention is that each dependency is cached
/// independently and restored in other projects based on a matching
/// cache key derived from other instances of `hurry` reading the
/// `Cargo.lock` and other workspace/compiler/platform metadata.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Builder)]
#[display("{name}@{version}")]
pub struct Dependency {
    /// The name of the dependency.
    #[builder(into)]
    pub name: String,

    /// The version of the dependency.
    #[builder(into)]
    pub version: String,

    /// The checksum of the dependency.
    #[builder(into)]
    pub checksum: String,

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
}

impl Dependency {
    /// Hash key for the dependency.
    pub fn key(&self) -> Blake3 {
        Self::key_for()
            .checksum(&self.checksum)
            .name(&self.name)
            .target(&self.target)
            .version(&self.version)
            .call()
    }
}

#[bon]
impl Dependency {
    /// Produce a hash key for all the fields of a dependency
    /// without having to actually make a dependency instance
    /// (which may involve cloning).
    #[builder]
    pub fn key_for(
        name: impl AsRef<[u8]>,
        version: impl AsRef<[u8]>,
        checksum: impl AsRef<[u8]>,
        target: impl AsRef<[u8]>,
    ) -> Blake3 {
        let name = name.as_ref();
        let version = version.as_ref();
        let checksum = checksum.as_ref();
        let target = target.as_ref();
        Blake3::from_fields([name, version, checksum, target])
    }
}