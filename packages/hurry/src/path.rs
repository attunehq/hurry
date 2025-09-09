//! Path types tailored to `hurry`.
//!
//! Inside this module, we refer to `std::path` by its fully
//! qualified path to make it maximally clear what we are using.
//!
//! ## Rationale
//!
//! `hurry` previously had a proliferation of path-like types:
//! - `std::path::{Path, PathBuf}` of course.
//! - `camino::{Utf8Path, Utf8PathBuf}` via `cargo_metadata`
//! - `relative_path::{RelativePath, RelativePathBuf}`
//!
//! These were all used to serve a few goals:
//! - Most FS APIs need `std::path` variants.
//! - Paths we reference are nearly always relative to the project workspace.
//! - We need to serialize paths to disk, and they need to be cross-platform.
//! - We aren't working with filesystems that support non-UTF8 paths,
//!   so we want to take advantage of these for pretty printing and
//!   string operations like "does this path contain this string".
//!
//! We also had some needs that no path-like type provided:
//! - We want all FS operations to go through the `fs` module,
//!   so operations like `PathBuf::exists` were not allowed,
//!   but we had no real way to actually enforce this.
//! - We want convenient creation of relative paths, and convenient
//!   conversion of relative paths to absolute paths,
//!   ideally cheaply.
//! - At the same time, we don't want relative paths to bend over backwards
//!   to create a "relative path" that is _so relative_ that it isn't
//!   cross platform/machine anymore (`relative_path`, I'm looking at you).
//!
//! Juggling all these different path types has turned into a nightmare
//! almost immediately, so we've created this module for our own path types
//! that provide all the needs above and any others we find later.

use std::{borrow::Cow, ffi::OsString, marker::PhantomData};

use color_eyre::{Result, eyre::Context};
use derive_more::{Display, Error};
use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use subenum::subenum;
use tap::Pipe;

use crate::fs;

pub type RelFileBuf = TypedPath<Rel, File>;
pub type RelDirBuf = TypedPath<Rel, Dir>;
pub type AbsFileBuf = TypedPath<Abs, File>;
pub type AbsDirBuf = TypedPath<Abs, Dir>;

