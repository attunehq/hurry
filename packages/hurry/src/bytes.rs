//! Types and helper functions for working with plain bytes.

use std::fmt::Debug;

use bstr::ByteSlice;
use extfn::extfn;
use tracing::instrument;

/// Replace multiple slices in the byte buffer.
///
/// Tuples for `replacements` are in the form `(search, replacement)`, e.g.
/// ```
/// # use hurry::bytes::replace_all;
/// let greeting = b"Hello world!";
/// let farewell = greeting.replace_all([(b"Hello", b"Goodbye")]);
/// assert_eq!(greeting, b"Hello world!");
/// assert_eq!(farewell, b"Goodbye world!");
/// ```
///
/// Replacements are guaranteed to be performed in order; e.g. if you provide
/// `[(b"foobar", b"FOOBAR"), (b"foo", b"FOO")]` then all instances of `foobar`
/// are replaced before beginning replacements for `foo`:
/// ```
/// # use hurry::bytes::replace_all;
/// let foos = b"foobar foobaz foobam";
/// let excited_foos = foos.replace_all([
///     (b"foobar".as_slice(), b"FOOBAR".as_slice()),
///     (b"foo".as_slice(), b"FOO".as_slice()),
/// ]);
/// assert_eq!(excited_foos, b"FOOBAR FOObaz FOObam");
/// ```
#[extfn]
#[instrument(skip(self))]
pub fn replace_all(
    self: impl Into<Vec<u8>> + Debug,
    replacements: impl IntoIterator<Item = (impl AsRef<[u8]>, impl AsRef<[u8]>)> + Debug,
) -> Vec<u8> {
    // Note: we should probably use something like aho-corasick to speed this
    // up. `bstr` also has a `replace_into` method that may be faster. If this
    // function ends up being a bottleneck, those are probably places to start.
    let mut content = self.into();
    for (from, to) in replacements {
        content = content.replace(from.as_ref(), to.as_ref());
    }
    content
}
