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

use std::{
    borrow::Cow,
    ffi::{OsStr, OsString},
    marker::PhantomData,
    path::Component,
};

use color_eyre::{Result, eyre::Context};
use derive_more::{Display, Error};
use duplicate::{duplicate, duplicate_item};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use subenum::subenum;
use tap::Pipe;

use crate::fs;

pub type RelFileBuf = TypedPath<Rel, File>;
pub type RelDirBuf = TypedPath<Rel, Dir>;
pub type AbsFileBuf = TypedPath<Abs, File>;
pub type AbsDirBuf = TypedPath<Abs, Dir>;
pub type GenericPathBuf = TypedPath<SomeBase, SomeType>;

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

/// Make an instance of a [`TypedPath<Rel, File>`] without validating
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

/// Make an instance of a [`TypedPath<Rel, Dir>`] without validating
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
#[cfg(not(target_os = "windows"))]
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
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct SomeBase;

/// Indicates an unknown value for this type of path.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct SomeType;

/// An absolute path always begins from the absolute start of the filesystem
/// and describes every step through the filesystem to end up at the target.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Abs;

/// A relative path is a "partial" path; it describes a path starting from
/// an undefined point. Once the "starting location" is given, the relative
/// path can take over, describing where to go from that location.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Rel;

/// A directory contains other file system entities,
/// such as files or other directories.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct Dir;

/// A file contains data.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct File;

/// A location on the file system according to the type modifiers.
///
/// This location is usually validated to exist and match the `Base` and `Type`
/// constraints at construction/conversion time; in other words if you
/// are working with a `TypedPath<Abs, Dir>` this type has been validated
/// to actually reference a concrete directory on disk and point to it
/// using an absolute path.
///
/// That being said, this isn't 100% foolproof as of course some other program
/// (or a different part of this program) can technically remove or alter
/// the path between when this type is constructed and when the type is
/// accessed.
///
/// There are also intended exceptions to this: specifically the [`mk_rel_dir`]
/// and [`mk_rel_file`] macros _do not_ validate that the path actually exists
/// or that the path is the correct type. The reasoning here is that sometimes
/// we have to have paths that reference things that don't yet exist
/// (for example, so that we can create them) and as such these escape hatches
/// sort of have to be present. There are a couple methods that do similar
/// operations for similar reasons.
///
/// Still, perfect is the enemy of good, and there's only so much
/// we can do with the giant ball of global mutable state that is a filesystem.
///
/// ## Path manipulation
///
/// With the standard path-like types, you're probably used to methods like
/// `some_base.join("name")` or other similar operations.
///
/// Types in this module use strong types; in the above scenario prefer
/// e.g. `some_base.join(mk_rel_file!("name"))` instead.
/// Similar differences hold for other similar operations.
///
/// ## Deserialization and conversion
///
/// Since we have to check whether the path exists and is the right type
/// at the time we construct this type, and the standard conversion (`TryFrom`)
/// and `Deserialize` implementations unfortunately aren't async,
/// we are forced to perform synchronous I/O. These operations _should_
/// be quite fast on modern file systems as they just check file metadata.
///
/// If this is not acceptable, the workaround is to use a blocking thread
/// e.g. via [`tokio::task::spawn_blocking`]. Note that as a special case,
/// [`TypedPath<SomeBase, SomeType>`] does not require I/O at all,
/// so is safe to use in structs that are `Deserialize`/`Serialize` without
/// any possibility of blocking I/O. You can then perform fallible conversion
/// using `TryFrom` in a blocking thread.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Display)]
#[display("{}", self.inner.display())]
pub struct TypedPath<Base, Type> {
    base: PhantomData<Base>,
    ty: PhantomData<Type>,
    inner: std::path::PathBuf,
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

    /// Iterate through the components of the path.
    pub fn components<'a>(&'a self) -> impl Iterator<Item = Component<'a>> {
        self.inner.components()
    }

    /// Returns the final component of the path, if there is one.
    ///
    /// If the path is a file, this is the file name.
    /// If it's the path of a directory, this is the directory name.
    pub fn file_name(&self) -> Option<&OsStr> {
        self.inner.file_name()
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

impl<Base, Type> AsRef<std::path::Path> for TypedPath<Base, Type> {
    fn as_ref(&self) -> &std::path::Path {
        &self.inner
    }
}
impl<Base, Type> AsRef<TypedPath<Base, Type>> for TypedPath<Base, Type> {
    fn as_ref(&self) -> &TypedPath<Base, Type> {
        self
    }
}
impl<Base, Type> From<TypedPath<Base, Type>> for std::path::PathBuf {
    fn from(value: TypedPath<Base, Type>) -> Self {
        value.inner
    }
}
impl<Base, Type> From<&TypedPath<Base, Type>> for std::path::PathBuf {
    fn from(value: &TypedPath<Base, Type>) -> Self {
        value.inner.clone()
    }
}
impl<Base: Clone, Type: Clone> From<&TypedPath<Base, Type>> for TypedPath<Base, Type> {
    fn from(value: &TypedPath<Base, Type>) -> Self {
        value.clone()
    }
}

impl TypedPath<Abs, Dir> {
    /// Get the current working directory for the process.
    pub fn current() -> Result<TypedPath<Abs, Dir>> {
        std::env::current_dir()
            .context("get current directory")
            .map(TypedPath::<SomeBase, SomeType>::from)
            .and_then(|p| Self::try_from(p).context("convert to typed abs dir"))
    }
}

#[duplicate_item(
    ty make err;
    [ TypedPath<Rel, File> ] [ dangerously_make_rel_file ] [ RelFileError ];
    [ TypedPath<Rel, Dir> ] [ dangerously_make_rel_dir ] [ RelDirError ];
)]
impl ty {
    /// Parse the provided path into the strongly typed path.
    ///
    /// This method validates that the path is not absolute, but does not
    /// validate that the path on disk is the correct type.
    ///
    /// Most of the time, users should prefer the `new_` method for this type;
    /// the intended use case for this function is when creating a type
    /// from a user-provided value prior to creating the path on disk,
    /// or to validate whether the path on disk exists.
    pub fn make(p: impl AsRef<str>) -> Result<ty, err> {
        let path = std::path::PathBuf::from(p.as_ref());
        if !path.is_relative() {
            bail!(err::NotRelative => path);
        }
        Ok(Self::new(path))
    }
}

