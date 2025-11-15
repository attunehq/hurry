use std::{fmt::Debug, str::FromStr};

use color_eyre::{
    Report, Result, Section, SectionExt,
    eyre::{Context, eyre},
};
use derive_more::Display;
use enum_assoc::Assoc;
use itertools::PeekingNext;
use parse_display::{Display as ParseDisplay, FromStr as ParseFromStr};
use serde::{Deserialize, Deserializer};
use tracing::{instrument, trace};

use crate::{cargo::CargoBuildArguments, path::AbsDirPath};

/// Rust compiler metadata for cache key generation.
///
/// Contains platform-specific compiler information needed to generate cache
/// keys that are valid only for the current compilation target. This ensures
/// cached artifacts are not incorrectly shared between different platforms or
/// compiler configurations.
///
/// Currently only captures the LLVM target triple, but could be extended to
/// include compiler version, feature flags, or other compilation options that
/// affect output compatibility.
//
// TODO: Support users cross compiling; probably need to parse argv?
//
// TODO: Determine minimum compiler version.
//
// TODO: Is there a better way to get this?
//
// TODO: Add output from `rustc -vV`, which is what Cargo invokes? How does
// Cargo use this information?
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize)]
pub struct RustcMetadata {
    /// The host target triple.
    #[serde(rename = "llvm-target")]
    pub host_target: String,
}

impl RustcMetadata {
    /// Get platform metadata from the current compiler.
    #[instrument(name = "RustcMetadata::from_argv")]
    pub async fn from_argv(
        workspace_root: &AbsDirPath,
        args: impl AsRef<CargoBuildArguments> + Debug,
    ) -> Result<Self> {
        // TODO: Is this the correct `rustc` to use? Do we need to specially
        // handle interactions with `rustup` and `rust-toolchain.toml`?
        let mut cmd = tokio::process::Command::new("rustc");

        // Bypasses the check that disallows using unstable commands on stable.
        cmd.env("RUSTC_BOOTSTRAP", "1");

        // Forward the user's --target flag to rustc to get metadata for the
        // correct target, not just the host.
        cmd.args(["-Z", "unstable-options", "--print", "target-spec-json"]);
        if let Some(target) = args.as_ref().target() {
            cmd.args(["--target", target]);
        }
        cmd.current_dir(workspace_root.as_std_path());
        let output = cmd.output().await.context("run rustc")?;
        if !output.status.success() {
            return Err(eyre!("invoke rustc"))
                .with_section(|| {
                    String::from_utf8_lossy(&output.stdout)
                        .to_string()
                        .header("Stdout:")
                })
                .with_section(|| {
                    String::from_utf8_lossy(&output.stderr)
                        .to_string()
                        .header("Stderr:")
                });
        }

        serde_json::from_slice::<RustcMetadata>(&output.stdout)
            .context("parse rustc output")
            .with_section(|| {
                String::from_utf8_lossy(&output.stdout)
                    .to_string()
                    .header("Rustc Output:")
            })
    }
}

/// Ordered list of arguments for a `rustc` invocation.
///
/// ## Parsing
///
/// This type parses a list of strings as emitted by the Cargo build plan. Each
/// string is assumed to be a distinct argument to a `rustc` invocation; these
/// are referred to as "items" below.
///
/// This type handles parsing both space-separated (`--flag value`) and
/// equals-separated (`--flag=value`) flag formats. For flags without values,
/// parses the flag standalone (e.g. `--flag`).
///
/// Since we strive to handle arbitrary input this does rely on some heuristics.
/// For each new item:
/// - Whether the item is a flag is determined by checking if it starts with
///   either `--` or `-`; if it does then it is considered a flag.
/// - If the item is not a flag, it is parsed as a positional argument.
/// - If the flag is known to not accept a value, it is parsed as a positional
///   argument (for example, `--verbose`).
/// - If the flag has an equals-separated value, it is parsed as a flag and
///   value immediately (for example, `--flag=value`).
/// - Otherwise, the next item is checked to see if it is also a flag.
///   - If it is, the current flag is parsed as a positional argument and the
///     and the process starts over from the top for the next flag.
///   - If the next item is not a flag, the pair of items are parsed as a flag
///     and value (for example, `--flag value`).
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct RustcArguments(Vec<RustcArgument>);

impl RustcArguments {
    /// Iterate over the arguments in the invocation.
    pub fn iter(&self) -> impl Iterator<Item = &RustcArgument> {
        self.0.iter()
    }
    /// The crate name if specified.
    pub fn crate_name(&self) -> Option<&str> {
        self.0.iter().find_map(|arg| match arg {
            RustcArgument::CrateName(name) => Some(name.as_str()),
            _ => None,
        })
    }

