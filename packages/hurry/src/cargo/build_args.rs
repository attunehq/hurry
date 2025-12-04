use color_eyre::{
    Result,
    eyre::{Context, ContextCompat},
};
use itertools::PeekingNext;
use parse_display::{Display as ParseDisplay, FromStr as ParseFromStr};
use tracing::trace;

use crate::cargo::{RustcTarget, RustcTargetPlatform};

/// Parsed arguments for a `cargo build` invocation.
///
/// ## Parsing
///
/// This type parses a list of strings from command-line arguments. Each string
/// is assumed to be a distinct argument to a `cargo build` invocation.
///
/// Handles both space-separated (`--flag value`) and equals-separated
/// (`--flag=value`) flag formats. For flags without values, parses the flag
/// standalone (e.g. `--flag`).
///
/// ## Usage
///
/// ```
/// # use hurry::cargo::CargoBuildArguments;
/// let args = vec!["--release", "--package", "foo", "--features", "feat1,feat2"];
/// let parsed = CargoBuildArguments::from_iter(args);
/// assert!(parsed.is_release());
/// assert_eq!(parsed.packages(), vec!["foo"]);
/// assert_eq!(parsed.features(), vec!["feat1", "feat2"]);
/// ```
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct CargoBuildArguments(Vec<CargoBuildArgument>);

impl CargoBuildArguments {
    /// Create an empty set of arguments.
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Convert to argv format for passing to `cargo build`.
    pub fn to_argv(&self) -> Vec<String> {
        self.0.iter().flat_map(|arg| arg.to_argv()).collect()
    }

    /// The profile specified by the user.
    pub fn profile(&self) -> Option<&str> {
        self.0.iter().find_map(|arg| match arg {
            CargoBuildArgument::Profile(p) => Some(p.as_str()),
            CargoBuildArgument::Release => Some("release"),
            _ => None,
        })
    }

    /// Whether release mode is enabled.
    pub fn is_release(&self) -> bool {
        self.0
            .iter()
            .any(|arg| matches!(arg, CargoBuildArgument::Release))
    }

