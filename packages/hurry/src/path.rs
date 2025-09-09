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
    ffi::OsStr,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use cargo_metadata::camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use color_eyre::{Result, eyre::Context};
use derive_more::{Display, Error};
use duplicate::{duplicate, duplicate_item};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subenum::subenum;
use tap::Pipe;

use crate::fs;

pub type RelFilePath = TypedPath<Rel, File>;
pub type RelDirPath = TypedPath<Rel, Dir>;
pub type AbsFilePath = TypedPath<Abs, File>;
pub type AbsDirPath = TypedPath<Abs, Dir>;
pub type GenericPath = TypedPath<SomeBase, SomeType>;

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
    MakeRelativeError,
    MakeUtf8Error
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

    /// The path was not valid UTF8.
    #[subenum(AbsDirError, AbsFileError, RelDirError, RelFileError, MakeUtf8Error)]
    #[display("input path is not UTF8: {_0:?}")]
    NotUtf8(#[error(not(source))] std::path::PathBuf),

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
        $crate::path::RelFilePath::new_unchecked($path)
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
        $crate::path::RelDirPath::new_unchecked($path)
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
///
//
// TODO: This currently is not fully cross-platform as it does not attempt to
// normalize components in the path; when we decide to add Windows support
// we will need to handle this.
//
// TODO: We should really add async methods. Right now this isn't a huge deal
// since we're running client side anyway so we don't have to worry about e.g.
// "this blocking call stops the server from accepting a new connection"
// but it does harm the ability to e.g. do operations concurrently.
#[cfg(not(target_os = "windows"))]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Display)]
#[display("{}", self.inner)]
pub struct TypedPath<Base, Type> {
    base: PhantomData<Base>,
    ty: PhantomData<Type>,
    inner: cargo_metadata::camino::Utf8PathBuf,
}

impl<Base, Type> TypedPath<Base, Type> {
    /// View the path as a standard path.
    pub fn as_std_path(&self) -> &std::path::Path {
        self.inner.as_std_path()
    }

    /// View the path as a string.
    pub fn as_str(&self) -> &str {
        self.inner.as_str()
    }

    /// View the path as an OS string.
    pub fn as_os_str(&self) -> &OsStr {
        self.inner.as_os_str()
    }

    /// View the path as a UTF8 normalized path.
    pub fn as_utf8_path(&self) -> &cargo_metadata::camino::Utf8Path {
        &self.inner
    }

    /// Get the parent of the provided path, if one exists.
    pub fn parent(&self) -> Option<TypedPath<Base, Dir>> {
        self.inner
            .parent()
            .map(ToOwned::to_owned)
            .map(TypedPath::new_unchecked)
    }

    /// Iterate through the components of the path.
    pub fn components<'a>(&'a self) -> impl Iterator<Item = Utf8Component<'a>> {
        self.inner.components()
    }

    /// Returns the final component of the path, if there is one.
    ///
    /// If the path is a file, this is the file name.
    /// If it's the path of a directory, this is the directory name.
    pub fn file_name(&self) -> Option<&str> {
        self.inner.file_name()
    }

    /// Create the type without actually validating that it exists.
    ///
    /// This has the potential to break invariants, but is needed
    /// in some cases (notably when you're creating things).
    /// Try to minimize its use if at all possible.
    pub fn new_unchecked(inner: impl Into<String>) -> Self {
        Self {
            base: PhantomData,
            ty: PhantomData,
            inner: Utf8PathBuf::from(inner.into()),
        }
    }
}

impl<Base, Type> AsRef<TypedPath<Base, Type>> for TypedPath<Base, Type> {
    fn as_ref(&self) -> &TypedPath<Base, Type> {
        self
    }
}
impl<Base, Type> From<TypedPath<Base, Type>> for std::path::PathBuf {
    fn from(value: TypedPath<Base, Type>) -> Self {
        value.inner.into_std_path_buf()
    }
}
impl<Base, Type> From<&TypedPath<Base, Type>> for std::path::PathBuf {
    fn from(value: &TypedPath<Base, Type>) -> Self {
        value.inner.clone().into_std_path_buf()
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
        let path = cargo_metadata::camino::Utf8PathBuf::from(p.as_ref());
        if !path.is_relative() {
            bail!(err::NotRelative => path);
        }
        Ok(Self::new_unchecked(path))
    }
}