    /// The path to the source file being compiled.
    pub fn src_path(&self) -> &str {
        let positional_arguments = self
            .0
            .iter()
            .filter_map(|arg| match arg {
                RustcArgument::Positional(p) => Some(p.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        debug_assert_eq!(
            positional_arguments.len(),
            1,
            "expected one rustc positional argument"
        );
        positional_arguments
            .first()
            .expect("rustc arguments should have one positional argument")
    }

    /// Find the `-C extra-filename` flag value if specified.
    pub fn extra_filename(&self) -> Option<&str> {
        self.0.iter().find_map(|arg| match arg {
            RustcArgument::Codegen(RustcCodegenOption::ExtraFilename(s)) => Some(s.as_str()),
            _ => None,
        })
    }
}

impl IntoIterator for RustcArguments {
    type Item = RustcArgument;
    type IntoIter = std::vec::IntoIter<Self::Item>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'de> Deserialize<'de> for RustcArguments {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut raw = Vec::<String>::deserialize(deserializer)?
            .into_iter()
            .peekable();
        let mut parsed = Vec::new();
        while let Some(arg) = raw.next() {
            let arg = RustcArgument::alias(&arg);
            if !RustcArgument::flag_accepts_value(arg) {
                parsed.push(RustcArgument::parse(arg, None));
                continue;
            }

            if let Some((flag, value)) = RustcArgument::split_equals(arg) {
                parsed.push(RustcArgument::parse(flag, Some(value)));
                continue;
            }

            match raw.peeking_next(|upcoming| !RustcArgument::is_flag(upcoming)) {
                Some(upcoming) => {
                    parsed.push(RustcArgument::parse(arg, Some(&upcoming)));
                }
                None => {
                    parsed.push(RustcArgument::parse(arg, None));
                }
            }
        }

        Ok(Self(parsed))
    }
}

/// A parsed argument for a `rustc` invocation.
///
/// ## Parsing
///
/// Since parsing arguments relies on state over multiple items in a collection
/// of arguments, this type does not handle parsing fully on its own (which is
/// why it doesn't implement `Deserialize`). Instead, use
/// `RustcInvocationArguments` to parse a collection of this type.
///
/// ## Rendering format
///
/// For flags with values, renders to equals-separated format (`--flag=value`)
/// by default. For flags that don't support equals-separated format, renders to
/// space-separated format (`--flag value`). For flags without values, renders
/// the flag standalone (e.g. `--flag`).
///
/// ## Aliases
///
/// Flags that have aliases always parse to a single canonical enum variant and
/// render using the same canonical variant. For example, `-g` is equivalent to
/// `-C debuginfo=2` so when we see `-g` we parse it as `-C debuginfo=2` and
/// similarly we render it as `-C debuginfo=2`.
///
/// We do this because for caching purposes these should be equivalent, and we
/// want to be able to write logic for them in a consistent way.
///
/// If we discover that a supposed alias actually surfaces different behavior
/// we'll untangle it as a standalone unique variant.
///
/// Be aware that these aliasing rules obviously cannot apply to the `Other`
/// variant, since that is treated as an opaque catch-all for arguments
/// unsupported by the current version of `hurry`.
///
/// ## Completeness
///
/// This type does not claim to complete all possible `rustc` invocation
/// arguments, if for no other reason than that there will be cases where they
/// update before we update this type.
///
/// However, it does try to parse everything we think we need to track for hurry
/// to work properly, or anything we think we might reasonably need in the
/// future.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum RustcArgument {
    /// `--cfg <spec>`
    Cfg(RustcCfgSpec),

    /// `--check-cfg <spec>`
    CheckCfg(RustcCheckCfgSpec),

    /// `-L [<kind>=]<path>`
    LibrarySearchPath(RustcLibrarySearchPath),

    /// `-l [<KIND>[:<MODIFIERS>]=]<NAME>[:<RENAME>]`
    Link(RustcLinkSpec),

    /// `--crate-name <name>`
    CrateName(String),

    /// `--crate-type <type>`
    CrateType(RustcCrateType),

    /// `--edition <edition>`
    Edition(RustcEdition),

    /// `--emit <type>[=<file>]`
    Emit(RustcEmitSpec),

    /// `--print <info>[=<file>]`
    Print(RustcPrintSpec),

    /// `-o <filename>`
    Output(String),

    /// `--out-dir <dir>`
    OutDir(String),

    /// `--explain <opt>`
    Explain(String),

    /// `--test`
    Test,

    /// `--target <target>`
    Target(String),

    /// `-A <lint>` or `--allow <lint>`
    Allow(String),

    /// `-W <lint>` or `--warn <lint>`
    Warn(String),

    /// `--force-warn <lint>`
    ForceWarn(String),

    /// `-D <lint>` or `--deny <lint>`
    Deny(String),

    /// `-F <lint>` or `--forbid <lint>`
    Forbid(String),

    /// `--cap-lints <level>`
    CapLints(RustcLintLevel),

    /// `-C <opt>[=<value>]` or `--codegen <opt>[=<value>]`
    Codegen(RustcCodegenOption),

    /// `--extern <name>=<path>`
    Extern(RustcExternSpec),

    /// `--error-format <format>`
    ErrorFormat(RustcErrorFormat),

    /// `--json <options>`
    Json(String),

    /// `-v` or `--verbose`
    Verbose,

    /// Positional argument without a flag
    Positional(String),

    /// Any generic argument and flag that isn't explicitly handled.
    Generic(String, String),
}

impl RustcArgument {
    const CFG: &'static str = "--cfg";
    const CHECK_CFG: &'static str = "--check-cfg";
    const CRATE_NAME: &'static str = "--crate-name";
    const CRATE_TYPE: &'static str = "--crate-type";
    const EDITION: &'static str = "--edition";
    const EMIT: &'static str = "--emit";
    const PRINT: &'static str = "--print";
    const OUTPUT: &'static str = "-o";
    const OUT_DIR: &'static str = "--out-dir";
    const EXPLAIN: &'static str = "--explain";
    const TEST: &'static str = "--test";
    const TARGET: &'static str = "--target";
    const ALLOW: &'static str = "--allow";
    const WARN: &'static str = "--warn";
    const FORCE_WARN: &'static str = "--force-warn";
    const DENY: &'static str = "--deny";
    const FORBID: &'static str = "--forbid";
    const CAP_LINTS: &'static str = "--cap-lints";
    const CODEGEN: &'static str = "--codegen";
    const EXTERN: &'static str = "--extern";
    const ERROR_FORMAT: &'static str = "--error-format";
    const JSON: &'static str = "--json";
    const LIBRARY_SEARCH: &'static str = "--library-search";
    const LINK: &'static str = "--link";
    const VERBOSE: &'static str = "--verbose";

    fn is_flag(flag: &str) -> bool {
        lazy_regex::regex_is_match!(r#"(?:^--|^-).+"#, flag)
    }

    fn flag_accepts_value(flag: &str) -> bool {
        Self::is_flag(flag) && !matches!(flag, Self::TEST | Self::VERBOSE)
    }

    fn split_equals(flag: &str) -> Option<(&str, &str)> {
        lazy_regex::regex_captures!(r#"^(?P<flag>[^=]+)=(?P<value>.+)$"#, flag)
            .map(|(_, flag, value)| (flag, value))
    }

    fn parse(flag: &str, value: Option<&str>) -> Self {
        let Some(value) = value else {
            return match flag {
                Self::TEST => Self::Test,
                Self::VERBOSE => Self::Verbose,
                _ => Self::Positional(flag.to_string()),
            };
        };

        match Self::parse_inner(flag, value) {
            Ok(parsed) => parsed,
            Err(err) => {
                trace!(
                    ?flag,
                    ?value,
                    ?err,
                    "failed to parse rustc invocation argument"
                );
                Self::Generic(flag.to_string(), value.to_string())
            }
        }
    }

    fn parse_inner(flag: &str, value: &str) -> Result<Self> {
        match flag {
            Self::CFG => Ok(Self::Cfg(value.parse()?)),
            Self::CHECK_CFG => Ok(Self::CheckCfg(value.parse()?)),
            Self::CRATE_NAME => Ok(Self::CrateName(value.to_string())),
            Self::CRATE_TYPE => Ok(Self::CrateType(value.parse()?)),
            Self::EDITION => Ok(Self::Edition(value.parse()?)),
            Self::EMIT => Ok(Self::Emit(value.parse()?)),
            Self::PRINT => Ok(Self::Print(value.parse()?)),
            Self::OUTPUT => Ok(Self::Output(value.to_string())),
            Self::OUT_DIR => Ok(Self::OutDir(value.to_string())),
            Self::EXPLAIN => Ok(Self::Explain(value.to_string())),
            Self::TARGET => Ok(Self::Target(value.to_string())),
            Self::ALLOW => Ok(Self::Allow(value.to_string())),
            Self::WARN => Ok(Self::Warn(value.to_string())),
            Self::FORCE_WARN => Ok(Self::ForceWarn(value.to_string())),
            Self::DENY => Ok(Self::Deny(value.to_string())),
            Self::FORBID => Ok(Self::Forbid(value.to_string())),
            Self::CAP_LINTS => Ok(Self::CapLints(value.parse()?)),
            Self::CODEGEN => Ok(Self::Codegen(value.parse()?)),
            Self::EXTERN => Ok(Self::Extern(value.parse()?)),
            Self::ERROR_FORMAT => Ok(Self::ErrorFormat(value.parse()?)),
            Self::JSON => Ok(Self::Json(value.to_string())),
            Self::LIBRARY_SEARCH => Ok(Self::LibrarySearchPath(value.parse()?)),
            Self::LINK => Ok(Self::Link(value.parse()?)),
            _ => Ok(Self::Generic(flag.to_string(), value.to_string())),
        }
    }

    fn alias(s: &str) -> &str {
        match s {
            "-A" => "--allow",
            "-W" => "--warn",
            "-D" => "--deny",
            "-F" => "--forbid",
            "-C" => "--codegen",
            "-L" => "--library-search",
            "-l" => "--link",
            "-v" => "--verbose",
            "-g" => "--codegen=debuginfo=2",
            "-O" => "--codegen=opt-level=3",
            _ => s,
        }
    }
}

/// Type of crate for the compiler to emit.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
pub enum RustcCrateType {
    #[display("bin")]
    Bin,

    #[display("lib")]
    Lib,

    #[display("rlib")]
    Rlib,

    #[display("dylib")]
    Dylib,

    #[display("cdylib")]
    Cdylib,

    #[display("staticlib")]
    Staticlib,

    #[display("proc-macro")]
    ProcMacro,

    /// Any other unrecognized variant.
    #[display("{0}")]
    Other(String),
}

/// Specify which edition of the compiler to use when
/// compiling code. The default is 2015 and the latest
/// stable edition is 2024.
#[derive(
    Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Default, ParseDisplay, ParseFromStr,
)]
pub enum RustcEdition {
    #[default]
    #[display("2015")]
    Edition2015,

    #[display("2018")]
    Edition2018,

    #[display("2021")]
    Edition2021,

    #[display("2024")]
    Edition2024,

    #[display("future")]
    EditionFuture,

    /// Any other unrecognized variant.
    #[display("{0}")]
    Other(String),
}

impl RustcEdition {
    /// The latest stable edition.
    pub const LATEST_STABLE: Self = Self::Edition2024;
}

/// Comma separated list of types of output for the compiler to emit.
///
/// Each TYPE has the default FILE name:
/// * asm - CRATE_NAME.s
/// * dep-info - CRATE_NAME.d
/// * link - (platform and crate-type dependent)
/// * llvm-bc - CRATE_NAME.bc
/// * llvm-ir - CRATE_NAME.ll
/// * metadata - libCRATE_NAME.rmeta
/// * mir - CRATE_NAME.mir
/// * obj - CRATE_NAME.o
/// * thin-link-bitcode - CRATE_NAME.indexing.o
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Assoc, ParseDisplay, ParseFromStr)]
#[func(pub fn default_file_name(&self, crate_name: &str) -> Option<String>)]
pub enum RustcEmitFormat {
    #[display("asm")]
    #[assoc(default_file_name = format!("{crate_name}.s"))]
    Asm,