/// Errors for path conversions in this module.
///
/// Different options return subenums of this main enum depending on their
/// actual possible failure modes; all subenums are trivially convertable
/// to this one if desired or can be handled distinctly.
#[subenum(
    AbsDirError,
    AbsFileError,
    RelDirError,
    RelFileError,
    MakeRelativeError
)]
#[derive(Clone, Debug, Display, Error)]
pub enum Error {
    /// The path was not absolute.
    #[subenum(AbsDirError, AbsFileError)]
    #[display("not absolute: {_0:?}")]
    NotAbsolute(#[error(not(source))] std::path::PathBuf),

    /// The path was not relative.
    #[subenum(RelDirError, RelFileError)]
    #[display("not relative: {_0:?}")]
    NotRelative(#[error(not(source))] std::path::PathBuf),

    /// The path was not a directory.
    #[subenum(AbsDirError, RelDirError)]
    #[display("not a directory: {_0:?}")]
    NotDirectory(#[error(not(source))] std::path::PathBuf),

    /// The path was not a file.
    #[subenum(AbsFileError, RelFileError)]
    #[display("not a file: {_0:?}")]
    NotFile(#[error(not(source))] std::path::PathBuf),

    /// Path is not able to be made relative to another path.
    #[subenum(MakeRelativeError)]
    #[display("{child:?} is not able to be made relative to {parent:?}: {source:?}")]
    NotChild {
        #[error(not(source))]
        parent: std::path::PathBuf,
        #[error(not(source))]
        child: std::path::PathBuf,
        source: std::path::StripPrefixError,
    },
}

/// Early return from the function with the provided error variant and
/// arguments. All arguments are transformed with `.into()`.
macro_rules! bail {
    ($err:path => $($arg:ident: $value:expr),* $(,)?) => {
        return Err({$err {
            $($arg: $value.into()),*
        }})
    };

    ($err:path => $($args:expr),* $(,)?) => {
        return Err($err($($args.into()),*))
    };
}

/// Make an instance of a [`Path<Rel, File>`] without validating
/// that it exists and is a file on disk.
///
/// This macro does perform compile-time validation that the path is not
/// an absolute path, but does not validate that the path exists
/// or that the path is a regular file.
///
/// ```
/// use hurry::path::mk_rel_file;
///
/// let file = mk_rel_file!("src/main.rs");
/// assert_eq!(file.as_std_path().to_str(), Some("src/main.rs"));
/// ```
#[macro_export]
macro_rules! mk_rel_file {
    ($path:literal) => {{
        $crate::assert_relative!($path);
        $crate::path::TypedPath::<$crate::path::Rel, $crate::path::File>::new($path.into())
    }};
}

/// Make an instance of a [`Path<Rel, Dir>`] without validating
/// that it exists and is a directory on disk.
///
/// This macro does perform compile-time validation that the path is not
/// an absolute path, but does not validate that the path exists
/// or that the path is a directory.
///
/// ```
/// use hurry::path::mk_rel_dir;
///
/// let dir = mk_rel_dir!("src");
/// assert_eq!(dir.as_std_path().to_str(), Some("src"));
/// ```
#[macro_export]
macro_rules! mk_rel_dir {
    ($path:literal) => {{
        $crate::assert_relative!($path);
        $crate::path::TypedPath::<$crate::path::Rel, $crate::path::Dir>::new($path.into())
    }};
}

/// Assert that the string provided indicates a relative path.
///
/// TODO: make this work on Windows too.
#[doc(hidden)]
#[macro_export]
macro_rules! assert_relative {
    ($path:literal) => {{
        const _: () = assert!(
            !const_str::starts_with!($path, '/'),
            "{}",
            const_str::format!("path is not relative: {:?}", $path),
        );
    }};
}

/// Indicates an unknown value for this path base.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct SomeBase;

/// Indicates an unknown value for this type of path.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct SomeType;

/// An absolute path always begins from the absolute start of the filesystem
/// and describes every step through the filesystem to end up at the target.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Abs;

/// A relative path is a "partial" path; it describes a path starting from
/// an undefined point. Once the "starting location" is given, the relative
/// path can take over, describing where to go from that location.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Rel;

/// A directory contains other file system entities,
/// such as files or other directories.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Dir;

/// A file contains data.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct File;

/// A location on the file system according to the type modifiers.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display)]
#[display("{}", self.inner.display())]
pub struct TypedPath<Base, Type> {
    base: PhantomData<Base>,
    ty: PhantomData<Type>,
    inner: std::path::PathBuf,
}

impl<Type> AsRef<std::path::Path> for TypedPath<Abs, Type> {
    fn as_ref(&self) -> &std::path::Path {
        &self.inner
    }
}

impl<Base, Type> AsRef<TypedPath<Base, Type>> for TypedPath<Base, Type> {
    fn as_ref(&self) -> &TypedPath<Base, Type> {
        self
    }
}

impl<Type> From<TypedPath<Abs, Type>> for std::path::PathBuf {
    fn from(value: TypedPath<Abs, Type>) -> Self {
        value.inner
    }
}

impl<Base, Type> TypedPath<Base, Type> {
    /// View the path as a standard path.
    pub fn as_std_path(&self) -> &std::path::Path {
        &self.inner
    }

    /// Get the parent of the provided path, if one exists.
    pub fn parent(&self) -> Option<TypedPath<Base, Dir>> {
        self.inner
            .parent()
            .map(ToOwned::to_owned)
            .map(TypedPath::new)
    }

    /// Convenience function to create an instance using the provided
    /// inner path.
    ///
    /// This is only exported so that macros can call it;
    /// do not call this as it has the potential to break invariants.
    #[doc(hidden)]
    pub const fn new(inner: std::path::PathBuf) -> Self {
        Self {
            base: PhantomData,
            ty: PhantomData,
            inner,
        }
    }
}

impl TypedPath<Abs, Dir> {
    pub async fn current() -> Result<Self> {
        std::env::current_dir()
            .map(Self::try_from_path)
            .context("get current directory")?
            .await
            .context("parse current directory")
    }

    pub async fn new_abs_dir(p: impl PathLike<'_>) -> Result<TypedPath<Abs, Dir>, AbsDirError> {
        TypedPath::<Abs, Dir>::try_from_path(p).await
    }

    pub fn make_relative_to(
        &self,
        anchor: impl AbsPathLike,
    ) -> Result<TypedPath<Rel, Dir>, MakeRelativeError> {
        let parent = anchor.as_path();
        Ok(match self.inner.strip_prefix(parent) {
            Ok(rel) => TypedPath::new(rel.to_path_buf()),
            Err(err) => {
                bail!(MakeRelativeError::NotChild => parent: parent, child: &self.inner, source: err);
            }
        })
    }

    pub async fn try_from_generic(
        value: TypedPath<SomeBase, SomeType>,
    ) -> Result<Self, AbsDirError> {
        Self::try_from_path(value.as_std_path()).await
    }

    pub async fn try_from_path(value: impl PathLike<'_>) -> Result<Self, AbsDirError> {
        let path = value.as_path();
        if !path.is_absolute() {
            bail!(AbsDirError::NotAbsolute => path);
        }
        if !fs::is_dir(&path).await {
            bail!(AbsDirError::NotDirectory => path);
        }
        Ok(Self {
            base: PhantomData,
            ty: PhantomData,
            inner: path.into_owned(),
        })
    }
}

impl TypedPath<Rel, Dir> {
    pub async fn try_from_generic(
        value: TypedPath<SomeBase, SomeType>,
    ) -> Result<Self, RelDirError> {
        Self::try_from_path(value.as_std_path()).await
    }

    pub async fn new_rel_dir(p: impl PathLike<'_>) -> Result<TypedPath<Rel, Dir>, RelDirError> {
        TypedPath::<Rel, Dir>::try_from_path(p).await
    }

    pub fn mk_rel_dir(p: impl AsRef<str>) -> Result<TypedPath<Rel, Dir>, RelDirError> {
        let path = std::path::PathBuf::from(p.as_ref());
        if path.is_absolute() {
            bail!(RelDirError::NotRelative => path);
        }
        Ok(TypedPath::<Rel, Dir>::new(path))
    }

    pub async fn try_from_path(value: impl PathLike<'_>) -> Result<Self, RelDirError> {
        let path = value.as_path();
        if path.is_absolute() {
            bail!(RelDirError::NotRelative => path);
        }
        if !fs::is_dir(&path).await {
            bail!(RelDirError::NotDirectory => path);
        }
        Ok(Self {
            base: PhantomData,
            ty: PhantomData,
            inner: path.into_owned(),
        })
    }

    pub fn make_abs_from(&self, anchor: impl AbsPathLike) -> TypedPath<Abs, Dir> {
        anchor
            .as_path()
            .join(self.as_std_path())
            .pipe(TypedPath::new)
    }
}

impl TypedPath<Abs, File> {
    pub fn make_relative_to(
        &self,
        anchor: impl AbsPathLike,
    ) -> Result<TypedPath<Rel, File>, MakeRelativeError> {
        let parent = anchor.as_path();
        Ok(match self.inner.strip_prefix(parent) {
            Ok(rel) => TypedPath::new(rel.to_path_buf()),
            Err(err) => {
                bail!(MakeRelativeError::NotChild => parent: parent, child: &self.inner, source: err);
            }
        })
    }

    pub async fn new_abs_file(p: impl PathLike<'_>) -> Result<TypedPath<Abs, File>, AbsFileError> {
        TypedPath::<Abs, File>::try_from_path(p).await
    }

    pub async fn try_from_generic(
        value: TypedPath<SomeBase, SomeType>,
    ) -> Result<Self, AbsFileError> {
        Self::try_from_path(value.as_std_path()).await
    }

    pub async fn try_from_path(value: impl PathLike<'_>) -> Result<Self, AbsFileError> {
        let path = value.as_path();
        if !path.is_absolute() {
            bail!(AbsFileError::NotAbsolute => path);
        }
        if !fs::is_file(&path).await {
            bail!(AbsFileError::NotFile => path);
        }
        Ok(Self {
            base: PhantomData,
            ty: PhantomData,
            inner: path.into_owned(),
        })
    }
}

impl TypedPath<Rel, File> {
    pub fn make_abs_from(&self, anchor: impl AbsPathLike) -> TypedPath<Abs, File> {
        anchor
            .as_path()
            .join(self.as_std_path())
            .pipe(TypedPath::new)
    }

    pub async fn new_rel_file(p: impl PathLike<'_>) -> Result<TypedPath<Rel, File>, RelFileError> {
        TypedPath::<Rel, File>::try_from_path(p).await
    }

    pub fn mk_rel_file(p: impl AsRef<str>) -> Result<TypedPath<Rel, File>, RelFileError> {
        let path = std::path::PathBuf::from(p.as_ref());
        if path.is_absolute() {
            bail!(RelFileError::NotRelative => path);
        }
        Ok(TypedPath::<Rel, File>::new(path))
    }

    pub async fn try_from_generic(
        value: TypedPath<SomeBase, SomeType>,
    ) -> Result<Self, RelFileError> {
        Self::try_from_path(value.as_std_path()).await
    }

    pub async fn try_from_path(value: impl PathLike<'_>) -> Result<Self, RelFileError> {
        let path = value.as_path();
        if path.is_absolute() {
            bail!(RelFileError::NotRelative => path);
        }
        if !fs::is_file(&path).await {
            bail!(RelFileError::NotFile => path);
        }
        Ok(Self {
            base: PhantomData,
            ty: PhantomData,
            inner: path.into_owned(),
        })
    }
}

impl<'de> Deserialize<'de> for TypedPath<SomeBase, SomeType> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = std::path::PathBuf::deserialize(deserializer)?;
        Ok(Self::new(s))
    }
}

/// Functionality for known absolute paths.
pub trait AbsPathLike {
    fn as_path(&self) -> &std::path::Path;
}

impl AbsPathLike for AbsDirBuf {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
    }
}
impl AbsPathLike for &AbsDirBuf {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
    }
}

