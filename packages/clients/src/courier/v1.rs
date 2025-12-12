//! Courier v1 API types and client.
//!
//! # Intrinsics
//!
//! This module attempts to push as much context as reasonably possible out of
//! the shared layer, which means it makes heavy use of opaque values and
//! generics. It also tries to use the semantically least broad data types
//! possible.
//!
//! ## Data type broadness
//!
//! Collections or container types attempt to use the "least broad"
//! implementation when reasonable. For example, a `HashSet` is "less broad"
//! than a `Vec` because the latter implies ordering while the former does not.
//! The concept of "ordering" is considered "more broad" than the concept of
//! "equality", even though both require explicit opt-in, because equality
//! can nearly always be derived: it's rarely a matter of business logic. In
//! contrast, ordering is nearly always inherently dependant on business logic.
//!
//! However, sometimes this isn't possible: if the contents aren't able to be
//! compared for equality or cannot be hashed, then obviously a `HashSet` won't
//! work. It's also sometimes not _desired_ to imply that order doesn't matter.
//!
//! As such, if you see a `Vec` over a `HashSet` in this module, you can be sure
//! that Courier treats order like it matters and therefore so should the
//! client.
//!
//! ## Opaque values
//!
//! The types in this module make relatively heavy use of "opaque values", such
//! as but not limited to `Key`, `DiskPath`, and `Fingerprint`. The intention
//! with these types is to allow applications like Hurry to encode arbitrary
//! types into these values while freeing Courier to treat them as opaque types.
//!
//! For example, Hurry might parse a `DiskPath` as a `QualifiedPath` or as a
//! `TypedPath` depending on the data structure and use case involved, but
//! collapsing both into the opaque value of `DiskPath` frees Courier from
//! needing to know or care about the difference: it just stores and returns
//! what Courier provides.

use std::{cmp::Ordering, fmt::Display, str::FromStr};

use bon::Builder;
use color_eyre::eyre::{self, Context, bail, eyre};
use derive_more::{Debug, Display};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tracing::{instrument, trace};

pub mod cache;
pub mod cas;

#[cfg(feature = "client")]
mod client;

#[cfg(feature = "client")]
pub use client::Client;

/// Opaque value signifying a CAS key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Display)]
#[display("{}", self.to_hex())]
pub struct Key(#[debug("{:?}", self.to_hex())] Vec<u8>);

impl Key {
    /// View the key as a hex string.
    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }

    /// Attempt to parse the key from a hex string.
    #[instrument(fields(hex = hex.as_ref()))]
    pub fn from_hex(hex: impl AsRef<str>) -> color_eyre::Result<Self> {
        let bytes = hex::decode(hex.as_ref()).context("decode hex")?;
        let len = bytes.len();
        trace!(?bytes, ?len, "decoded hex");
        if len != 32 {
            bail!("invalid hash length");
        }
        Ok(Self(bytes))
    }

    /// View the key as bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Parse a key from raw bytes (the inverse of `as_bytes`).
    ///
    /// This is used when deserializing keys from the database or other binary
    /// formats. The bytes must be exactly 32 bytes (a blake3 hash).
    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> color_eyre::Result<Self> {
        let bytes = bytes.as_ref();
        let len = bytes.len();
        if len != 32 {
            bail!("invalid hash length: expected 32 bytes, got {len}");
        }
        Ok(Self(bytes.to_vec()))
    }

    /// Create a key from a blake3 hash.
    pub fn from_blake3(hash: blake3::Hash) -> Self {
        Self(hash.as_bytes().to_vec())
    }

    /// Hash the contents of a buffer to create a key.
    ///
    /// This computes the blake3 hash of the provided buffer and returns the
    /// resulting key. Use this when you have file contents or other data
    /// that you want to content-address. This is NOT for parsing keys that
    /// are already in binary format: use `from_bytes` for that.
    pub fn from_buffer(buffer: impl AsRef<[u8]>) -> Self {
        let buffer = buffer.as_ref();
        let mut hasher = blake3::Hasher::new();
        hasher.update(buffer);
        let hash = hasher.finalize();
        Self::from_blake3(hash)
    }

    /// Hash the contents of the iterator in order.
    pub fn from_fields(fields: impl IntoIterator<Item = impl AsRef<[u8]>>) -> Self {
        let mut hasher = blake3::Hasher::new();
        for field in fields {
            hasher.update(field.as_ref());
        }
        let hash = hasher.finalize();
        Self::from_blake3(hash)
    }
}

