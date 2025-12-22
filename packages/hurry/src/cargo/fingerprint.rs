use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use color_eyre::{
    Result,
    eyre::{OptionExt as _, bail},
};
use derive_more::Debug;
use rustc_stable_hash::StableSipHasher128;
use serde::{Deserialize, Serialize, de, ser};
use tracing::{debug, instrument, trace};

use crate::{fs, path::AbsFilePath};

/// A Cargo fingerprint. This struct is vendored and modified from the Cargo
/// source code. In particular, some `serde(skip)`ed fields are elided. For
/// details, see the module documentation[^1] and the struct definition[^2].
///
/// We parse and rewrite fingerprints because they are non-relocatable. In
/// particular, the `path` field is an absolute path for crates whose base
/// source file is located outside of the workspace root, including packages
/// downloaded from the crates.io registry. On save, we rewrite this path to be
/// relative to `CARGO_HOME`, and on restore, we rewrite this field with the
/// correct source path for the destination machine. Note that because this
/// changes the fingerprint hash on relocation, this also changes the `deps`
/// field of all downstream dependents, which must also then have their
/// fingerprints rewritten.
///
/// All Cargo units (library compilation, build script compilation, and build
/// script execution) have corresponding fingerprint directories, located at
/// `{profile}/.fingerprint/{package_name}-{unit_hash}`. Within these
/// directories, this fingerprint struct's hash is saved in the
/// `{kind}-{crate_name}` file, and a JSON serialization of its data is saved in
/// `{kind}-{crate_name}.json`.
///
/// Fingerprints are generated when `prepare_target`[^3] is called (which is the
/// main fingerprint logic entrypoint called by
/// `cargo::core::compiler::compile`), and their saved location is defined in
/// `fingerprint_file_path`[^4].
///
/// [^1]: https://doc.rust-lang.org/nightly/nightly-rustc/cargo/core/compiler/fingerprint/struct.Fingerprint.html
/// [^2]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/core/compiler/fingerprint/mod.rs#L600
/// [^3]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/core/compiler/fingerprint/mod.rs#L431
/// [^4]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/core/compiler/build_runner/compilation_files.rs#L294
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fingerprint {
    /// Hash of the version of `rustc` used. This comes from `rustc -vV`.
    rustc: u64,
    /// Sorted list of cfg features enabled.
    features: String,
    /// Sorted list of all the declared cfg features.
    declared_features: String,
    /// Hash of the `Target` struct, including the target name,
    /// package-relative source path, edition, etc.
    target: u64,
    /// Hash of the [`Profile`], [`CompileMode`], and any extra flags passed via
    /// `cargo rustc` or `cargo rustdoc`.
    ///
    /// [`Profile`]: crate::core::profiles::Profile
    /// [`CompileMode`]: crate::core::compiler::CompileMode
    profile: u64,
    /// Hash of the path to the base source file. This is relative to the
    /// workspace root for path members, or absolute for other sources.
    pub path: u64,
    /// Fingerprints of dependencies.
    pub deps: Vec<DepFingerprint>,
    /// Information about the inputs that affect this Unit (such as source
    /// file mtimes or build script environment variables).
    local: Arc<Mutex<Vec<LocalFingerprint>>>,
    /// Cached hash of the [`Fingerprint`] struct. Used to improve performance
    /// for hashing.
    #[serde(skip)]
    memoized_hash: Arc<Mutex<Option<u64>>>,
    /// RUSTFLAGS/RUSTDOCFLAGS environment variable value (or config value).
    rustflags: Vec<String>,
    /// Hash of various config settings that change how things are compiled.
    config: u64,
    /// The rustc target. This is only relevant for `.json` files, otherwise
    /// the metadata hash segregates the units.
    compile_kind: u64,
}

