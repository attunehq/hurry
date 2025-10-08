use std::{collections::HashMap, str::FromStr};

use color_eyre::{Report, Result, eyre::Context};
use derive_more::Display;
use enum_assoc::Assoc;
use parse_display::{Display as ParseDisplay, FromStr as ParseFromStr};
use serde::Deserialize;

use crate::cargo::unit_graph::CargoCompileMode;

#[derive(Debug, Deserialize)]
pub struct BuildPlan {
    pub invocations: Vec<BuildPlanInvocation>,
    pub inputs: Vec<String>,
}

// Note that these fields are all undocumented. To see their definition, see
// https://github.com/rust-lang/cargo/blob/0436f86288a4d9bce1c712c4eea5b05eb82682b9/src/cargo/core/compiler/build_plan.rs#L21-L34
#[derive(Debug, Deserialize)]
pub struct BuildPlanInvocation {
    pub package_name: String,
    pub package_version: String,
    pub target_kind: Vec<cargo_metadata::TargetKind>,
    pub kind: Option<String>,
    pub compile_mode: CargoCompileMode,
    pub deps: Vec<usize>,
    pub outputs: Vec<String>,
    // Note that this map is a link of built artifacts to hardlinks on the
    // filesystem (that are used to alias the built artifacts). This does NOT
    // enumerate libraries being linked in.
    pub links: HashMap<String, String>,
    pub program: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: String,
}

/// A parsed argument for a `rustc` invocation.
///
/// ## Parsing format
///
/// For flags with values, handles parsing both space-separated (`--flag value`)
/// and equals-separated (`--flag=value`) flag formats. For flags without
/// values, parses the flag standalone (e.g. `--flag`).
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
pub enum RustcInvocationArgument {
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

    /// `-g` (alias for `-C debuginfo=2`)
    /// Parsed as `Codegen(Debuginfo(2))`

    /// `-O` (alias for `-C opt-level=3`)
    /// Parsed as `Codegen(OptLevel(3))`

    /// `-v` or `--verbose`
    Verbose,

    /// Any other unrecognized variant.
    ///
    /// `key` is the argument key and `value` is the argument value; e.g. these
    /// are all equivalent:
    /// ```not_rust
    /// "--someflag=somevalue"
    /// ["--someflag", "somevalue"]
    /// Other("--someflag", "somevalue")
    /// ```
    Other(String, String),
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

    #[display("link")]
    #[assoc(default_file_name = format!("{crate_name}.link"))]
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

/// Emit spec: `--emit <type>[=<file>]`
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct RustcEmitSpec {
    pub format: RustcEmitFormat,
    pub file: Option<String>,
}

impl FromStr for RustcEmitSpec {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self> {
        match s.split_once('=') {
            Some((format, file)) => Ok(Self {
                format: format.parse()?,
                file: Some(file.to_string()),
            }),
            None => Ok(Self {
                format: s.parse()?,
                file: None,
            }),
        }
    }
}

impl std::fmt::Display for RustcEmitSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.format)?;
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
    Debuginfo(u8),

    /// `opt-level=<level>`
    OptLevel(String),

    /// Any other codegen option
    Other(String, Option<String>),
}

impl FromStr for RustcCodegenOption {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self> {
        match s.split_once('=') {
            Some(("debuginfo", level)) => Ok(Self::Debuginfo(level.parse()?)),
            Some(("opt-level", level)) => Ok(Self::OptLevel(level.to_string())),
            Some((key, value)) => Ok(Self::Other(key.to_string(), Some(value.to_string()))),
            None => Ok(Self::Other(s.to_string(), None)),
        }
    }
}

impl std::fmt::Display for RustcCodegenOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Debuginfo(level) => write!(f, "debuginfo={level}"),
            Self::OptLevel(level) => write!(f, "opt-level={level}"),
            Self::Other(key, Some(value)) => write!(f, "{key}={value}"),
            Self::Other(key, None) => write!(f, "{key}"),
        }
    }
}
