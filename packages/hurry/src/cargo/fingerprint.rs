use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use color_eyre::Result;
use derive_more::Debug;
use rustc_stable_hash::StableSipHasher128;
use serde::{Deserialize, Serialize, de, ser};
use tracing::debug;

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

    pub fn fingerprint_hash(&self) -> String {
        hex::encode(self.hash_u64().to_le_bytes())
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
    pub fingerprint: Arc<Fingerprint>,
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
            fingerprint: Arc::new(Fingerprint {
                memoized_hash: Mutex::new(Some(hash)).into(),
                ..Fingerprint::new()
            }),
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