impl<'de> Deserialize<'de> for TypedPath<SomeBase, SomeType> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let p = cargo_metadata::camino::Utf8PathBuf::deserialize(deserializer)?;
        Ok(Self::new_unchecked(p))
    }
}

duplicate! {
    [
        ty_self fn_new ty_err;
        [ TypedPath<Abs, Dir> ] [ new_abs_dir ] [ AbsDirError ];
        [ TypedPath<Abs, File> ] [ new_abs_file ] [ AbsFileError ];
        [ TypedPath<Rel, Dir> ] [ new_rel_dir ] [ RelDirError ];
        [ TypedPath<Rel, File> ] [ new_rel_file ] [ RelFileError ];
    ]
    impl<'de> Deserialize<'de> for ty_self {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let p = TypedPath::<SomeBase, SomeType>::deserialize(deserializer)?;
            Self::try_from(p).map_err(serde::de::Error::custom)
        }
    }
    impl Serialize for ty_self {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            self.inner.serialize(serializer)
        }
    }

    // Note: Both of these functions exist so that callers can use
    // `TypedPath` and their aliases (e.g. `AbsDirPath`) interchangeably:
    // - `TypedPath::new_abs_dir` -> Coerces to `TypedPath<Abs, Dir>`;
    //   needed because methods on `TypedPath` alone are ambiguous
    //   if we're trying to lean on type inference.
    // - `AbsDirPath::new` -> Coerces to `TypedPath<Abs, Dir>`;
    //   more natural to type than `AbsDirPath::new_abs_dir`.
    // - `TypedPath::new` will then be ambiguous unless there's
    //   another source of type inference (as it should be).
    impl ty_self {
        /// Parse the provided path into the strongly typed path.
        ///
        /// This method validates that the path actually exists on disk
        /// and that it is the appropriate type.
        pub fn fn_new(p: impl AsRef<Path>) -> Result<Self, ty_err> {
            p.as_ref().pipe(Self::try_from)
        }
        /// Parse the provided path into the strongly typed path.
        ///
        /// This method validates that the path actually exists on disk
        /// and that it is the appropriate type.
        pub fn new(p: impl AsRef<Path>) -> Result<Self, ty_err> {
            Self::fn_new(p)
        }
    }
}

duplicate! {
    [
        ty_from ty_into_inner;
        [ PathBuf ] [ |p: PathBuf| Utf8PathBuf::try_from(p).map_err(|err| err.into_path_buf()) ];
        [ &Path ] [ |p: &Path| Utf8PathBuf::try_from(p.to_path_buf()).map_err(|err| err.into_path_buf()) ];
        [ Cow<'_, Path> ] [ |p: Cow<'_, Path>| Utf8PathBuf::try_from(p.to_path_buf()).map_err(|err| err.into_path_buf()) ];
        [ Utf8PathBuf ] [ |p: Utf8PathBuf| -> Result<_, PathBuf> { Ok(p) } ];
        [ &Utf8Path ] [ |p: &Utf8Path| -> Result<_, PathBuf> { Ok(p.to_owned()) } ];
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
            match (ty_into_inner)(value) {
                Ok(inner) => Ok(Self::new_unchecked(inner)),
                Err(value) => bail!(AbsDirError::NotUtf8 => value),
            }
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
            match (ty_into_inner)(value) {
                Ok(inner) => Ok(Self::new_unchecked(inner)),
                Err(value) => bail!(AbsFileError::NotUtf8 => value),
            }
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
            match (ty_into_inner)(value) {
                Ok(inner) => Ok(Self::new_unchecked(inner)),
                Err(value) => bail!(RelDirError::NotUtf8 => value),
            }
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
            match (ty_into_inner)(value) {
                Ok(inner) => Ok(Self::new_unchecked(inner)),
                Err(value) => bail!(RelFileError::NotUtf8 => value),
            }
        }
    }
    impl TryFrom<ty_from> for TypedPath<SomeBase, SomeType> {
        type Error = MakeUtf8Error;
        fn try_from(value: ty_from) -> Result<Self, Self::Error> {
            match (ty_into_inner)(value) {
                Ok(inner) => Ok(Self::new_unchecked(inner)),
                Err(value) => bail!(MakeUtf8Error::NotUtf8 => value),
            }
        }
    }
}