impl Fingerprint {
    fn new() -> Fingerprint {
        Fingerprint {
            rustc: 0,
            target: 0,
            profile: 0,
            path: 0,
            features: String::new(),
            declared_features: String::new(),
            deps: Vec::new(),
            local: Arc::new(Mutex::new(Vec::new())),
            memoized_hash: Arc::new(Mutex::new(None)),
            rustflags: Vec::new(),
            config: 0,
            compile_kind: 0,
        }
    }

    /// For performance reasons fingerprints will memoize their own hash, but
    /// there's also internal mutability with its `local` field which can
    /// change, for example with build scripts, during a build.
    ///
    /// This method can be used to bust all memoized hashes just before a build
    /// to ensure that after a build completes everything is up-to-date.
    pub fn clear_memoized(&self) {
        *self.memoized_hash.lock().unwrap() = None;
    }

    pub fn hash_u64(&self) -> u64 {
        if let Some(s) = *self.memoized_hash.lock().unwrap() {
            return s;
        }
        let ret = util_hash_u64(self);
        *self.memoized_hash.lock().unwrap() = Some(ret);
        ret
    }

    /// Render the fingerprint hash as a string for comparison, using Cargo's
    /// rendering logic.
    pub fn fingerprint_hash(&self) -> String {
        // Cargo uses `util::to_hex` to write[^1] and compare[^2] fingerprint hashes,
        // which serializes using `to_le_bytes`[^3].
        //
        // [^1]: https://github.com/attunehq/cargo/blob/d59205e6303b011e2c7b1fcd92946a5e783b77bb/src/cargo/core/compiler/fingerprint/mod.rs#L1946
        // [^2]: https://github.com/attunehq/cargo/blob/d59205e6303b011e2c7b1fcd92946a5e783b77bb/src/cargo/core/compiler/fingerprint/mod.rs#L2034
        // [^3]: https://github.com/attunehq/cargo/blob/d59205e6303b011e2c7b1fcd92946a5e783b77bb/src/cargo/util/hex.rs#L7
        hex::encode(self.hash_u64().to_le_bytes())
    }

    pub async fn read(
        fingerprint_json_path: AbsFilePath,
        fingerprint_hash_path: AbsFilePath,
    ) -> Result<Fingerprint> {
        let fingerprint_json = fs::must_read_buffered_utf8(&fingerprint_json_path).await?;
        let fingerprint: Fingerprint = serde_json::from_str(&fingerprint_json)?;

        let fingerprint_hash = fs::must_read_buffered_utf8(&fingerprint_hash_path).await?;
        // Sanity check that the fingerprint hashes match.
        if fingerprint.fingerprint_hash() != fingerprint_hash {
            bail!("fingerprint hash mismatch");
        }

        Ok(fingerprint)
    }

