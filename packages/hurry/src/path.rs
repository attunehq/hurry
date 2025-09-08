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

use std::{borrow::Cow, marker::PhantomData};

use color_eyre::{Result, eyre::Context};
use derive_more::{Display, Error};
use subenum::subenum;
use tap::Pipe;

use crate::fs;

pub type RelFileBuf = PathBuf<Rel, File>;
pub type RelDirBuf = PathBuf<Rel, Dir>;
pub type AbsFileBuf = PathBuf<Abs, File>;
pub type AbsDirBuf = PathBuf<Abs, Dir>;

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
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct PathBuf<Base, Type> {
    base: PhantomData<Base>,
    ty: PhantomData<Type>,
    inner: std::path::PathBuf,
}

impl<Type> AsRef<std::path::Path> for PathBuf<Abs, Type> {
    fn as_ref(&self) -> &std::path::Path {
        &self.inner
    }
}

impl<Type> From<PathBuf<Abs, Type>> for std::path::PathBuf {
    fn from(value: PathBuf<Abs, Type>) -> Self {
        value.inner
    }
}

impl<Base, Type> PathBuf<Base, Type> {
    /// View the path as a standard path.
    pub fn as_std_path(&self) -> &std::path::Path {
        &self.inner
    }

    /// Convenience function to create an instance using the provided
    /// inner path. Nobody outside this module should be able to
    /// call this as it has the potential to break invariants.
    fn new(inner: std::path::PathBuf) -> Self {
        Self {
            base: PhantomData,
            ty: PhantomData,
            inner,
        }
    }
}

impl PathBuf<Abs, Dir> {
    pub async fn current() -> Result<Self> {
        std::env::current_dir()
            .map(Self::try_from_path)
            .context("get current directory")?
            .await
            .context("parse current directory")
    }

    pub fn make_relative_to(
        &self,
        anchor: impl Anchor,
    ) -> Result<PathBuf<Rel, Dir>, MakeRelativeError> {
        let parent = anchor.as_path();
        Ok(match self.inner.strip_prefix(parent) {
            Ok(rel) => PathBuf::new(rel.to_path_buf()),
            Err(err) => {
                bail!(MakeRelativeError::NotChild => parent: parent, child: &self.inner, source: err);
            }
        })
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

impl PathBuf<Rel, Dir> {
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

    pub fn make_abs_from(&self, anchor: impl Anchor) -> PathBuf<Abs, Dir> {
        anchor.as_path().join(self.as_std_path()).pipe(PathBuf::new)
    }
}

impl PathBuf<Abs, File> {
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

    pub fn make_relative_to(
        &self,
        anchor: impl Anchor,
    ) -> Result<PathBuf<Rel, File>, MakeRelativeError> {
        let parent = anchor.as_path();
        Ok(match self.inner.strip_prefix(parent) {
            Ok(rel) => PathBuf::new(rel.to_path_buf()),
            Err(err) => {
                bail!(MakeRelativeError::NotChild => parent: parent, child: &self.inner, source: err);
            }
        })
    }
}

impl PathBuf<Rel, File> {
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

    pub fn make_abs_from(&self, anchor: impl Anchor) -> PathBuf<Abs, File> {
        anchor.as_path().join(self.as_std_path()).pipe(PathBuf::new)
    }
}

/// Functionality for paths which can "anchor" other paths.
///
/// Anchors are able to have paths made relative to them
/// if the path being made relative is inside a "child path"
/// of the anchor. Anchors can additionally convert relative
/// paths to absolute paths.
pub trait Anchor {
    fn as_path(&self) -> &std::path::Path;
}

impl Anchor for AbsDirBuf {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
    }
}

impl Anchor for AbsFileBuf {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
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

impl<'a, P: PathLike<'a>> From<P> for PathBuf<SomeBase, SomeType> {
    fn from(value: P) -> Self {
        Self {
            base: PhantomData,
            ty: PhantomData,
            inner: value.as_path().into_owned(),
        }
    }
}