impl AbsPathLike for AbsFileBuf {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
    }
}
impl AbsPathLike for &AbsFileBuf {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
    }
}

/// Functionality for known relative paths.
pub trait RelPathLike {
    fn as_path(&self) -> &std::path::Path;
}

impl RelPathLike for RelDirBuf {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
    }
}

impl RelPathLike for RelFileBuf {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
    }
}

/// Joins paths together, creating a different type.
pub trait JoinWith<Other> {
    type Output;

    /// Join `other` to `self`.
    fn join(&self, other: Other) -> Self::Output;
}

impl<Type> JoinWith<TypedPath<Rel, Type>> for TypedPath<Abs, Dir> {
    type Output = TypedPath<Abs, Type>;

    fn join(&self, other: TypedPath<Rel, Type>) -> Self::Output {
        self.join(&other)
    }
}

impl<Type> JoinWith<&TypedPath<Rel, Type>> for TypedPath<Abs, Dir> {
    type Output = TypedPath<Abs, Type>;

    fn join(&self, other: &TypedPath<Rel, Type>) -> Self::Output {
        self.as_std_path()
            .join(other.as_std_path())
            .pipe(TypedPath::new)
    }
}

/// Implemented by types that can be trivially converted to [`std::path::Path`]
/// and have the same or very similar semantics.
///
/// Note that [`PathBuf`] _does not_ implement this trait; this is because
/// it has deeper semantics than `std::path::Path` and also to avoid
/// conflicting with `std`-provided blanket `Into` implementations.
pub trait PathLike<'a> {
    fn as_path(self) -> Cow<'a, std::path::Path>;
}

