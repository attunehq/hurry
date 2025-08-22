use clap::Subcommand;
use enum_assoc::Assoc;
use itertools::Itertools;
use strum::{EnumIter, IntoEnumIterator};
use subenum::subenum;
use tracing::instrument;

pub mod build;
pub mod run;

/// Supported cargo subcommands.
#[derive(Clone, Subcommand)]
pub enum Command {
    /// Fast `cargo` builds.
    Build(build::Options),

    /// Execute `cargo` commands.
    Run(run::Options),
}

/// The profile for the build.
///
/// Reference: https://doc.rust-lang.org/cargo/reference/profiles.html
//
// Note: We define `ProfileBuiltin` and only derive `EnumIter` on it
// because `EnumIter` over `Custom` does so over the default string value,
// which is an empty string; this is meaningless from an application logic
// perspective and can only ever result in bugs and wasted allocations.
#[subenum(ProfileBuiltin(derive(EnumIter)))]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Default, Assoc)]
#[func(pub const fn as_str(&self) -> &str)]
pub enum Profile {
    /// The `debug` profile.
    #[default]
    #[assoc(as_str = "debug")]
    #[subenum(ProfileBuiltin)]
    Debug,

    /// The `bench` profile.
    #[assoc(as_str = "bench")]
    #[subenum(ProfileBuiltin)]
    Bench,

    /// The `test` profile.
    #[assoc(as_str = "test")]
    #[subenum(ProfileBuiltin)]
    Test,

    /// The `release` profile.
    #[assoc(as_str = "release")]
    #[subenum(ProfileBuiltin)]
    Release,

    /// A custom user-specified profile.
    #[assoc(as_str = _0.as_str())]
    Custom(String),
}

impl Profile {
    /// Get the profile specified by the user.
    ///
    /// If the user didn't specify, defaults to [`Profile::Debug`].
    pub fn from_argv(argv: &[String]) -> Profile {
        if let Some(profile) = read_argv(argv, "profile") {
            return Profile::from(profile);
        }

        // TODO: today this will never result in `bench` or `test` profiles;
        // how should we detect and handle these?
        argv.iter()
            .tuple_windows()
            .find_map(|(a, b)| {
                if a == "--release" || b == "--release" {
                    Some(Profile::Release)
                } else {
                    None
                }
            })
            .unwrap_or(Profile::Debug)
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<String> for Profile {
    fn from(value: String) -> Self {
        for variant in ProfileBuiltin::iter() {
            if variant.as_str() == value {
                return variant.into();
            }
        }
        Profile::Custom(value)
    }
}

impl From<&str> for Profile {
    fn from(value: &str) -> Self {
        for variant in ProfileBuiltin::iter() {
            if variant.as_str() == value {
                return variant.into();
            }
        }
        Profile::Custom(value.to_string())
    }
}

impl From<&String> for Profile {
    fn from(value: &String) -> Self {
        value.as_str().into()
    }
}

/// Parse the value of an argument flag from `argv`.
///
/// Handles cases like:
/// - `--flag value`
/// - `--flag=value`
#[instrument(name = "cargo::read_argv")]
pub fn read_argv<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    debug_assert!(flag.starts_with("--"), "flag must start with `--`");
    argv.into_iter().tuple_windows().find_map(|(a, b)| {
        let (a, b) = (a.trim(), b.trim());

        // Handle the `--flag value` case, where the flag and its value
        // are distinct entries in `argv`.
        if a == flag {
            return Some(b);
        }

        // Handle the `--flag=value` case, where the flag and its value
        // are the same entry in `argv`.
        //
        // Due to how tuple windows work, this case could be in either
        // `a` or `b`. If `b` is the _last_ element in `argv`,
        // it won't be iterated over again as a future `a`,
        // so we have to check both.
        //
        // Unfortunately this leads to rework as all but the last `b`
        // will be checked again as a future `a`, but since `argv`
        // is relatively small this shouldn't be an issue in practice.
        //
        // Just in case I've thrown an `instrument` call on the function,
        // but this is extremely unlikely to ever be an issue.
        for v in [a, b] {
            if let Some((a, b)) = v.split_once('=') {
                if a == flag {
                    return Some(b);
                }
            }
        }

        None
    })
}
