//! Library for `hurry`.
//!
//! This library is not intended to be used directly and is unsupported in
//! that configuration. It's only a library to enable sharing code in `hurry`
//! with benchmarks and integration tests in the `hurry` repository.

use derive_more::Display;

pub mod cargo;
pub mod cas;
pub mod client;
pub mod ext;
pub mod fs;
pub mod hash;
pub mod path;

/// The associated type's state is unlocked.
/// Used for the typestate pattern.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Default)]
pub struct Unlocked;

/// The associated type's state is locked.
/// Used for the typestate pattern.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Default)]
pub struct Locked;