    #[display("dep-info")]
    #[assoc(default_file_name = format!("{crate_name}.d"))]
    DepInfo,

    // Does not have a default file name as it is "platform and crate-type
    // dependent" according to `rustc` documentation.
    #[display("link")]
    Link,

    #[display("llvm-bc")]
    #[assoc(default_file_name = format!("{crate_name}.bc"))]
    LlvmBc,

    #[display("llvm-ir")]
    #[assoc(default_file_name = format!("{crate_name}.ll"))]
    LlvmIr,

    #[display("metadata")]
    #[assoc(default_file_name = format!("lib{crate_name}.rmeta"))]
    Metadata,

    #[display("mir")]
    #[assoc(default_file_name = format!("{crate_name}.mir"))]
    Mir,

    #[display("obj")]
    #[assoc(default_file_name = format!("{crate_name}.o"))]
    Obj,

    #[display("thin-link-bitcode")]
    #[assoc(default_file_name = format!("{crate_name}.indexing.o"))]
    ThinLinkBitcode,

    /// Any other unrecognized variant.
    #[display("{0}")]
    Other(String),
}

/// Expected config for checking the compilation environment.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
#[display("{0}")]
pub struct RustcCheckCfgSpec(String);

/// Configure the compilation environment.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
#[display("{0}={1}")]
pub struct RustcCfgSpec(RustcCfgSpecKey, RustcCfgSpecValue);