    /// All package names specified.
    pub fn packages(&self) -> Vec<&str> {
        self.0
            .iter()
            .filter_map(|arg| match arg {
                CargoBuildArgument::Package(p) => Some(p.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The target triple if specified.
    pub fn target(&self) -> RustcTarget {
        self.0
            .iter()
            .find_map(|arg| match arg {
                CargoBuildArgument::Target(Some(t)) => Some(RustcTarget::Specified(
                    RustcTargetPlatform::try_from_str(t)
                        .unwrap_or(RustcTargetPlatform::Unsupported(t.clone())),
                )),
                _ => None,
            })
            .unwrap_or(RustcTarget::ImplicitHost)
    }

    /// The target directory if specified.
    pub fn target_dir(&self) -> Option<&str> {
        self.0.iter().find_map(|arg| match arg {
            CargoBuildArgument::TargetDir(d) => Some(d.as_str()),
            _ => None,
        })
    }

    /// The manifest path if specified.
    pub fn manifest_path(&self) -> Option<&str> {
        self.0.iter().find_map(|arg| match arg {
            CargoBuildArgument::ManifestPath(p) => Some(p.as_str()),
            _ => None,
        })
    }

    /// All features explicitly specified.
    ///
    /// This does not change in the presence of the "all features" flag; use
    /// [`CargoBuildArguments::all_features`] to check for that.
    ///
    /// This also does not include default features: it is strictly features
    /// that have been explicitly specified by the user.
    pub fn features(&self) -> Vec<&str> {
        self.0
            .iter()
            .flat_map(|arg| match arg {
                CargoBuildArgument::Features(features) => {
                    features.iter().map(|s| s.as_str()).collect()
                }
                _ => vec![],
            })
            .collect()
    }

    /// Whether "all features" are enabled.
    pub fn all_features(&self) -> bool {
        self.0
            .iter()
            .any(|arg| matches!(arg, CargoBuildArgument::AllFeatures))
    }

    /// Whether default features are disabled.
    pub fn no_default_features(&self) -> bool {
        self.0
            .iter()
            .any(|arg| matches!(arg, CargoBuildArgument::NoDefaultFeatures))
    }
}

impl IntoIterator for CargoBuildArguments {
    type Item = CargoBuildArgument;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// Parse from an iterator of strings.
impl<I> FromIterator<I> for CargoBuildArguments
where
    I: AsRef<str>,
{
    fn from_iter<T: IntoIterator<Item = I>>(iter: T) -> Self {
        let mut raw = iter.into_iter().map(|s| s.as_ref().to_string()).peekable();
        let mut parsed = Vec::new();

        while let Some(arg) = raw.next() {
            // Handle verbose flag specially since -vv is a single argument
            if arg.starts_with("-v") {
                let count = arg.chars().filter(|c| *c == 'v').count();
                parsed.push(CargoBuildArgument::Verbose(count as u8));
                continue;
            }

            // Apply aliases to normalize short flags to long form
            let arg = CargoBuildArgument::alias(&arg);

            if !CargoBuildArgument::is_flag(arg) {
                parsed.push(CargoBuildArgument::Positional(arg.to_string()));
                continue;
            }

            if !CargoBuildArgument::flag_accepts_value(arg) {
                parsed.push(CargoBuildArgument::parse(arg, None));
                continue;
            }

            if let Some((flag, value)) = CargoBuildArgument::split_equals(arg) {
                let flag = CargoBuildArgument::alias(flag);
                parsed.push(CargoBuildArgument::parse(flag, Some(value)));
                continue;
            }

            match raw.peeking_next(|upcoming| !CargoBuildArgument::is_flag(upcoming)) {
                Some(upcoming) => {
                    parsed.push(CargoBuildArgument::parse(arg, Some(&upcoming)));
                }
                None => {
                    parsed.push(CargoBuildArgument::parse(arg, None));
                }
            }
        }

        Self(parsed)
    }
}

impl AsRef<CargoBuildArguments> for CargoBuildArguments {
    fn as_ref(&self) -> &CargoBuildArguments {
        self
    }
}

impl From<&CargoBuildArguments> for CargoBuildArguments {
    fn from(args: &CargoBuildArguments) -> Self {
        args.clone()
    }
}

/// A parsed argument for a `cargo build` invocation.
///
/// ## Aliases
///
/// Flags that have short aliases always parse to a single canonical enum
/// variant and render using the canonical long form. For example, `-p foo` is
/// equivalent to `--package foo`, so both parse to
/// `Package(String::from("foo"))` and render as `--package foo`.
///
/// The following aliases are supported:
/// - `-v` → `--verbose`
/// - `-q` → `--quiet`
/// - `-p` → `--package`
/// - `-F` → `--features`
/// - `-r` → `--release`
/// - `-j` → `--jobs`
///
/// This ensures consistent cache keys regardless of whether the user uses short
/// or long flag forms.
///
/// ## Generic Flags
///
/// Flags that are not explicitly recognized are parsed as either `GenericFlag`
/// (for flags without values) or `GenericValueFlag` (for flags with values).
/// This provides forward compatibility with future cargo flags and allows
/// handling of custom flags without requiring changes to this parser.
///
/// ## Rendering
///
/// For flags with values, renders to space-separated format by default (`--flag
/// value`). Some flags support equals-separated format and will use that when
/// appropriate.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum CargoBuildArgument {
    /// `-v` or `--verbose` with count (e.g., -vv = 2)
    Verbose(u8),

    /// `-q` or `--quiet`
    Quiet,

    /// `--color <when>`
    Color(ColorWhen),

    /// `--config <key=value>`
    Config(String, String),

    /// `-Z <flag>`
    ///
    /// Note: We don't parse the flag value further (e.g., `-Z build-std=core`)
    /// since these are unstable; if we end up needing to parse them we can do
    /// so in a helper function or add additional typing later.
    UnstableFlag(String),

    /// `--frozen`
    Frozen,

    /// `--locked`
    Locked,

    /// `--offline`
    Offline,

    /// `-p` or `--package <spec>`
    Package(String),

    /// `--workspace`
    Workspace,

    /// `--exclude <spec>`
    Exclude(String),

    /// `--all` (deprecated alias for --workspace)
    All,

    /// `--lib`
    Lib,

    /// `--bins`
    Bins,

    /// `--bin [<name>]`
    Bin(Option<String>),

    /// `--examples`
    Examples,

    /// `--example [<name>]`
    Example(Option<String>),

    /// `--tests`
    Tests,

    /// `--test [<name>]`
    Test(Option<String>),

    /// `--benches`
    Benches,

    /// `--bench [<name>]`
    Bench(Option<String>),

    /// `--all-targets`
    AllTargets,

    /// `-F` or `--features <features>` (space or comma separated)
    Features(Vec<String>),

    /// `--all-features`
    AllFeatures,

    /// `--no-default-features`
    NoDefaultFeatures,

    /// `-r` or `--release`
    Release,

    /// `--profile <name>`
    Profile(String),

    /// `-j` or `--jobs [<n>]`
    Jobs(Option<u32>),

    /// `--keep-going`
    KeepGoing,

    /// `--target [<triple>]`
    Target(Option<String>),

    /// `--target-dir <directory>`
    TargetDir(String),

    /// `--artifact-dir <path>` (unstable)
    ArtifactDir(String),

    /// `--build-plan` (unstable)
    BuildPlan,

    /// `--unit-graph` (unstable)
    UnitGraph,

    /// `--timings[=<fmts>]` (unstable)
    Timings(Vec<String>),

    /// `--manifest-path <path>`
    ManifestPath(String),

    /// `--lockfile-path <path>` (unstable)
    LockfilePath(String),

    /// `--ignore-rust-version`
    IgnoreRustVersion,

    /// `--future-incompat-report`
    FutureIncompatReport,

    /// `--message-format <fmt>`
    MessageFormat(MessageFormat),

    /// Positional argument without a flag
    Positional(String),

    /// Any other flag without a value that isn't explicitly handled.
    GenericFlag(String),

    /// Any other flag with a value that isn't explicitly handled.
    GenericValueFlag(String, String),
}

impl CargoBuildArgument {
    const VERBOSE: &'static str = "--verbose";
    const QUIET: &'static str = "--quiet";
    const COLOR: &'static str = "--color";
    const CONFIG: &'static str = "--config";
    const UNSTABLE: &'static str = "-Z";
    const FROZEN: &'static str = "--frozen";
    const LOCKED: &'static str = "--locked";
    const OFFLINE: &'static str = "--offline";
    const PACKAGE: &'static str = "--package";
    const WORKSPACE: &'static str = "--workspace";
    const EXCLUDE: &'static str = "--exclude";
    const ALL: &'static str = "--all";
    const LIB: &'static str = "--lib";
    const BINS: &'static str = "--bins";
    const BIN: &'static str = "--bin";
    const EXAMPLES: &'static str = "--examples";
    const EXAMPLE: &'static str = "--example";
    const TESTS: &'static str = "--tests";
    const TEST: &'static str = "--test";
    const BENCHES: &'static str = "--benches";
    const BENCH: &'static str = "--bench";
    const ALL_TARGETS: &'static str = "--all-targets";
    const FEATURES: &'static str = "--features";
    const ALL_FEATURES: &'static str = "--all-features";
    const NO_DEFAULT_FEATURES: &'static str = "--no-default-features";
    const RELEASE: &'static str = "--release";
    const PROFILE: &'static str = "--profile";
    const JOBS: &'static str = "--jobs";
    const KEEP_GOING: &'static str = "--keep-going";
    const TARGET: &'static str = "--target";
    const TARGET_DIR: &'static str = "--target-dir";
    const ARTIFACT_DIR: &'static str = "--artifact-dir";
    const BUILD_PLAN: &'static str = "--build-plan";
    const UNIT_GRAPH: &'static str = "--unit-graph";
    const TIMINGS: &'static str = "--timings";
    const MANIFEST_PATH: &'static str = "--manifest-path";
    const LOCKFILE_PATH: &'static str = "--lockfile-path";
    const IGNORE_RUST_VERSION: &'static str = "--ignore-rust-version";
    const FUTURE_INCOMPAT_REPORT: &'static str = "--future-incompat-report";
    const MESSAGE_FORMAT: &'static str = "--message-format";

    fn is_flag(s: &str) -> bool {
        s.starts_with('-')
    }

    /// Map short flags to their canonical long form.
    fn alias(s: &str) -> &str {
        match s {
            "-v" => Self::VERBOSE,
            "-q" => Self::QUIET,
            "-p" => Self::PACKAGE,
            "-F" => Self::FEATURES,
            "-r" => Self::RELEASE,
            "-j" => Self::JOBS,
            _ => s,
        }
    }

    fn flag_accepts_value(flag: &str) -> bool {
        !matches!(
            flag,
            Self::VERBOSE
                | Self::QUIET
                | Self::FROZEN
                | Self::LOCKED
                | Self::OFFLINE
                | Self::WORKSPACE
                | Self::ALL
                | Self::LIB
                | Self::BINS
                | Self::EXAMPLES
                | Self::TESTS
                | Self::BENCHES
                | Self::ALL_TARGETS
                | Self::ALL_FEATURES
                | Self::NO_DEFAULT_FEATURES
                | Self::RELEASE
                | Self::KEEP_GOING
                | Self::BUILD_PLAN
                | Self::UNIT_GRAPH
                | Self::IGNORE_RUST_VERSION
                | Self::FUTURE_INCOMPAT_REPORT
        ) && Self::is_flag(flag)
    }

    fn split_equals(s: &str) -> Option<(&str, &str)> {
        s.split_once('=')
    }

    fn parse(flag: &str, value: Option<&str>) -> Self {
        let Some(value) = value else {
            return match flag {
                // Flags that don't accept values
                Self::VERBOSE => Self::Verbose(1),
                Self::QUIET => Self::Quiet,
                Self::FROZEN => Self::Frozen,
                Self::LOCKED => Self::Locked,
                Self::OFFLINE => Self::Offline,
                Self::WORKSPACE => Self::Workspace,
                Self::ALL => Self::All,
                Self::LIB => Self::Lib,
                Self::BINS => Self::Bins,
                Self::EXAMPLES => Self::Examples,
                Self::TESTS => Self::Tests,
                Self::BENCHES => Self::Benches,
                Self::ALL_TARGETS => Self::AllTargets,
                Self::ALL_FEATURES => Self::AllFeatures,
                Self::NO_DEFAULT_FEATURES => Self::NoDefaultFeatures,
                Self::RELEASE => Self::Release,
                Self::KEEP_GOING => Self::KeepGoing,
                Self::BUILD_PLAN => Self::BuildPlan,
                Self::UNIT_GRAPH => Self::UnitGraph,
                Self::IGNORE_RUST_VERSION => Self::IgnoreRustVersion,
                Self::FUTURE_INCOMPAT_REPORT => Self::FutureIncompatReport,
                // Flags that have optional values
                Self::BIN => Self::Bin(None),
                Self::EXAMPLE => Self::Example(None),
                Self::TEST => Self::Test(None),
                Self::BENCH => Self::Bench(None),
                Self::JOBS => Self::Jobs(None),
                Self::TARGET => Self::Target(None),
                Self::TIMINGS => Self::Timings(Vec::new()),
                _ if Self::is_flag(flag) => Self::GenericFlag(flag.to_string()),
                _ => Self::Positional(flag.to_string()),
            };
        };

        // Flags with values
        match Self::parse_inner(flag, value) {
            Ok(parsed) => parsed,
            Err(err) => {
                trace!(?flag, ?value, ?err, "failed to parse cargo build argument");
                Self::GenericValueFlag(flag.to_string(), value.to_string())
            }
        }
    }

    fn parse_inner(flag: &str, value: &str) -> Result<Self> {
        match flag {
            Self::COLOR => Ok(Self::Color(value.parse()?)),
            Self::CONFIG => {
                let (key, val) = value
                    .split_once('=')
                    .context("config value must be in key=value format")?;
                Ok(Self::Config(key.to_string(), val.to_string()))
            }
            Self::UNSTABLE => Ok(Self::UnstableFlag(value.to_string())),
            Self::PACKAGE => Ok(Self::Package(value.to_string())),
            Self::EXCLUDE => Ok(Self::Exclude(value.to_string())),
            Self::BIN => Ok(Self::Bin(Some(value.to_string()))),
            Self::EXAMPLE => Ok(Self::Example(Some(value.to_string()))),
            Self::TEST => Ok(Self::Test(Some(value.to_string()))),
            Self::BENCH => Ok(Self::Bench(Some(value.to_string()))),
            Self::FEATURES => {
                let features = value
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                Ok(Self::Features(features))
            }
            Self::PROFILE => Ok(Self::Profile(value.to_string())),
            Self::JOBS => Ok(Self::Jobs(Some(value.parse().context("parse jobs")?))),
            Self::TARGET => Ok(Self::Target(Some(value.to_string()))),
            Self::TARGET_DIR => Ok(Self::TargetDir(value.to_string())),
            Self::ARTIFACT_DIR => Ok(Self::ArtifactDir(value.to_string())),
            Self::TIMINGS => {
                let formats = value
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                Ok(Self::Timings(formats))
            }
            Self::MANIFEST_PATH => Ok(Self::ManifestPath(value.to_string())),
            Self::LOCKFILE_PATH => Ok(Self::LockfilePath(value.to_string())),
            Self::MESSAGE_FORMAT => Ok(Self::MessageFormat(value.parse()?)),
            _ => Ok(Self::GenericValueFlag(flag.to_string(), value.to_string())),
        }
    }

    fn to_argv(&self) -> Vec<String> {
        match self {
            Self::Verbose(count) => {
                if *count == 0 {
                    vec![]
                } else {
                    vec![format!("-{}", "v".repeat(*count as usize))]
                }
            }
            Self::Quiet => vec![Self::QUIET.to_string()],
            Self::Color(when) => vec![Self::COLOR.to_string(), when.to_string()],
            Self::Config(key, value) => {
                vec![Self::CONFIG.to_string(), format!("{key}={value}")]
            }
            Self::UnstableFlag(flag) => vec![Self::UNSTABLE.to_string(), flag.clone()],
            Self::Frozen => vec![Self::FROZEN.to_string()],
            Self::Locked => vec![Self::LOCKED.to_string()],
            Self::Offline => vec![Self::OFFLINE.to_string()],
            Self::Package(pkg) => vec![Self::PACKAGE.to_string(), pkg.clone()],
            Self::Workspace => vec![Self::WORKSPACE.to_string()],
            Self::Exclude(spec) => vec![Self::EXCLUDE.to_string(), spec.clone()],
            Self::All => vec![Self::ALL.to_string()],
            Self::Lib => vec![Self::LIB.to_string()],
            Self::Bins => vec![Self::BINS.to_string()],
            Self::Bin(None) => vec![Self::BIN.to_string()],
            Self::Bin(Some(name)) => vec![Self::BIN.to_string(), name.clone()],
            Self::Examples => vec![Self::EXAMPLES.to_string()],
            Self::Example(None) => vec![Self::EXAMPLE.to_string()],
            Self::Example(Some(name)) => vec![Self::EXAMPLE.to_string(), name.clone()],
            Self::Tests => vec![Self::TESTS.to_string()],
            Self::Test(None) => vec![Self::TEST.to_string()],
            Self::Test(Some(name)) => vec![Self::TEST.to_string(), name.clone()],
            Self::Benches => vec![Self::BENCHES.to_string()],
            Self::Bench(None) => vec![Self::BENCH.to_string()],
            Self::Bench(Some(name)) => vec![Self::BENCH.to_string(), name.clone()],
            Self::AllTargets => vec![Self::ALL_TARGETS.to_string()],
            Self::Features(features) => {
                vec![Self::FEATURES.to_string(), features.join(",")]
            }
            Self::AllFeatures => vec![Self::ALL_FEATURES.to_string()],
            Self::NoDefaultFeatures => vec![Self::NO_DEFAULT_FEATURES.to_string()],
            Self::Release => vec![Self::RELEASE.to_string()],
            Self::Profile(profile) => vec![Self::PROFILE.to_string(), profile.clone()],
            Self::Jobs(None) => vec![Self::JOBS.to_string()],
            Self::Jobs(Some(n)) => vec![Self::JOBS.to_string(), n.to_string()],
            Self::KeepGoing => vec![Self::KEEP_GOING.to_string()],
            Self::Target(None) => vec![Self::TARGET.to_string()],
            Self::Target(Some(triple)) => vec![Self::TARGET.to_string(), triple.clone()],
            Self::TargetDir(dir) => vec![Self::TARGET_DIR.to_string(), dir.clone()],
            Self::ArtifactDir(dir) => vec![Self::ARTIFACT_DIR.to_string(), dir.clone()],
            Self::BuildPlan => vec![Self::BUILD_PLAN.to_string()],
            Self::UnitGraph => vec![Self::UNIT_GRAPH.to_string()],
            Self::Timings(formats) => {
                if formats.is_empty() {
                    vec![Self::TIMINGS.to_string()]
                } else {
                    vec![Self::TIMINGS.to_string(), formats.join(",")]
                }
            }
            Self::ManifestPath(path) => vec![Self::MANIFEST_PATH.to_string(), path.clone()],
            Self::LockfilePath(path) => vec![Self::LOCKFILE_PATH.to_string(), path.clone()],
            Self::IgnoreRustVersion => vec![Self::IGNORE_RUST_VERSION.to_string()],
            Self::FutureIncompatReport => vec![Self::FUTURE_INCOMPAT_REPORT.to_string()],
            Self::MessageFormat(fmt) => vec![Self::MESSAGE_FORMAT.to_string(), fmt.to_string()],
            Self::Positional(arg) => vec![arg.clone()],
            Self::GenericFlag(flag) => vec![flag.clone()],
            Self::GenericValueFlag(flag, value) => vec![flag.clone(), value.clone()],
        }
    }
}

/// Color output setting for `--color`.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
#[display(style = "kebab-case")]
pub enum ColorWhen {
    Auto,
    Always,
    Never,

    #[display("{0}")]
    Other(String),
}

/// Message format for `--message-format`.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
#[display(style = "kebab-case")]
pub enum MessageFormat {
    Human,
    Short,
    Json,
    JsonDiagnosticShort,
    JsonDiagnosticRenderedAnsi,
    JsonRenderDiagnostics,

    #[display("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use simple_test_case::test_case;

    use super::*;

    #[test_case("--release"; "long")]
    #[test_case("-r"; "short")]
    #[test]
    fn parses_release_flag(flag: &str) {
        let args = CargoBuildArguments::from_iter(vec![flag]);
        assert!(args.is_release());
        pretty_assert_eq!(args.profile(), Some("release"));
    }

    #[test_case(&["--profile", "custom"]; "space_separated")]
    #[test_case(&["--profile=custom"]; "equals_separated")]
    #[test]
    fn parses_profile_flag(args: &[&str]) {
        let parsed = CargoBuildArguments::from_iter(args.to_vec());
        pretty_assert_eq!(parsed.profile(), Some("custom"));
        assert!(!parsed.is_release());
    }

    #[test_case(&["-p", "foo"], vec!["foo"]; "short_space")]
    #[test_case(&["--package", "bar"], vec!["bar"]; "long_space")]
    #[test_case(&["-p=bam"], vec!["bam"]; "short_equals")]
    #[test_case(&["--package=bap"], vec!["bap"]; "long_equals")]
    #[test_case(&["-p", "foo", "--package", "bar", "-p=bam", "--package=bap"], vec!["foo", "bar", "bam", "bap"]; "multiple")]
    #[test]
    fn parses_package_flags(args: &[&str], expected: Vec<&str>) {
        let parsed = CargoBuildArguments::from_iter(args.to_vec());
        pretty_assert_eq!(parsed.packages(), expected);
    }

    #[test_case(&["--features", "foo,bar"], vec!["foo", "bar"]; "long_space_comma")]
    #[test_case(&["--features=baz,qux"], vec!["baz", "qux"]; "long_equals_comma")]
    #[test_case(&["-F", "alpha,beta"], vec!["alpha", "beta"]; "short_space_comma")]
    #[test_case(&["-F=gamma,delta"], vec!["gamma", "delta"]; "short_equals_comma")]
    #[test_case(&["--features", "one two"], vec!["one", "two"]; "long_space_whitespace")]
    #[test_case(
        &["--features", "a,b", "--features=c,d", "-F", "e,f", "-F=g,h", "--features", "i j"],
        vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        "multiple"
    )]
    #[test]
    fn parses_features(args: &[&str], expected: Vec<&str>) {
        let parsed = CargoBuildArguments::from_iter(args.to_vec());
        pretty_assert_eq!(parsed.features(), expected);
    }

    #[test_case("-v", 1; "v")]
    #[test_case("-vv", 2; "vv")]
    #[test_case("-vvv", 3; "vvv")]
    #[test_case("--verbose", 1; "long")]
    #[test]
    fn parses_verbose_levels(flag: &str, expected_count: u8) {
        let args = CargoBuildArguments::from_iter(vec![flag]);
        let expected = vec![CargoBuildArgument::Verbose(expected_count)];
        pretty_assert_eq!(args.0, expected);
    }

    #[test_case(&["--target", "x86_64-unknown-linux-gnu"]; "space_separated")]
    #[test_case(&["--target=x86_64-unknown-linux-gnu"]; "equals_separated")]
    #[test]
    fn parses_target(args: &[&str]) {
        let parsed = CargoBuildArguments::from_iter(args.to_vec());
        pretty_assert_eq!(
            parsed.target(),
            RustcTarget::Specified(RustcTargetPlatform::X86_64LinuxGNU)
        );
    }

    #[test_case(&["--target-dir", "/custom/target"]; "space_separated")]
    #[test_case(&["--target-dir=/custom/target"]; "equals_separated")]
    #[test]
    fn parses_target_dir(args: &[&str]) {
        let parsed = CargoBuildArguments::from_iter(args.to_vec());
        pretty_assert_eq!(parsed.target_dir(), Some("/custom/target"));
    }

    #[test_case(&["--manifest-path", "/path/to/Cargo.toml"]; "space_separated")]
    #[test_case(&["--manifest-path=/path/to/Cargo.toml"]; "equals_separated")]
    #[test]
    fn parses_manifest_path(args: &[&str]) {
        let parsed = CargoBuildArguments::from_iter(args.to_vec());
        pretty_assert_eq!(parsed.manifest_path(), Some("/path/to/Cargo.toml"));
    }

    #[test_case(&["--jobs", "4"], 4; "long_space")]
    #[test_case(&["--jobs=8"], 8; "long_equals")]
    #[test_case(&["-j", "2"], 2; "short_space")]
    #[test_case(&["-j=16"], 16; "short_equals")]
    #[test]
    fn parses_jobs(args: &[&str], expected: u32) {
        let parsed = CargoBuildArguments::from_iter(args.to_vec());
        let jobs = parsed.0.iter().find_map(|arg| match arg {
            CargoBuildArgument::Jobs(Some(n)) => Some(*n),
            _ => None,
        });
        pretty_assert_eq!(jobs, Some(expected));
    }

    #[test_case("--quiet"; "long")]
    #[test_case("-q"; "short")]
    #[test]
    fn parses_quiet(flag: &str) {
        let args = CargoBuildArguments::from_iter(vec![flag]);
        let expected = vec![CargoBuildArgument::Quiet];
        pretty_assert_eq!(args.0, expected);
    }

    #[test]
    fn roundtrip_to_argv() {
        let original = vec!["--release", "-p", "foo", "--features", "feat1,feat2"];
        let args = CargoBuildArguments::from_iter(original.clone());
        let reconstructed = args.to_argv();

        // Should reconstruct to _equivalent_ form.
        //
        // We don't test against the _original_ input because normalization
        // occurs, but at the very least we should be able to roundtrip
        // `CargoBuildArguments` itself.
        let reparsed = CargoBuildArguments::from_iter(reconstructed);
        pretty_assert_eq!(args, reparsed);
    }

    #[test]
    fn short_flags_are_aliased() {
        let short = CargoBuildArguments::from_iter(vec!["-p", "foo", "-r", "-F", "feat1"]);
        let long = CargoBuildArguments::from_iter(vec![
            "--package",
            "foo",
            "--release",
            "--features",
            "feat1",
        ]);

        // Short and long forms should parse to identical structures
        pretty_assert_eq!(short, long);
    }

    #[test]
    fn parses_generic_flags() {
        let args = CargoBuildArguments::from_iter(vec![
            "--some-unknown-flag",
            "--another-flag",
            "value",
            "--another-flag=value2",
            "--release",
        ]);

        let expected = vec![
            CargoBuildArgument::GenericFlag(String::from("--some-unknown-flag")),
            CargoBuildArgument::GenericValueFlag(
                String::from("--another-flag"),
                String::from("value"),
            ),
            CargoBuildArgument::GenericValueFlag(
                String::from("--another-flag"),
                String::from("value2"),
            ),
            CargoBuildArgument::Release,
        ];
        pretty_assert_eq!(args.0, expected);
    }

    #[test]
    fn roundtrip_generic_flags() {
        let original = vec![
            "--unknown-flag",
            "--another-flag",
            "value",
            "--another-flag=value2",
            "--release",
        ];
        let args = CargoBuildArguments::from_iter(original.clone());
        let reconstructed = args.to_argv();

        // Should reconstruct to _equivalent_ form.
        //
        // We don't test against the _original_ input because normalization
        // occurs, but at the very least we should be able to roundtrip
        // `CargoBuildArguments` itself.
        let reparsed = CargoBuildArguments::from_iter(reconstructed);
        pretty_assert_eq!(args, reparsed);
    }

    #[test_case(&["--config", "build.jobs=4"], "build.jobs", "4"; "space_separated")]
    #[test_case(&["--config=build.jobs=4"], "build.jobs", "4"; "equals_separated")]
    #[test_case(&["--config", "net.git-fetch-with-cli=true"], "net.git-fetch-with-cli", "true"; "boolean_value")]
    #[test_case(&["--config", "target.x86_64-unknown-linux-gnu.linker=\"clang\""], "target.x86_64-unknown-linux-gnu.linker", "\"clang\""; "quoted_value")]
    #[test]
    fn parses_config(args: &[&str], expected_key: &str, expected_value: &str) {
        let parsed = CargoBuildArguments::from_iter(args.to_vec());
        let expected = vec![CargoBuildArgument::Config(
            String::from(expected_key),
            String::from(expected_value),
        )];
        pretty_assert_eq!(parsed.0, expected);
    }

    #[test_case(&["--config", "build.jobs=4", "--config=net.git-fetch-with-cli=true"]; "multiple_configs")]
    #[test]
    fn parses_multiple_config_flags(args: &[&str]) {
        let parsed = CargoBuildArguments::from_iter(args.to_vec());
        let expected = vec![
            CargoBuildArgument::Config(String::from("build.jobs"), String::from("4")),
            CargoBuildArgument::Config(
                String::from("net.git-fetch-with-cli"),
                String::from("true"),
            ),
        ];
        pretty_assert_eq!(parsed.0, expected);
    }

    #[test]
    fn roundtrip_config() {
        let original = vec!["--config", "build.jobs=4", "--release"];
        let args = CargoBuildArguments::from_iter(original.clone());
        let reconstructed = args.to_argv();

        let reparsed = CargoBuildArguments::from_iter(reconstructed);
        pretty_assert_eq!(args, reparsed);
    }

    #[test_case(&["--timings"], Vec::<&str>::new(); "no_formats")]
    #[test_case(&["--timings", "html"], vec!["html"]; "single_format_space")]
    #[test_case(&["--timings=html"], vec!["html"]; "single_format_equals")]
    #[test_case(&["--timings", "html,json"], vec!["html", "json"]; "multiple_formats_space")]
    #[test_case(&["--timings=html,json"], vec!["html", "json"]; "multiple_formats_equals")]
    #[test]
    fn parses_timings(args: &[&str], expected: Vec<&str>) {
        let parsed = CargoBuildArguments::from_iter(args.to_vec());
        let expected = vec![CargoBuildArgument::Timings(
            expected.iter().map(|s| String::from(*s)).collect(),
        )];
        pretty_assert_eq!(parsed.0, expected);
    }

    #[test]
    fn roundtrip_timings() {
        let original = vec!["--timings=html,json", "--release"];
        let args = CargoBuildArguments::from_iter(original.clone());
        let reconstructed = args.to_argv();

        let reparsed = CargoBuildArguments::from_iter(reconstructed);
        pretty_assert_eq!(args, reparsed);
    }
}
