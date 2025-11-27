use enum_assoc::Assoc;
use serde::{Deserialize, Serialize};
use strum::{EnumIter, IntoEnumIterator};
use subenum::subenum;

/// Cargo build profile specification.
///
/// Represents the different compilation profiles available in Cargo,
/// including the four built-in profiles (`debug`, `release`, `test`, `bench`)
/// and support for custom user-defined profiles.
///
/// Used for cache key generation and target directory organization.
/// Each profile has different optimization settings and produces different
/// compilation artifacts that must be cached separately.
///
/// Reference: https://doc.rust-lang.org/cargo/reference/profiles.html
//
// Note: We define `ProfileBuiltin` and only derive `EnumIter` on it
// because `EnumIter` over `Custom` does so over the default string value,
// which is an empty string; this is meaningless from an application logic
// perspective and can only ever result in bugs and wasted allocations.
#[subenum(ProfileBuiltin(derive(EnumIter)))]
#[derive(Clone, Eq, PartialEq, Hash, Debug, Default, Assoc, Serialize, Deserialize)]
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
        Profile::Custom(String::from(value))
    }
}

impl From<&String> for Profile {
    fn from(value: &String) -> Self {
        value.as_str().into()
    }
}