/// The key used to configure the compilation environment.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
pub enum RustcCfgSpecKey {
    #[display("feature")]
    Feature,

    #[display("{0}")]
    Other(String),
}

/// The value used to configure the compilation environment.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
#[display(r#""{0}""#)]
pub struct RustcCfgSpecValue(String);

/// A directory added to the library search path: `-L [<kind>=]<path>`
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display)]
#[display("{_0}={_1}")]
pub struct RustcLibrarySearchPath(RustcLibrarySearchPathKind, String);

impl FromStr for RustcLibrarySearchPath {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self> {
        match s.split_once('=') {
            Some((kind, path)) => {
                let kind = kind.parse()?;
                Ok(Self(kind, path.to_string()))
            }
            None => Ok(Self(RustcLibrarySearchPathKind::default(), s.to_string())),
        }
    }
}

/// Kind of library search path.
#[derive(
    Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default, Debug, ParseDisplay, ParseFromStr,
)]
pub enum RustcLibrarySearchPathKind {
    #[default]
    #[display("all")]
    All,

    #[display("crate")]
    Crate,

    #[display("dependency")]
    Dependency,

    #[display("framework")]
    Framework,

    #[display("native")]
    Native,

    #[display("{0}")]
    Other(String),
}

/// Kind of linking to perform for a native library.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
pub enum RustcLinkKind {
    #[display("dylib")]
    Dylib,

    #[display("framework")]
    Framework,

    #[display("static")]
    Static,

    #[display("{0}")]
    Other(String),
}

/// Modifier when linking a native library.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
pub enum RustcLinkModifier {
    #[display("{0}bundle")]
    Bundle(RustcLinkModifierState),

    #[display("{0}verbatim")]
    Verbatim(RustcLinkModifierState),

    #[display("{0}whole-archive")]
    WholeArchive(RustcLinkModifierState),

    #[display("{0}as-needed")]
    AsNeeded(RustcLinkModifierState),

    #[display("{0}{1}")]
    Other(RustcLinkModifierState, String),
}

/// Whether the link modifier is enabled, disabled, or unspecified.
#[derive(
    Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Default, ParseDisplay, ParseFromStr,
)]
pub enum RustcLinkModifierState {
    /// The link modifier is enabled.
    #[display("+")]
    Enabled,

    /// The link modifier is disabled.
    #[display("-")]
    Disabled,

    /// The link modifier is unspecified.
    #[default]
    #[display("")]
    Unspecified,
}

/// The target for printing compiler information.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
pub enum RustcPrintTarget {
    /// Print to a file.
    #[display("{0}")]
    File(String),

    /// Print to stdout.
    #[display("")]
    Stdout,
}

/// Compiler information to print on stdout (or to a file).
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
#[display(style = "kebab-case")]
pub enum RustcPrintInfo {
    AllTargetSpecsJson,
    CallingConventions,
    Cfg,
    CheckCfg,
    CodeModels,
    CrateName,
    CrateRootLintLevels,
    DeploymentTarget,
    FileNames,
    HostTuple,
    LinkArgs,
    NativeStaticLibs,
    RelocationModels,
    SplitDebuginfo,
    StackProtectorStrategies,
    SupportedCrateTypes,
    Sysroot,
    TargetCpus,
    TargetFeatures,
    TargetLibdir,
    TargetList,
    TargetSpecJson,
    TlsModels,