duplicate! {
    [
        ty_from;
        [ TypedPath<SomeBase, SomeType> ];
        [ &TypedPath<SomeBase, SomeType> ];
    ]
    #[duplicate_item(
        ty_to ty_err;
        [ TypedPath<Abs, Dir> ] [ AbsDirError ];
        [ TypedPath<Abs, File> ] [ AbsFileError ];
        [ TypedPath<Rel, Dir> ] [ RelDirError ];
        [ TypedPath<Rel, File> ] [ RelFileError ];
    )]
    impl TryFrom<ty_from> for ty_to {
        type Error = ty_err;
        fn try_from(value: ty_from) -> Result<Self, Self::Error> {
            value.inner.to_owned().pipe(Self::try_from)
        }
    }
}

/// Functionality for making a path relative using a base path.
pub trait RelativeTo<Other> {
    type Output;

    /// Make `self` relative to `other` if possible.
    fn relative_to(&self, other: Other) -> Self::Output;
}

duplicate! {
    [
        ty_other;
        [ TypedPath<Abs, Dir> ];
        [ TypedPath<Abs, File> ];
        [ &TypedPath<Abs, Dir> ];
        [ &TypedPath<Abs, File> ];
    ]
    #[duplicate_item(
        ty_self ty_output;
        [ TypedPath<Abs, Dir> ] [ TypedPath<Rel, Dir> ];
        [ TypedPath<Abs, File> ] [ TypedPath<Rel, File> ];
        [ &TypedPath<Abs, Dir> ] [ TypedPath<Rel, Dir> ];
        [ &TypedPath<Abs, File> ] [ TypedPath<Rel, File> ];
    )]
    impl RelativeTo<ty_other> for ty_self {
        type Output = Result<ty_output, MakeRelativeError>;

        fn relative_to(&self, other: ty_other) -> Self::Output {
            self.inner
                .strip_prefix(&other.inner)
                .map_err(|err| MakeRelativeError::NotChild {
                    parent: other.inner.clone().into(),
                    child: self.inner.clone().into(),
                    source: err,
                })
                .map(|p| TypedPath::new_unchecked(p.to_path_buf()))
        }
    }
}

/// Creates and joins a path from the input and confirms that
/// the overall path is valid.
pub trait TryJoinWith {
    /// Join `dir` to `self` as a directory.
    ///
    /// If joining multiple items, consider [`TryJoinWith::try_join_dirs`]
    /// or [`TryJoinWith::try_join_combined`] as these are more efficient.
    fn try_join_dir(&self, dir: impl AsRef<str>) -> Result<AbsDirPath, AbsDirError>;

    /// Join `file` to `self` as a file.
    ///
    /// If joining multiple items, consider [`TryJoinWith::try_join_dirs`]
    /// or [`TryJoinWith::try_join_combined`] as these are more efficient.
    fn try_join_file(&self, file: impl AsRef<str>) -> Result<AbsFilePath, AbsFileError>;