    /// Create a new Fingerprint with rewritten path and dependencies.
    #[instrument(skip(self, dep_fingerprints))]
    pub fn rewrite(
        mut self,
        path: Option<PathBuf>,
        dep_fingerprints: &mut HashMap<u64, Fingerprint>,
    ) -> Result<Fingerprint> {
        let old = self.hash_u64();

        // First, rewrite the `path` field.
        //
        // Note that certain unit types (in particular build script
        // executions[^1]), never set the `path` field, and therefore should not
        // have it rewritten. We sanity check this case by checking whether
        // `path` is zero in the original fingerprint.
        //
        // [^1]: https://github.com/attunehq/cargo/blob/21f1bfe23aa3fafd6205b8e3368a499466336bb9/src/cargo/core/compiler/fingerprint/mod.rs#L1696
        if let Some(path) = path.clone() {
            if self.path == 0 {
                bail!("tried to rewrite unset fingerprint path");
            }
            // WARNING: Even though this method accepts any hashable, this MUST
            // be a `PathBuf`! Other types representing the same value will hash
            // to a different value, which will break fingerprint calculation!
            self.path = util_hash_u64(path);
        }
        debug!(?path, path_hash = ?self.path.clone(), "rewritten fingerprint path");

        // Then, rewrite the `deps` field.
        //
        // We don't actually have enough information to synthesize our own
        // DepFingerprints (in particular, it would be very annoying to derive
        // the config fields independently). But the old fingerprint hashes are
        // unique and uniquely identify each unit hash[^1], and we know our old
        // fingerprint hash! So we save a map of the old fingerprint hashes to
        // the replacement fingerprint hashes, and use that to look up the
        // correct replacement fingerprint hash in future DepFingerprints,
        // leaving all other fields untouched.
        //
        // This works because we know the units are in dependency order, so
        // previous replacement fingerprint hashes will always have already been
        // calculated when we need them.
        //
        // [^1]: This is actually only true under certain conditions! In
        //     particular, the old fingerprint must have been for a unit
        //     compiled with the same src_path and against dependencies with the
        //     same src_path[^2]. To ensure this invariant, we must rewrite
        //     fingerprints to have the same synthetic `src_path` (rooted at
        //     `CARGO_HOME`) before saving them to the database.
        // [^2]: We know this to be true from comparing how fingerprint hashes
        //     are calculated[^3][^4] with how unit hashes are calculated[^5].
        //     Note that the only fields used in the fingerprint hash that vary
        //     separately from those used in the unit hash are the `path` and
        //     `deps` fields (note in particular that the local fingerprints do
        //     not vary between host machines).
        // [^3]: https://github.com/attunehq/cargo/blob/d59205e6303b011e2c7b1fcd92946a5e783b77bb/src/cargo/core/compiler/fingerprint/mod.rs#L1506
        // [^4]: https://github.com/attunehq/cargo/blob/d59205e6303b011e2c7b1fcd92946a5e783b77bb/src/cargo/core/compiler/fingerprint/mod.rs#L1349
        // [^5]: https://github.com/attunehq/cargo/blob/d59205e6303b011e2c7b1fcd92946a5e783b77bb/src/cargo/core/compiler/build_runner/compilation_files.rs#L765
        debug!("rewrite fingerprint deps: start");
        for dep in self.deps.iter_mut() {
            debug!(?dep, "rewriting fingerprint dep");
            let old_dep_fingerprint = dep.fingerprint.hash_u64();
            trace!(
                ?old_dep_fingerprint,
                ?dep_fingerprints,
                "searching for dependency fingerprint hash"
            );
            dep.fingerprint = dep_fingerprints
                .get(&old_dep_fingerprint)
                .ok_or_eyre("dependency fingerprint hash not found")?
                .clone();
        }
        debug!("rewrite fingerprint deps: done");

        // Clear and recalculate fingerprint hash.
        self.clear_memoized();
        // `new` is only ever used in the debug!() call but cannot be inlined
        // because hash_u64 calls Fingerprint::hash which has its own debug!()
        // call and event generation cannot be nested[^1].
        //
        // [^1]: https://github.com/tokio-rs/tracing/issues/2448
        let new = self.hash_u64();
        debug!(?old, ?new, "rewritten fingerprint hash");

        // Save unit fingerprint (for future dependents).
        dep_fingerprints.insert(old, self.clone());

        Ok(self)
    }
}

impl Hash for Fingerprint {
    fn hash<H: Hasher>(&self, h: &mut H) {
        let Fingerprint {
            rustc,
            ref features,
            ref declared_features,
            target,
            path,
            profile,
            ref deps,
            ref local,
            config,
            compile_kind,
            ref rustflags,
            ..
        } = *self;
        debug!(?path, "hashing fingerprint");
        let local = local.lock().unwrap();
        (
            rustc,
            features,
            declared_features,
            target,
            path,
            profile,
            &*local,
            config,
            compile_kind,
            rustflags,
        )
            .hash(h);

        h.write_usize(deps.len());
        for DepFingerprint {
            pkg_id,
            name,
            public,
            fingerprint,
        } in deps
        {
            pkg_id.hash(h);
            name.hash(h);
            public.hash(h);
            // use memoized dep hashes to avoid exponential blowup
            h.write_u64(fingerprint.hash_u64());
        }
    }
}