    /// Any other unrecognized variant.
    #[display("{0}")]
    Other(String),
}

/// Link spec: `-l [<KIND>[:<MODIFIERS>]=]<NAME>[:<RENAME>]`
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct RustcLinkSpec {
    pub kind: Option<RustcLinkKind>,
    pub modifiers: Vec<RustcLinkModifier>,
    pub name: String,
    pub rename: Option<String>,
}

impl FromStr for RustcLinkSpec {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self> {
        // Split on '=' to separate kind+modifiers from name+rename
        let (kind_mods, name_rename) = match s.split_once('=') {
            Some((left, right)) => (Some(left), right),
            None => (None, s),
        };

        // Parse kind and modifiers if present
        let (kind, modifiers) = if let Some(kind_mods) = kind_mods {
            match kind_mods.split_once(':') {
                Some((kind_str, mods_str)) => {
                    let kind = kind_str.parse()?;
                    let modifiers = mods_str
                        .split(',')
                        .map(|s| s.parse().context("parse modifier"))
                        .collect::<Result<Vec<_>>>()?;
                    (Some(kind), modifiers)
                }
                None => (Some(kind_mods.parse()?), Vec::new()),
            }
        } else {
            (None, Vec::new())
        };

        // Parse name and rename
        let (name, rename) = match name_rename.split_once(':') {
            Some((name, rename)) => (name.to_string(), Some(rename.to_string())),
            None => (name_rename.to_string(), None),
        };

        Ok(Self {
            kind,
            modifiers,
            name,
            rename,
        })
    }
}

impl std::fmt::Display for RustcLinkSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(kind) = &self.kind {
            write!(f, "{kind}")?;
            if !self.modifiers.is_empty() {
                write!(f, ":")?;
                for (i, modifier) in self.modifiers.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{modifier}")?;
                }
            }
            write!(f, "=")?;
        }
        write!(f, "{}", self.name)?;
        if let Some(rename) = &self.rename {
            write!(f, ":{rename}")?;
        }
        Ok(())
    }
}

/// Emit spec: `--emit <type>[=<file>]` or `--emit <type1>,<type2>,...`
///
/// Rustc supports emitting multiple output types with comma-separated formats.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct RustcEmitSpec {
    pub formats: Vec<RustcEmitFormat>,
    pub file: Option<String>,
}

impl FromStr for RustcEmitSpec {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self> {
        match s.split_once('=') {
            Some((formats, file)) => {
                let formats = formats
                    .split(',')
                    .map(|f| f.parse().context("parse emit format"))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Self {
                    formats,
                    file: Some(file.to_string()),
                })
            }
            None => {
                let formats = s
                    .split(',')
                    .map(|f| f.parse().context("parse emit format"))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Self {
                    formats,
                    file: None,
                })
            }
        }
    }
}

impl std::fmt::Display for RustcEmitSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, format) in self.formats.iter().enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }
            write!(f, "{format}")?;
        }
        if let Some(file) = &self.file {
            write!(f, "={file}")?;
        }
        Ok(())
    }
}

/// Print spec: `--print <info>[=<file>]`
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct RustcPrintSpec {
    pub info: RustcPrintInfo,
    pub target: RustcPrintTarget,
}

impl FromStr for RustcPrintSpec {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self> {
        match s.split_once('=') {
            Some((info, file)) => Ok(Self {
                info: info.parse()?,
                target: RustcPrintTarget::File(file.to_string()),
            }),
            None => Ok(Self {
                info: s.parse()?,
                target: RustcPrintTarget::Stdout,
            }),
        }
    }
}

impl std::fmt::Display for RustcPrintSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.info)?;
        match &self.target {
            RustcPrintTarget::File(file) => write!(f, "={file}"),
            RustcPrintTarget::Stdout => Ok(()),
        }
    }
}

/// Lint level for cap-lints.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
pub enum RustcLintLevel {
    #[display("allow")]
    Allow,

    #[display("warn")]
    Warn,

    #[display("deny")]
    Deny,

    #[display("forbid")]
    Forbid,

    #[display("{0}")]
    Other(String),
}

/// Codegen option: `-C <opt>[=<value>]`
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum RustcCodegenOption {
    /// `debuginfo=<level>`
    Debuginfo(String),

    /// `opt-level=<level>`
    OptLevel(String),

    /// `metadata=<value>`
    Metadata(String),

    /// `extra-filename=<value>`
    ExtraFilename(String),

    /// `split-debuginfo=<value>`
    SplitDebuginfo(RustcSplitDebuginfo),

    /// `embed-bitcode=<value>`
    EmbedBitcode(RustcEmbedBitcode),

    /// `prefer-dynamic`
    PreferDynamic,

    /// Any other codegen option
    Other(String, Option<String>),
}

impl FromStr for RustcCodegenOption {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self> {
        match s.split_once('=') {
            Some(("debuginfo", level)) => Ok(Self::Debuginfo(level.to_string())),
            Some(("opt-level", level)) => Ok(Self::OptLevel(level.to_string())),
            Some(("metadata", value)) => Ok(Self::Metadata(value.to_string())),
            Some(("extra-filename", value)) => Ok(Self::ExtraFilename(value.to_string())),
            Some(("split-debuginfo", value)) => Ok(Self::SplitDebuginfo(value.parse()?)),
            Some(("embed-bitcode", value)) => Ok(Self::EmbedBitcode(value.parse()?)),
            Some((key, value)) => Ok(Self::Other(key.to_string(), Some(value.to_string()))),
            None if s == "prefer-dynamic" => Ok(Self::PreferDynamic),
            None => Ok(Self::Other(s.to_string(), None)),
        }
    }
}

