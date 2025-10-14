//! Extension traits

use std::fmt::Display;

use color_eyre::{Result, eyre::WrapErr};
use extfn::extfn;
use url::Url;

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

/// Join all provided components into the URL.
///
/// This is a convenience method over joining multiple components using
/// [`Url::join`], allowing the caller to check the result for errors once at
/// the end instead of piece by piece.
///
/// # Notes
///
/// - A trailing slash is significant. Without it, the last path component is
///   considered to be a "file" name to be removed to get at the "directory"
///   that is used as the base.
/// - A [scheme relative special
///   URL](https://url.spec.whatwg.org/#scheme-relative-special-url-string) as
///   input replaces everything in the base URL after the scheme.
/// - An absolute URL (with a scheme) as input replaces the whole base URL (even
///   the scheme).
#[extfn]
pub fn join_all(
    self: &Url,
    components: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<Url, url::ParseError> {
    let mut base = self.clone();
    for component in components {
        base = base.join(component.as_ref())?;
    }
    Ok(base)
}