impl<'a> PathLike<'a> for &'a cargo_metadata::camino::Utf8Path {
    fn as_path(self) -> Cow<'a, std::path::Path> {
        Cow::Borrowed(self.as_std_path())
    }
}

impl<'a> PathLike<'a> for cargo_metadata::camino::Utf8PathBuf {
    fn as_path(self) -> Cow<'a, std::path::Path> {
        Cow::Owned(self.into_std_path_buf())
    }
}

impl<'a> PathLike<'a> for &'a cargo_metadata::camino::Utf8PathBuf {
    fn as_path(self) -> Cow<'a, std::path::Path> {
        Cow::Borrowed(self.as_std_path())
    }
}

impl<'a> PathLike<'a> for &'a std::path::Path {
    fn as_path(self) -> Cow<'a, std::path::Path> {
        Cow::Borrowed(self)
    }
}

impl<'a> PathLike<'a> for std::path::PathBuf {
    fn as_path(self) -> Cow<'a, std::path::Path> {
        Cow::Owned(self)
    }
}

impl<'a> PathLike<'a> for &'a std::path::PathBuf {
    fn as_path(self) -> Cow<'a, std::path::Path> {
        Cow::Borrowed(self.as_path())
    }
}

impl<'a, P: PathLike<'a>> From<P> for TypedPath<SomeBase, SomeType> {
    fn from(value: P) -> Self {
        Self {
            base: PhantomData,
            ty: PhantomData,
            inner: value.as_path().into_owned(),
        }
    }
}