    /// Join multiple directories to `self`.
    /// The overall path is checked at the end instead of piece by piece.
    fn try_join_dirs(
        &self,
        dirs: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<AbsDirPath, AbsDirError>;

    /// Join multiple directories, followed by a file, to `self`.
    /// The overall path is checked at the end instead of piece by piece.
    fn try_join_combined(
        &self,
        others: impl IntoIterator<Item = impl AsRef<str>>,
        file: impl AsRef<str>,
    ) -> Result<AbsFilePath, AbsFileError>;
}

impl TryJoinWith for TypedPath<Abs, Dir> {
    fn try_join_dir(&self, other: impl AsRef<str>) -> Result<AbsDirPath, AbsDirError> {
        self.inner.join(other.as_ref()).pipe(AbsDirPath::new)
    }

    fn try_join_file(&self, other: impl AsRef<str>) -> Result<AbsFilePath, AbsFileError> {
        self.inner.join(other.as_ref()).pipe(AbsFilePath::new)
    }

    fn try_join_dirs(
        &self,
        others: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<AbsDirPath, AbsDirError> {
        let mut inner = self.inner.clone();
        for other in others {
            inner = inner.join(other.as_ref());
        }
        AbsDirPath::new(inner)
    }

    fn try_join_combined(
        &self,
        others: impl IntoIterator<Item = impl AsRef<str>>,
        file: impl AsRef<str>,
    ) -> Result<AbsFilePath, AbsFileError> {
        let mut inner = self.inner.clone();
        for other in others {
            inner = inner.join(other.as_ref());
        }
        inner.join(file.as_ref()).pipe(AbsFilePath::new)
    }
}

/// Joins known valid paths together.
pub trait JoinWith<Other> {
    type Output;

    /// Join `other` to `self`.
    fn join(&self, other: Other) -> Self::Output;
}

#[duplicate_item(
    ty_other ty_output;
    [ TypedPath<Rel, Dir> ] [ TypedPath<Abs, Dir> ];
    [ &TypedPath<Rel, Dir> ] [ TypedPath<Abs, Dir> ];
    [ TypedPath<Rel, File> ] [ TypedPath<Abs, File> ];
    [ &TypedPath<Rel, File> ] [ TypedPath<Abs, File> ];
)]
impl JoinWith<ty_other> for TypedPath<Abs, Dir> {
    type Output = ty_output;

    fn join(&self, other: ty_other) -> Self::Output {
        self.as_utf8_path()
            .join(other.as_utf8_path())
            .pipe(TypedPath::new_unchecked)
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
    fn as_path(&self) -> &cargo_metadata::camino::Utf8Path;
}

#[duplicate_item(
    ty;
    [ AbsDirPath ];
    [ &AbsDirPath ];
    [ AbsFilePath ];
    [ &AbsFilePath ];
    [ RelDirPath ];
    [ &RelDirPath ];
    [ RelFilePath ];
    [ &RelFilePath ];
    [ GenericPath ];
    [ &GenericPath ];
)]
impl TypedPathLike for ty {
    fn as_path(&self) -> &cargo_metadata::camino::Utf8Path {
        self.as_utf8_path()
    }
}

/// Functionality for known absolute paths.
pub trait AbsPathLike {
    fn as_path(&self) -> &cargo_metadata::camino::Utf8Path;
}

#[duplicate_item(
    ty;
    [ AbsDirPath ];
    [ AbsFilePath ];
    [ &AbsDirPath ];
    [ &AbsFilePath ];
)]
impl AbsPathLike for ty {
    fn as_path(&self) -> &cargo_metadata::camino::Utf8Path {
        self.as_utf8_path()
    }
}

/// Functionality for known relative paths.
pub trait RelPathLike {
    fn as_path(&self) -> &cargo_metadata::camino::Utf8Path;
}

#[duplicate_item(
    ty;
    [ RelDirPath ];
    [ &RelDirPath ];
    [ RelFilePath ];
    [ &RelFilePath ];
)]
impl RelPathLike for ty {
    fn as_path(&self) -> &cargo_metadata::camino::Utf8Path {
        self.as_utf8_path()
    }
}