impl std::fmt::Display for RustcCodegenOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Debuginfo(level) => write!(f, "debuginfo={level}"),
            Self::OptLevel(level) => write!(f, "opt-level={level}"),
            Self::Metadata(value) => write!(f, "metadata={value}"),
            Self::ExtraFilename(value) => write!(f, "extra-filename={value}"),
            Self::SplitDebuginfo(value) => write!(f, "split-debuginfo={value}"),
            Self::EmbedBitcode(value) => write!(f, "embed-bitcode={value}"),
            Self::PreferDynamic => write!(f, "prefer-dynamic"),
            Self::Other(key, Some(value)) => write!(f, "{key}={value}"),
            Self::Other(key, None) => write!(f, "{key}"),
        }
    }
}

/// Split debuginfo mode.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
pub enum RustcSplitDebuginfo {
    #[display("off")]
    Off,

    #[display("packed")]
    Packed,

    #[display("unpacked")]
    Unpacked,

    #[display("{0}")]
    Other(String),
}

/// Embed bitcode mode.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
pub enum RustcEmbedBitcode {
    #[display("yes")]
    Yes,

    #[display("no")]
    No,

    #[display("{0}")]
    Other(String),
}

/// Extern crate spec: `--extern <name>=<path>` or `--extern <name>`
///
/// Some builtin crates like `proc_macro` don't require a path.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct RustcExternSpec {
    pub name: String,
    pub path: Option<String>,
}

impl FromStr for RustcExternSpec {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self> {
        match s.split_once('=') {
            Some((name, path)) => Ok(Self {
                name: name.to_string(),
                path: Some(path.to_string()),
            }),
            None => Ok(Self {
                name: s.to_string(),
                path: None,
            }),
        }
    }
}

impl std::fmt::Display for RustcExternSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)?;
        if let Some(path) = &self.path {
            write!(f, "={path}")?;
        }
        Ok(())
    }
}

/// Error format for compiler output.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, ParseDisplay, ParseFromStr)]
pub enum RustcErrorFormat {
    #[display("human")]
    Human,

    #[display("json")]
    Json,

    #[display("short")]
    Short,

    #[display("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use color_eyre::Result;
    use pretty_assertions::assert_eq as pretty_assert_eq;

    use super::*;