impl From<&Key> for Key {
    fn from(key: &Key) -> Self {
        key.clone()
    }
}

impl PartialEq<blake3::Hash> for Key {
    fn eq(&self, other: &blake3::Hash) -> bool {
        self.0 == other.as_bytes()
    }
}

impl PartialEq<blake3::Hash> for &Key {
    fn eq(&self, other: &blake3::Hash) -> bool {
        self.0 == other.as_bytes()
    }
}

impl Serialize for Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex = String::deserialize(deserializer)?;
        Self::from_hex(&hex).map_err(serde::de::Error::custom)
    }
}

/// Opaque value signifying a path on disk.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Display, Serialize, Deserialize)]
#[display("{}", self.0)]
pub struct DiskPath(String);

impl DiskPath {
    /// Create a new instance from a string.
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// View the underlying data as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<S: Into<String>> From<S> for DiskPath {
    fn from(path: S) -> Self {
        Self::new(path)
    }
}

impl AsRef<str> for DiskPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&DiskPath> for DiskPath {
    fn from(path: &DiskPath) -> Self {
        path.clone()
    }
}

/// Opaque value signifying a fingerprint that uniquely identifies a library.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Display, Serialize, Deserialize)]
#[display("{}", self.0)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// Create a new instance from a string.
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// View the underlying data as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<S: Into<String>> From<S> for Fingerprint {
    fn from(path: S) -> Self {
        Self::new(path)
    }
}

impl AsRef<str> for Fingerprint {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&Fingerprint> for Fingerprint {
    fn from(fingerprint: &Fingerprint) -> Self {
        fingerprint.clone()
    }
}

/// Opaque value signifying a unit hash that uniquely identifies a `SavedUnit`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Display, Serialize, Deserialize)]
#[display("{}", self.0)]
pub struct SavedUnitHash(String);

impl SavedUnitHash {
    /// Create a new instance from a string.
    pub fn new(hash: impl Into<String>) -> Self {
        Self(hash.into())
    }

    /// View the underlying data as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<S: Into<String>> From<S> for SavedUnitHash {
    fn from(path: S) -> Self {
        Self::new(path)
    }
}

impl AsRef<str> for SavedUnitHash {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&SavedUnitHash> for SavedUnitHash {
    fn from(hash: &SavedUnitHash) -> Self {
        hash.clone()
    }
}

/// Common metadata fields present in all unit plan types.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct UnitPlanInfo {
    /// The directory hash of the unit, which is used to construct the unit's
    /// file directories.
    ///
    /// See the `*_dir` methods on `CompilationFiles`[^1] for details.
    ///
    /// [^1]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/compiler/build_runner/compilation_files.rs#L117
    #[builder(into)]
    pub unit_hash: SavedUnitHash,

    /// The package name of this unit.
    ///
    /// This is used to reconstruct expected output directories. See the `*_dir`
    /// methods on `CompilationFiles`[^1] for details.
    ///
    /// [^1]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/compiler/build_runner/compilation_files.rs#L117
    #[builder(into)]
    pub package_name: String,

    /// The crate name of this unit.
    ///
    /// Note that this is not necessarily the _extern_ crate name, which can be
    /// affected by directives like `replace` and `patch`, and that the crate
    /// name used in fingerprints is the extern crate name[^1], not the
    /// canonical crate name.
    ///
    /// [^1]: https://github.com/attunehq/cargo/blob/7a93b36f1ae2f524d93efd16cd42864675f3e15b/src/cargo/core/compiler/fingerprint/mod.rs#L1366
    // FIXME: To properly support `replace` and `patch` directives, we need to
    // also calculate an extern_crate_name for each edge in the dependency
    // graph. Note that this is a per-edge value, not a per-unit value. Perhaps
    // we can derive this from the unit graph?
    #[builder(into)]
    pub crate_name: String,