/// This function is taken from `cargo::util::hex`[^1].
///
/// [^1]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/util/hex.rs#L10
pub fn util_hash_u64<H: Hash>(hashable: H) -> u64 {
    let mut hasher = StableSipHasher128::new();
    hashable.hash(&mut hasher);
    Hasher::finish(&hasher)
}

/// This is taken from the Cargo source code at
/// `cargo::core::compiler::fingerprint`[^1].
///
/// [^1]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/core/compiler/fingerprint/mod.rs#L557
#[derive(Debug, Clone)]
pub struct DepFingerprint {
    /// The hash of the package id that this dependency points to
    pub pkg_id: u64,
    /// The crate name we're using for this dependency, which if we change we'll
    /// need to recompile!
    pub name: String,
    /// Whether or not this dependency is flagged as a public dependency or not.
    pub public: bool,
    /// The dependency's fingerprint we recursively point to, containing all the
    /// other hash information we'd otherwise need.
    pub fingerprint: Fingerprint,
}

impl Serialize for DepFingerprint {
    fn serialize<S>(&self, ser: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        (
            &self.pkg_id,
            &self.name,
            &self.public,
            &self.fingerprint.hash_u64(),
        )
            .serialize(ser)
    }
}

impl<'de> Deserialize<'de> for DepFingerprint {
    fn deserialize<D>(d: D) -> Result<DepFingerprint, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let (pkg_id, name, public, hash) = <(u64, String, bool, u64)>::deserialize(d)?;
        Ok(DepFingerprint {
            pkg_id,
            name,
            public,
            fingerprint: Fingerprint {
                memoized_hash: Mutex::new(Some(hash)).into(),
                ..Fingerprint::new()
            },
        })
    }
}

/// This is taken from the Cargo source code at
/// `cargo::core::compiler::fingerprint`[^1].
///
/// [^1]: https://github.com/rust-lang/cargo/blob/df07b394850b07348c918703054712e3427715cf/src/cargo/core/compiler/fingerprint/mod.rs#L756
#[derive(Debug, Serialize, Deserialize, Hash)]
enum LocalFingerprint {
    /// This is a precalculated fingerprint which has an opaque string we just
    /// hash as usual. This variant is primarily used for rustdoc where we
    /// don't have a dep-info file to compare against.
    ///
    /// This is also used for build scripts with no `rerun-if-*` statements, but
    /// that's overall a mistake and causes bugs in Cargo. We shouldn't use this
    /// for build scripts.
    Precalculated(String),

    /// This is used for crate compilations. The `dep_info` file is a relative
    /// path anchored at `target_root(...)` to the dep-info file that Cargo
    /// generates (which is a custom serialization after parsing rustc's own
    /// `dep-info` output).
    ///
    /// The `dep_info` file, when present, also lists a number of other files
    /// for us to look at. If any of those files are newer than this file then
    /// we need to recompile.
    ///
    /// If the `checksum` bool is true then the `dep_info` file is expected to
    /// contain file checksums instead of file mtimes.
    CheckDepInfo { dep_info: PathBuf, checksum: bool },

    /// This represents a nonempty set of `rerun-if-changed` annotations printed
    /// out by a build script. The `output` file is a relative file anchored at
    /// `target_root(...)` which is the actual output of the build script. That
    /// output has already been parsed and the paths printed out via
    /// `rerun-if-changed` are listed in `paths`. The `paths` field is relative
    /// to `pkg.root()`
    ///
    /// This is considered up-to-date if all of the `paths` are older than
    /// `output`, otherwise we need to recompile.
    RerunIfChanged {
        output: PathBuf,
        paths: Vec<PathBuf>,
    },

    /// This represents a single `rerun-if-env-changed` annotation printed by a
    /// build script. The exact env var and value are hashed here. There's no
    /// filesystem dependence here, and if the values are changed the hash will
    /// change forcing a recompile.
    RerunIfEnvChanged { var: String, val: Option<String> },
}