    #[test]
    fn parse_lib_build_args() -> Result<()> {
        let json = include_str!("rustc/fixtures/lib_build.json");
        let args = serde_json::from_str::<RustcArguments>(json).context("parse lib build args")?;

        let expected = vec![
            RustcArgument::CrateName(String::from("base64")),
            RustcArgument::Edition(RustcEdition::Edition2018),
            RustcArgument::Positional(String::from(
                "/Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/base64-0.22.1/src/lib.rs",
            )),
            RustcArgument::ErrorFormat(RustcErrorFormat::Json),
            RustcArgument::Json(String::from(
                "diagnostic-rendered-ansi,artifacts,future-incompat",
            )),
            RustcArgument::CrateType(RustcCrateType::Lib),
            RustcArgument::Emit(RustcEmitSpec {
                formats: vec![
                    RustcEmitFormat::DepInfo,
                    RustcEmitFormat::Metadata,
                    RustcEmitFormat::Link,
                ],
                file: None,
            }),
            RustcArgument::Codegen(RustcCodegenOption::EmbedBitcode(RustcEmbedBitcode::No)),
            RustcArgument::Codegen(RustcCodegenOption::Debuginfo(String::from("2"))),
            RustcArgument::Codegen(RustcCodegenOption::SplitDebuginfo(
                RustcSplitDebuginfo::Unpacked,
            )),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("alloc")),
            )),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("default")),
            )),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("std")),
            )),
            RustcArgument::CheckCfg(RustcCheckCfgSpec(String::from("cfg(docsrs,test)"))),
            RustcArgument::CheckCfg(RustcCheckCfgSpec(String::from(
                "cfg(feature, values(\"alloc\", \"default\", \"std\"))",
            ))),
            RustcArgument::Codegen(RustcCodegenOption::Metadata(String::from(
                "d33a4080aa6108f0",
            ))),
            RustcArgument::Codegen(RustcCodegenOption::ExtraFilename(String::from(
                "-ac0e04d584580346",
            ))),
            RustcArgument::OutDir(String::from("/Users/jess/projects/hurry/target/debug/deps")),
            RustcArgument::LibrarySearchPath(RustcLibrarySearchPath(
                RustcLibrarySearchPathKind::Dependency,
                String::from("/Users/jess/projects/hurry/target/debug/deps"),
            )),
            RustcArgument::CapLints(RustcLintLevel::Allow),
        ];

        pretty_assert_eq!(args.0, expected);

        Ok(())
    }

    #[test]
    fn parse_build_script_args() -> Result<()> {
        let json = include_str!("rustc/fixtures/build_script.json");
        let args =
            serde_json::from_str::<RustcArguments>(json).context("parse build script args")?;

        let expected = vec![
            RustcArgument::CrateName(String::from("build_script_build")),
            RustcArgument::Edition(RustcEdition::Edition2018),
            RustcArgument::Positional(String::from(
                "/Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde-1.0.219/build.rs",
            )),
            RustcArgument::ErrorFormat(RustcErrorFormat::Json),
            RustcArgument::Json(String::from(
                "diagnostic-rendered-ansi,artifacts,future-incompat",
            )),
            RustcArgument::CrateType(RustcCrateType::Bin),
            RustcArgument::Emit(RustcEmitSpec {
                formats: vec![RustcEmitFormat::DepInfo, RustcEmitFormat::Link],
                file: None,
            }),
            RustcArgument::Codegen(RustcCodegenOption::EmbedBitcode(RustcEmbedBitcode::No)),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("alloc")),
            )),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("default")),
            )),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("derive")),
            )),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("rc")),
            )),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("serde_derive")),
            )),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("std")),
            )),
            RustcArgument::CheckCfg(RustcCheckCfgSpec(String::from("cfg(docsrs,test)"))),
            RustcArgument::CheckCfg(RustcCheckCfgSpec(String::from(
                "cfg(feature, values(\"alloc\", \"default\", \"derive\", \"rc\", \"serde_derive\", \"std\", \"unstable\"))",
            ))),
            RustcArgument::Codegen(RustcCodegenOption::Metadata(String::from(
                "b1f9da5bac2a885e",
            ))),
            RustcArgument::Codegen(RustcCodegenOption::ExtraFilename(String::from(
                "-4403cda6320c8d3c",
            ))),
            RustcArgument::OutDir(String::from(
                "/Users/jess/projects/hurry/target/debug/build/serde-4403cda6320c8d3c",
            )),
            RustcArgument::LibrarySearchPath(RustcLibrarySearchPath(
                RustcLibrarySearchPathKind::Dependency,
                String::from("/Users/jess/projects/hurry/target/debug/deps"),
            )),
            RustcArgument::CapLints(RustcLintLevel::Allow),
        ];

        pretty_assert_eq!(args.0, expected);

        Ok(())
    }

    #[test]
    fn parse_bin_target_args() -> Result<()> {
        let json = include_str!("rustc/fixtures/bin_target.json");
        let args = serde_json::from_str::<RustcArguments>(json).context("parse bin target args")?;

        let expected = vec![
            RustcArgument::CrateName(String::from("hurry")),
            RustcArgument::Edition(RustcEdition::Edition2024),
            RustcArgument::Positional(String::from("packages/hurry/src/bin/hurry/main.rs")),
            RustcArgument::ErrorFormat(RustcErrorFormat::Json),
            RustcArgument::Json(String::from(
                "diagnostic-rendered-ansi,artifacts,future-incompat",
            )),
            RustcArgument::CrateType(RustcCrateType::Bin),
            RustcArgument::Emit(RustcEmitSpec {
                formats: vec![RustcEmitFormat::DepInfo, RustcEmitFormat::Link],
                file: None,
            }),
            RustcArgument::Codegen(RustcCodegenOption::EmbedBitcode(RustcEmbedBitcode::No)),
            RustcArgument::Codegen(RustcCodegenOption::Debuginfo(String::from("2"))),
            RustcArgument::Codegen(RustcCodegenOption::SplitDebuginfo(
                RustcSplitDebuginfo::Unpacked,
            )),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("default")),
            )),
            RustcArgument::CheckCfg(RustcCheckCfgSpec(String::from("cfg(docsrs,test)"))),
            RustcArgument::CheckCfg(RustcCheckCfgSpec(String::from(
                "cfg(feature, values(\"default\"))",
            ))),
            RustcArgument::Codegen(RustcCodegenOption::Metadata(String::from(
                "00796917b4ea58e8",
            ))),
            RustcArgument::Codegen(RustcCodegenOption::ExtraFilename(String::from(
                "-e1b02ccff50f16f0",
            ))),
            RustcArgument::OutDir(String::from("/Users/jess/projects/hurry/target/debug/deps")),
            RustcArgument::Codegen(RustcCodegenOption::Other(
                String::from("incremental"),
                Some(String::from(
                    "/Users/jess/projects/hurry/target/debug/incremental",
                )),
            )),
            RustcArgument::LibrarySearchPath(RustcLibrarySearchPath(
                RustcLibrarySearchPathKind::Dependency,
                String::from("/Users/jess/projects/hurry/target/debug/deps"),
            )),
            RustcArgument::Extern(RustcExternSpec {
                name: String::from("async_walkdir"),
                path: Some(String::from(
                    "/Users/jess/projects/hurry/target/debug/deps/libasync_walkdir-e2da76a81aef6c7d.rlib",
                )),
            }),
            RustcArgument::Extern(RustcExternSpec {
                name: String::from("tokio"),
                path: Some(String::from(
                    "/Users/jess/projects/hurry/target/debug/deps/libtokio-d6c67f45b37bedc2.rlib",
                )),
            }),
            RustcArgument::CapLints(RustcLintLevel::Allow),
        ];

        pretty_assert_eq!(args.0, expected);

        Ok(())
    }

    #[test]
    fn parse_proc_macro_args() -> Result<()> {
        let json = include_str!("rustc/fixtures/proc_macro.json");
        let args = serde_json::from_str::<RustcArguments>(json).context("parse proc-macro args")?;

        let expected = vec![
            RustcArgument::CrateName(String::from("serde_derive")),
            RustcArgument::Edition(RustcEdition::Edition2015),
            RustcArgument::Positional(String::from(
                "/Users/jess/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde_derive-1.0.219/src/lib.rs",
            )),
            RustcArgument::ErrorFormat(RustcErrorFormat::Json),
            RustcArgument::Json(String::from(
                "diagnostic-rendered-ansi,artifacts,future-incompat",
            )),
            RustcArgument::CrateType(RustcCrateType::ProcMacro),
            RustcArgument::Emit(RustcEmitSpec {
                formats: vec![RustcEmitFormat::DepInfo, RustcEmitFormat::Link],
                file: None,
            }),
            RustcArgument::Codegen(RustcCodegenOption::PreferDynamic),
            RustcArgument::Codegen(RustcCodegenOption::EmbedBitcode(RustcEmbedBitcode::No)),
            RustcArgument::Cfg(RustcCfgSpec(
                RustcCfgSpecKey::Feature,
                RustcCfgSpecValue(String::from("default")),
            )),
            RustcArgument::CheckCfg(RustcCheckCfgSpec(String::from("cfg(docsrs,test)"))),
            RustcArgument::CheckCfg(RustcCheckCfgSpec(String::from(
                "cfg(feature, values(\"default\", \"deserialize_in_place\"))",
            ))),
            RustcArgument::Codegen(RustcCodegenOption::Metadata(String::from(
                "7b47d0b71c75bce9",
            ))),
            RustcArgument::Codegen(RustcCodegenOption::ExtraFilename(String::from(
                "-1fa31a1e1f706456",
            ))),
            RustcArgument::OutDir(String::from("/Users/jess/projects/hurry/target/debug/deps")),
            RustcArgument::LibrarySearchPath(RustcLibrarySearchPath(
                RustcLibrarySearchPathKind::Dependency,
                String::from("/Users/jess/projects/hurry/target/debug/deps"),
            )),
            RustcArgument::Extern(RustcExternSpec {
                name: String::from("proc_macro2"),
                path: Some(String::from(
                    "/Users/jess/projects/hurry/target/debug/deps/libproc_macro2-997ccdc46b2e4671.rlib",
                )),
            }),
            RustcArgument::Extern(RustcExternSpec {
                name: String::from("quote"),
                path: Some(String::from(
                    "/Users/jess/projects/hurry/target/debug/deps/libquote-49b7c12bb0a8f649.rlib",
                )),
            }),
            RustcArgument::Extern(RustcExternSpec {
                name: String::from("syn"),
                path: Some(String::from(
                    "/Users/jess/projects/hurry/target/debug/deps/libsyn-ef32486969275305.rlib",
                )),
            }),
            RustcArgument::Extern(RustcExternSpec {
                name: String::from("proc_macro"),
                path: None,
            }),
            RustcArgument::CapLints(RustcLintLevel::Allow),
        ];

        pretty_assert_eq!(args.0, expected);

        Ok(())
    }

    #[test]
    fn parse_short_flag_aliases() -> Result<()> {
        let json = r#"["-A","unused","-W","dead-code","-D","warnings","-F","unsafe-code"]"#;
        let args =
            serde_json::from_str::<RustcArguments>(json).context("parse short flag aliases")?;

        let expected = vec![
            RustcArgument::Allow(String::from("unused")),
            RustcArgument::Warn(String::from("dead-code")),
            RustcArgument::Deny(String::from("warnings")),
            RustcArgument::Forbid(String::from("unsafe-code")),
        ];

        pretty_assert_eq!(args.0, expected);

        Ok(())
    }

    #[test]
    fn parse_codegen_aliases() -> Result<()> {
        let json = r#"["-C","opt-level=3","-g","-O"]"#;
        let args = serde_json::from_str::<RustcArguments>(json).context("parse codegen aliases")?;

        let expected = vec![
            RustcArgument::Codegen(RustcCodegenOption::OptLevel(String::from("3"))),
            RustcArgument::Codegen(RustcCodegenOption::Debuginfo(String::from("2"))),
            RustcArgument::Codegen(RustcCodegenOption::OptLevel(String::from("3"))),
        ];

        pretty_assert_eq!(args.0, expected);

        Ok(())
    }

    #[test]
    fn parse_library_search_alias() -> Result<()> {
        let json = r#"["-L","dependency=/path/to/deps"]"#;
        let args =
            serde_json::from_str::<RustcArguments>(json).context("parse library search alias")?;

        let expected = vec![RustcArgument::LibrarySearchPath(RustcLibrarySearchPath(
            RustcLibrarySearchPathKind::Dependency,
            String::from("/path/to/deps"),
        ))];

        pretty_assert_eq!(args.0, expected);

        Ok(())
    }

    #[test]
    fn parse_link_alias() -> Result<()> {
        let json = r#"["-l","static:+whole-archive=mylib"]"#;
        let args = serde_json::from_str::<RustcArguments>(json).context("parse link alias")?;

        let expected = vec![RustcArgument::Link(RustcLinkSpec {
            kind: Some(RustcLinkKind::Static),
            modifiers: vec![RustcLinkModifier::WholeArchive(
                RustcLinkModifierState::Enabled,
            )],
            name: String::from("mylib"),
            rename: None,
        })];

        pretty_assert_eq!(args.0, expected);

        Ok(())
    }

    #[test]
    fn parse_verbose_alias() -> Result<()> {
        let json = r#"["-v"]"#;
        let args = serde_json::from_str::<RustcArguments>(json).context("parse verbose alias")?;

        let expected = vec![RustcArgument::Verbose];

        pretty_assert_eq!(args.0, expected);

        Ok(())
    }
}