#[duplicate_item(
    rel_ty abs_ty;
    [ TypedPath<Rel, File> ] [ TypedPath<Abs, File> ];
    [ TypedPath<Rel, Dir> ] [ TypedPath<Abs, Dir> ];
)]
impl rel_ty {
    /// Make the path absolute by joining it with the provided absolute base.
    pub fn abs(&self, anchor: impl AbsPathLike) -> abs_ty {
        anchor
            .as_path()
            .join(self.as_std_path())
            .pipe(TypedPath::new)
    }
}

#[duplicate_item(
    abs_ty rel_ty;
    [ TypedPath<Abs, File> ] [ TypedPath<Rel, File> ];
    [ TypedPath<Abs, Dir> ] [ TypedPath<Rel, Dir> ];
)]
impl abs_ty {
    /// Make the path absolute by joining it with the provided absolute base.
    pub fn rel_to(&self, anchor: impl AbsPathLike) -> Result<rel_ty, MakeRelativeError> {
        self.inner
            .strip_prefix(anchor.as_path())
            .map_err(|err| MakeRelativeError::NotChild {
                parent: anchor.as_path().to_path_buf(),
                child: self.inner.clone(),
                source: err,
            })
            .map(|p| TypedPath::new(p.to_path_buf()))
    }
}

impl TypedPath<SomeBase, SomeType> {
    /// Report whether the path is absolute.
    ///
    /// This exists mainly for compatibility with standard-like path types
    /// so that we can reuse the logic for converting them.
    fn is_absolute(&self) -> bool {
        self.inner.is_absolute()
    }

    /// Report whether the path is relative.
    ///
    /// This exists mainly for compatibility with standard-like path types
    /// so that we can reuse the logic for converting them.
    fn is_relative(&self) -> bool {
        self.inner.is_relative()
    }
}

impl<'de> Deserialize<'de> for TypedPath<SomeBase, SomeType> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let p = std::path::PathBuf::deserialize(deserializer)?;
        Ok(Self::new(p))
    }
}

duplicate! {
    [
        ty new ty_err;
        [ TypedPath<Abs, Dir> ] [ new_abs_dir ] [ AbsDirError ];
        [ TypedPath<Abs, File> ] [ new_abs_file ] [ AbsFileError ];
        [ TypedPath<Rel, Dir> ] [ new_rel_dir ] [ RelDirError ];
        [ TypedPath<Rel, File> ] [ new_rel_file ] [ RelFileError ];
    ]
    impl<'de> Deserialize<'de> for ty {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let p = TypedPath::<SomeBase, SomeType>::deserialize(deserializer)?;
            Self::try_from(p).map_err(serde::de::Error::custom)
        }
    }
    impl Serialize for ty {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            self.inner.serialize(serializer)
        }
    }
    impl ty {
        /// Parse the provided path into the strongly typed path.
        ///
        /// This method validates that the path actually exists on disk
        /// and that it is the appropriate type.
        pub fn new<'a>(p: impl PathLike<'a>) -> Result<Self, ty_err> {
            TypedPath::<SomeBase, SomeType>::from(p)
                .pipe(Self::try_from)
        }
    }
}

