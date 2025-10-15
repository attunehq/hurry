//! Extension traits

use std::fmt::Display;

use color_eyre::{Result, eyre::WrapErr};
use extfn::extfn;

/// Wrap the fallible future with the provided context once it completes.
///
/// Intended to make e.g. [`tokio::try_join`] and similar APIs easier.
#[extfn]
pub async fn then_context<R, T, E, F, D>(self: F, msg: D) -> Result<T>
where
    F: Future<Output = R>,
    R: WrapErr<T, E>,
    D: Display + Send + Sync + 'static,
{
    self.await.context(msg)
}

/// Wrap the fallible future with the provided context once it completes.
/// The context parameter is not executed unless the result is an error.
///
/// Intended to make e.g. [`tokio::try_join`] and similar APIs easier.
#[extfn]
pub async fn then_with_context<R, T, E, F, D, C>(self: F, msg: C) -> Result<T>
where
    F: Future<Output = R>,
    R: WrapErr<T, E>,
    D: Display + Send + Sync + 'static,
    C: FnOnce() -> D,
{
    self.await.with_context(msg)
}
