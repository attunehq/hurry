use bon::{Builder, bon};
use derive_more::{Debug, Display};

use crate::hash::Blake3;

/// A third-party Cargo dependency for cache identification.
///
/// Contains the minimal set of information needed to uniquely identify
/// a dependency across different workspaces and machines for caching purposes.
/// Each dependency gets cached independently based on its cache key.
///
/// ## Cache Key Components
/// All fields contribute to the cache key generation:
/// - `name`: Crate name from `Cargo.lock`
/// - `version`: Exact version from `Cargo.lock`
/// - `checksum`: Registry checksum ensuring content integrity
/// - `target`: Compilation target (e.g., `x86_64-unknown-linux-gnu`)
///
/// ## Contract
/// - Only represents third-party dependencies from the default registry
/// - Cache keys must match exactly for artifacts to be reused
/// - Target triple ensures platform-specific artifacts aren't mixed
///
/// ## TODO
/// - We probably need to move to a model where we search for matching caches
///   based on the elements in this instead of opaque cache keys.
/// - We aren't including things that should almost definitely be included,
///   for example active features.
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
    /// Generate a cache key for this dependency.
    ///
    /// Creates a Blake3 hash from all dependency fields to uniquely
    /// identify this dependency for cache storage and retrieval.
    #[deprecated = "Refer to TODO's on this type"]
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
    /// Generate a cache key without creating a Dependency instance.
    ///
    /// Use this when you have the fields that would normally go into a
    /// [`Dependency`] instance borrowed.
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