    /// The unit's target architecture, if set.
    ///
    /// When None, this unit is not being compiled with a specific `--target` in
    /// mind, and therefore is being compiled for the host architecture.
    ///
    /// Note that some units (e.g. proc macros, build script compilations, and
    /// dependencies thereof) are compiled for the host architecture even when
    /// `--target` is set to a different architecture. This field already takes
    /// that into account.
    #[builder(into)]
    pub target_arch: Option<String>,
}

impl From<&UnitPlanInfo> for UnitPlanInfo {
    fn from(info: &UnitPlanInfo) -> Self {
        info.clone()
    }
}

/// A saved file in the cargo cache.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct SavedFile {
    pub executable: bool,

    #[builder(into)]
    pub object_key: Key,

    #[builder(into)]
    pub path: DiskPath,
}

impl From<&SavedFile> for SavedFile {
    fn from(file: &SavedFile) -> Self {
        file.clone()
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum SavedUnit {
    LibraryCrate(LibraryFiles, LibraryCrateUnitPlan),
    BuildScriptCompilation(BuildScriptCompiledFiles, BuildScriptCompilationUnitPlan),
    BuildScriptExecution(BuildScriptOutputFiles, BuildScriptExecutionUnitPlan),
}

impl SavedUnit {
    /// Read the unit hash from this saved unit.
    pub fn unit_hash(&self) -> &SavedUnitHash {
        match self {
            SavedUnit::LibraryCrate(_, plan) => &plan.info.unit_hash,
            SavedUnit::BuildScriptCompilation(_, plan) => &plan.info.unit_hash,
            SavedUnit::BuildScriptExecution(_, plan) => &plan.info.unit_hash,
        }
    }
}

impl From<&SavedUnit> for SavedUnit {
    fn from(unit: &SavedUnit) -> Self {
        unit.clone()
    }
}

/// Libraries are usually associated with 7 files:
///
/// - 2 output files (an `.rmeta` and an `.rlib`)
/// - 1 rustc dep-info (`.d`) file in the `deps` folder
/// - 4 files in the fingerprint directory
///   - An `EncodedDepInfo` file
///   - A fingerprint hash
///   - A fingerprint JSON
///   - An invoked timestamp
///
/// Of these files, the fingerprint hash, fingerprint JSON, and invoked
/// timestamp are all reconstructed from fingerprint information during
/// restoration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct LibraryFiles {
    /// These files come from the build plan's `outputs` field.
    // TODO: Can we specify this even more narrowly (e.g. with an `rmeta` and
    // `rlib` field)? I know there are other possible output files (e.g. `.so`
    // for proc macros on Linux and `.dylib` for something on macOS), but I
    // don't know what the enumerated list is.
    pub output_files: Vec<SavedFile>,

    /// This information is parsed from the initial fingerprint created after
    /// the build, and is used to dynamically reconstruct fingerprints on
    /// restoration.
    pub fingerprint: Fingerprint,

    /// This file is always at a known path in
    /// `deps/{package_name}-{unit_hash}.d`.
    pub dep_info_file: Key,

    /// This file is always at a known path in
    /// `.fingerprint/{package_name}-{unit_hash}/dep-lib-{crate_name}`. It can
    /// be safely relocatably copied because the `EncodedDepInfo` struct only
    /// ever contains relative file path information (note that deps always have
    /// a `DepInfoPathType`, which is either `PackageRootRelative` or
    /// `BuildRootRelative`)[^1].
    ///
    /// [^1]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/core/compiler/fingerprint/dep_info.rs#L112
    pub encoded_dep_info_file: Key,
}

impl From<&LibraryFiles> for LibraryFiles {
    fn from(files: &LibraryFiles) -> Self {
        files.clone()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct LibraryCrateUnitPlan {
    /// Common metadata fields present in all unit plan variants.
    #[serde(flatten)]
    #[builder(into)]
    pub info: UnitPlanInfo,

    /// The path to the source file on disk.
    #[builder(into)]
    pub src_path: DiskPath,

    /// The paths to output files on disk.
    #[builder(into)]
    pub outputs: Vec<DiskPath>,
}

impl From<&LibraryCrateUnitPlan> for LibraryCrateUnitPlan {
    fn from(plan: &LibraryCrateUnitPlan) -> Self {
        plan.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct BuildScriptCompiledFiles {
    /// This field contains the contents of the compiled build script program at
    /// `build_script_{build_script_entrypoint}-{build_script_compilation_unit_hash}`
    /// and hard linked at `build-script-{build_script_entrypoint}`.
    ///
    /// We need both of these files: the hard link is the file that's actually
    /// executed in the build plan, but the full path with the unit hash is the
    /// file that's tracked by the fingerprint.
    #[builder(into)]
    pub compiled_program: Key,

    /// The rustc dep-info file in the build directory.
    #[builder(into)]
    pub dep_info_file: Key,

    /// This fingerprint is stored in `.fingerprint`, and is used to derive the
    /// timestamp, fingerprint hash file, and fingerprint JSON file.
    #[builder(into)]
    pub fingerprint: Fingerprint,

    /// This `EncodedDepInfo` (i.e. Cargo dep-info) file is stored in
    /// `.fingerprint`, and is directly saved and restored.
    #[builder(into)]
    pub encoded_dep_info_file: Key,
}

impl From<&BuildScriptCompiledFiles> for BuildScriptCompiledFiles {
    fn from(files: &BuildScriptCompiledFiles) -> Self {
        files.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct BuildScriptCompilationUnitPlan {
    /// Common metadata fields present in all unit plan variants.
    #[serde(flatten)]
    #[builder(into)]
    pub info: UnitPlanInfo,

    /// The path to the build script's main entrypoint source file. This is
    /// usually `build.rs` within the package's source code, but can vary if
    /// the package author sets `package.build` in the package's
    /// `Cargo.toml`, which changes the build script's name[^1].
    ///
    /// This is parsed from the rustc invocation arguments in the unit's
    /// build plan invocation.
    ///
    /// This is used to rewrite the build script compilation's fingerprint
    /// on restore.
    ///
    /// [^1]: https://doc.rust-lang.org/cargo/reference/manifest.html#the-build-field
    #[builder(into)]
    pub src_path: DiskPath,
}

impl From<&BuildScriptCompilationUnitPlan> for BuildScriptCompilationUnitPlan {
    fn from(plan: &BuildScriptCompilationUnitPlan) -> Self {
        plan.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct BuildScriptOutputFiles {
    #[builder(default, with = |i: impl IntoIterator<Item = impl Into<SavedFile>>| i.into_iter().map(Into::into).collect())]
    pub out_dir_files: Vec<SavedFile>,

    #[builder(into)]
    pub stdout: Key,

    #[builder(into)]
    pub stderr: Key,

    #[builder(into)]
    pub fingerprint: Fingerprint,
}

impl From<&BuildScriptOutputFiles> for BuildScriptOutputFiles {
    fn from(files: &BuildScriptOutputFiles) -> Self {
        files.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct BuildScriptExecutionUnitPlan {
    /// Common metadata fields present in all unit plan variants.
    #[serde(flatten)]
    #[builder(into)]
    pub info: UnitPlanInfo,

    /// The entrypoint module name of the compiled build script program after
    /// linkage (i.e. using the original build script name, which is what Cargo
    /// uses to name the execution unit files).
    #[builder(into)]
    pub build_script_program_name: String,
}

impl From<&BuildScriptExecutionUnitPlan> for BuildScriptExecutionUnitPlan {
    fn from(plan: &BuildScriptExecutionUnitPlan) -> Self {
        plan.clone()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct GlibcVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Display for GlibcVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for GlibcVersion {
    type Err = eyre::Report;

    // For reference, see the full list of glibc versions[^1].
    //
    // [^1]: https://sourceware.org/glibc/wiki/Glibc%20Timeline
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('.');
        let major = parts
            .next()
            .ok_or(eyre!("could not parse major version"))?
            .parse()?;
        let minor = parts
            .next()
            .ok_or(eyre!("could not parse minor version"))?
            .parse()?;
        // Patch versions are optional, and default to zero for comparison
        // purposes.
        let patch = parts
            .next()
            .map(|s| {
                s.parse::<u32>()
                    .map_err(|e| eyre!("could not parse patch version: {e}"))
            })
            .unwrap_or(Ok(0))?;
        // Make sure there are no remaining parts.
        if parts.next().is_some() {
            bail!("expected end of string");
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

impl Ord for GlibcVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
    }
}

impl PartialOrd for GlibcVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