duplicate! {
    [
        ty_from;
        [ std::path::PathBuf ];
        [ &std::path::Path ];
        [ cargo_metadata::camino::Utf8PathBuf ];
        [ &cargo_metadata::camino::Utf8Path ];
        [ TypedPath<SomeBase, SomeType> ];
        [ &TypedPath<SomeBase, SomeType> ];
    ]
    impl TryFrom<ty_from> for TypedPath<Abs, Dir> {
        type Error = AbsDirError;
        fn try_from(value: ty_from) -> Result<Self, Self::Error> {
            if !value.is_absolute() {
                bail!(AbsDirError::NotAbsolute => value);
            }
            if !fs::is_dir_sync(&value) {
                bail!(AbsDirError::NotDirectory => value);
            }
            Ok(Self::new(value.into()))
        }
    }
    impl TryFrom<ty_from> for TypedPath<Abs, File> {
        type Error = AbsFileError;
        fn try_from(value: ty_from) -> Result<Self, Self::Error> {
            if !value.is_absolute() {
                bail!(AbsFileError::NotAbsolute => value);
            }
            if !fs::is_file_sync(&value) {
                bail!(AbsFileError::NotFile => value);
            }
            Ok(Self::new(value.into()))
        }
    }
    impl TryFrom<ty_from> for TypedPath<Rel, Dir> {
        type Error = RelDirError;
        fn try_from(value: ty_from) -> Result<Self, Self::Error> {
            if !value.is_relative() {
                bail!(RelDirError::NotRelative => value);
            }
            if !fs::is_dir_sync(&value) {
                bail!(RelDirError::NotDirectory => value);
            }
            Ok(Self::new(value.into()))
        }
    }
    impl TryFrom<ty_from> for TypedPath<Rel, File> {
        type Error = RelFileError;
        fn try_from(value: ty_from) -> Result<Self, Self::Error> {
            if value.is_absolute() {
                bail!(RelFileError::NotRelative => value);
            }
            if !fs::is_file_sync(&value) {
                bail!(RelFileError::NotFile => value);
            }
            Ok(Self::new(value.into()))
        }
    }
}

/// Joins paths together, creating a different type.
pub trait JoinWith<Other> {
    type Output;

    /// Join `other` to `self`.
    fn join(&self, other: Other) -> Self::Output;
}

#[duplicate_item(
    ty;
    [ TypedPath<Rel, Type> ];
    [ &TypedPath<Rel, Type> ];
)]
impl<Type> JoinWith<ty> for TypedPath<Abs, Dir> {
    type Output = TypedPath<Abs, Type>;

    fn join(&self, other: ty) -> Self::Output {
        self.as_std_path()
            .join(other.as_std_path())
            .pipe(TypedPath::new)
    }
}

#[duplicate_item(
    ty name;
    [ TypedPath<SomeBase, SomeType> ] [ "Generic" ];
    [ TypedPath<Abs, Dir> ] [ "AbsDir" ];
    [ TypedPath<Abs, File> ] [ "AbsFile" ];
    [ TypedPath<Rel, Dir> ] [ "RelDir" ];
    [ TypedPath<Rel, File> ] [ "RelFile" ];
)]
impl std::fmt::Debug for ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({:?})", name, self.inner)
    }
}

/// Shared functionality for known typed paths.
pub trait TypedPathLike {
    fn as_path(&self) -> &std::path::Path;
}

#[duplicate_item(
    ty;
    [ AbsDirBuf ];
    [ &AbsDirBuf ];
    [ AbsFileBuf ];
    [ &AbsFileBuf ];
    [ RelDirBuf ];
    [ &RelDirBuf ];
    [ RelFileBuf ];
    [ &RelFileBuf ];
    [ GenericPathBuf ];
    [ &GenericPathBuf ];
)]
impl TypedPathLike for ty {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
    }
}

/// Functionality for known absolute paths.
pub trait AbsPathLike {
    fn as_path(&self) -> &std::path::Path;
}

#[duplicate_item(
    ty;
    [ AbsDirBuf ];
    [ AbsFileBuf ];
    [ &AbsDirBuf ];
    [ &AbsFileBuf ];
)]
impl AbsPathLike for ty {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
    }
}

/// Functionality for known relative paths.
pub trait RelPathLike {
    fn as_path(&self) -> &std::path::Path;
}

#[duplicate_item(
    ty;
    [ RelDirBuf ];
    [ &RelDirBuf ];
    [ RelFileBuf ];
    [ &RelFileBuf ];
)]
impl RelPathLike for ty {
    fn as_path(&self) -> &std::path::Path {
        self.as_std_path()
    }
}

/// Implemented by types that can be trivially converted to [`std::path::Path`]
/// and have the same or very similar semantics.
pub trait PathLike<'a> {
    fn as_path(self) -> Cow<'a, std::path::Path>;
}

#[duplicate_item(
    ty expr;
    [ &'a cargo_metadata::camino::Utf8Path ] [ Cow::Borrowed(self.as_std_path()) ];
    [ &'a std::path::Path ] [ Cow::Borrowed(self) ];
    [ cargo_metadata::camino::Utf8PathBuf ] [ Cow::Owned(self.into_std_path_buf()) ];
    [ &'a cargo_metadata::camino::Utf8PathBuf ] [ Cow::Borrowed(self.as_std_path()) ];
    [ std::path::PathBuf ] [ Cow::Owned(self) ];
    [ &'a std::path::PathBuf ] [ Cow::Borrowed(self.as_path()) ];
    [ Cow<'a, std::path::Path> ] [ self ];
)]
impl<'a> PathLike<'a> for ty {
    fn as_path(self) -> Cow<'a, std::path::Path> {
        expr
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
