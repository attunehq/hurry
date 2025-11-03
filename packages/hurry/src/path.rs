//! Path types tailored to `hurry`.
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
//! - We used `Utf8Path` and friends because it was convenient.
//!
//! We also had some needs that no path-like type provided:
//! - We want all FS operations to go through the `fs` module, so operations
//!   like `PathBuf::exists` were not allowed, but we had no real way to
//!   actually enforce this.
//! - We want convenient creation of relative paths, and convenient conversion
//!   of relative paths to absolute paths, ideally cheaply.
//! - At the same time, we don't want relative paths to bend over backwards to
//!   create a "relative path" that is _so relative_ that it isn't cross
//!   platform/machine anymore (`relative_path`, I'm looking at you).
//!
//! Juggling all these different path types has turned into a nightmare
//! almost immediately, so we've created this module for our own path types
//! that provide all the needs above and any others we find later.
//!
//! ## Cross-Platform Support
//!
//! This module supports both Unix and Windows paths. Paths are stored as-is
//! without normalization, preserving the exact separators and format provided
//! by the caller.

use std::{
    any::type_name,
    borrow::Cow,
    ffi::{OsStr, OsString},
    marker::PhantomData,
    path::{Component, Path, PathBuf},
    str::FromStr,
};

use cargo_metadata::camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    Report, Result,
    eyre::{Context, bail},
};
use derive_more::Display;
use duplicate::{duplicate, duplicate_item};
use paste::paste;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tap::Pipe;

use crate::fs;

pub type RelFilePath = TypedPath<Rel, File>;
pub type RelDirPath = TypedPath<Rel, Dir>;
pub type RelSomePath = TypedPath<Rel, SomeType>;
pub type AbsFilePath = TypedPath<Abs, File>;
pub type AbsDirPath = TypedPath<Abs, Dir>;
pub type AbsSomePath = TypedPath<Abs, SomeType>;
pub type SomeDirPath = TypedPath<SomeBase, Dir>;
pub type SomeFilePath = TypedPath<SomeBase, File>;
pub type GenericPath = TypedPath<SomeBase, SomeType>;

/// Make an instance of a [`TypedPath<Rel, File>`] with compile-time validation.
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
        $crate::path::RelFilePath::try_from($path).unwrap()
    }};
}

/// Make an instance of a [`TypedPath<Rel, Dir>`] with compile-time validation.
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
        $crate::path::RelDirPath::try_from($path).unwrap()
    }};
}

/// Assert that the string provided indicates a relative path.
#[doc(hidden)]
#[macro_export]
macro_rules! assert_relative {
    ($path:literal) => {{
        #[cfg(unix)]
        const _: () = {
            assert!(!const_str::starts_with!($path, '/'), "path is not relative",);
        };

        #[cfg(windows)]
        const _: () = {
            // Reject drive letters: C:, D:, etc.
            assert!(
                !($path.len() >= 2
                    && ($path.as_bytes()[0] as char).is_ascii_alphabetic()
                    && $path.as_bytes()[1] == b':'),
                "path has drive letter"
            );

            // Reject UNC paths: \\server or //server
            assert!(
                !const_str::starts_with!($path, "\\\\") && !const_str::starts_with!($path, "//"),
                "path is UNC"
            );

            // Reject paths starting with separator.
            // Note: While Windows paths do not naturally start with '/', cross-platform
            // code or user input may provide such paths. This check is
            // intentionally defensive to catch both cases.
            assert!(
                !const_str::starts_with!($path, '/') && !const_str::starts_with!($path, '\\'),
                "path starts with separator"
            );
        };
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
/// This type is about _intent_ within the working program;
/// it does not actually validate that given resources on disk exist
/// or are the correct type.
///
/// The reason for this is because validating makes it very difficult to
/// construct paths that are _meant_ to indicate objects that don't yet exist;
/// at the same time validating is prone to race conditions (you can validate
/// and then another program can change the type of the resource) so all
/// that effort doesn't actually pay off.
///
/// Still, perfect is the enemy of good, and there's only so much
/// we can do with the giant ball of global mutable state that is a filesystem.
/// If you do want to validate the type matches, refer to [`TypedPath::exists`]
/// (but keep in mind that this is very prone to race conditions and has no
/// guarantees; you'll still need to handle errors).
///
/// ## Path manipulation
///
/// With the standard path-like types, you're probably used to methods like
/// `some_base.join("name")` or other similar operations.
///
/// Types in this module use strong types; in the above scenario prefer
/// e.g. `some_base.join(mk_rel_file!("name"))` (which has compile-time
/// validation) or `some_base.try_join_file("name")` (which has runtime
/// validation) or other similar methods.
///
/// ## Fallibility
///
/// Fallible methods on `TypedPath` variants are powered by instances of
/// the [`Validator`] trait on the `Base` and `Type` generics.
/// For example, the `Validator` implementation for [`Rel`] validates that
/// the path appears to be a relative path (e.g., it doesn't start with a `/`
/// on unix systems).
///
/// This is what powers fallible functionality: in all cases, the operation
/// succeeds if _all_ validators succeed, and fails if they do not.
///
/// Note: this does mean that in the future we could theoretically create
/// additional bases/types of paths with different validators and they would
/// effectively just slot in (although we currently generate most methods
/// using macros, not generics, so if we want plugin types to be supported
/// we'll need to revisit that).
///
/// ## Path Normalization
///
/// This type does NOT perform path normalization. Paths are stored exactly as
/// provided by the caller. In particular this means:
/// - `some/path` and `some/path/` are NOT considered equivalent.
/// - `some/path/../other` and `some/other` are NOT considered equivalent.
/// - `SOME/path` and `some/path` are NOT considered equivalent, even on case
///   insensitive file systems.
/// - On Windows, `some\path` and `some/path` are NOT considered equivalent
///   (though the OS treats them the same).
///
/// The reason for this is twofold: first, normalization would require lossy
/// conversions (e.g., `to_string_lossy()`) that could lose information for
/// non-UTF-8 paths. Second, we run into the validation issues noted earlier
/// in the docs for this type. If the caller cares about true normalization,
/// make sure to normalize before passing into this function.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Display)]
#[display("{}", self.inner.display())]
pub struct TypedPath<Base, Type> {
    /// The base of the path.
    base: PhantomData<Base>,

    /// The type of the path.
    ty: PhantomData<Type>,

    /// The inner path.
    inner: PathBuf,
}

impl<B, T> TypedPath<B, T> {
    /// View the path as a standard path.
    pub fn as_std_path(&self) -> &std::path::Path {
        &self.inner
    }

    /// View the path as a lossily-converted string.
    ///
    /// Any non-UTF-8 sequences are replaced with `U+FFFD REPLACEMENT CHARACTER`
    /// so be careful using this to construct _new_ paths.
    pub fn as_str_lossy(&self) -> Cow<'_, str> {
        self.inner.to_string_lossy()
    }

    /// View the path as an OS string.
    pub fn as_os_str(&self) -> &OsStr {
        self.inner.as_os_str()
    }

    /// Get the parent of the provided path, if one exists.
    ///
    /// Unlike the standard library, this method returns `None`
    /// if you request the parent of a relative path with one component.
    pub fn parent(&self) -> Option<TypedPath<B, Dir>> {
        self.inner
            .parent()
            .and_then(|p| {
                if p.as_os_str().is_empty() {
                    None
                } else {
                    Some(p)
                }
            })
            .map(ToOwned::to_owned)
            .map(TypedPath::new_unchecked)
    }

    /// Iterate through the components of the path.
    pub fn components<'a>(&'a self) -> impl DoubleEndedIterator<Item = Component<'a>> {
        self.inner.components()
    }

    /// Iterate through the components of the path as lossily-converted strings.
    ///
    /// Any non-UTF-8 sequences are replaced with `U+FFFD REPLACEMENT CHARACTER`
    /// so be careful using this to construct _new_ paths.
    pub fn component_strs_lossy<'a>(&'a self) -> impl DoubleEndedIterator<Item = Cow<'a, str>> {
        self.inner
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
    }

    /// Returns the final component of the path, if there is one.
    ///
    /// If the path is a file, this is the file name.
    /// If it's the path of a directory, this is the directory name.
    pub fn file_name(&self) -> Option<&OsStr> {
        self.inner.file_name()
    }

    /// Returns the final component of the path, if there is one,
    /// as a lossily-converted string.
    ///
    /// If the path is a file, this is the file name.
    /// If it's the path of a directory, this is the directory name.
    ///
    /// Any non-UTF-8 sequences are replaced with `U+FFFD REPLACEMENT CHARACTER`
    /// so be careful using this to construct _new_ paths.
    pub fn file_name_str_lossy(&self) -> Option<Cow<'_, str>> {
        self.inner.file_name().map(|s| s.to_string_lossy())
    }

    fn new_unchecked(inner: impl Into<PathBuf>) -> Self {
        Self {
            base: PhantomData,
            ty: PhantomData,
            inner: inner.into(),
        }
    }
}

// We use a macro here instead of merely writing out "impl TryFrom for all T
// where T can be converted into PathBuf" so that we can allow `TypedPath` to
// be converted into `PathBuf` (otherwise we conflict with the existing
// `impl From<T> for T` in `std`).
//
// We don't implement `AsRef<Path>` with `TypedPath`, so we _could_ use that,
// but then we'd be forced to uneccesarily clone paths that were moved.
// That's not a horrible tradeoff though if we ever find this list to be
// too restrictive.
#[duplicate_item(
    ty_from;
    [ PathBuf ];
    [ &PathBuf ];
    [ &Path ];
    [ String ];
    [ &String ];
    [ &str ];
    [ OsString ];
    [ &OsString ];
    [ &OsStr ];
    [ Utf8PathBuf ];
    [ &Utf8PathBuf ];
    [ &Utf8Path ];
)]
impl<B: Validator, T: Validator> TryFrom<ty_from> for TypedPath<B, T> {
    type Error = Report;

    fn try_from(value: ty_from) -> Result<Self, Self::Error> {
        #[allow(
            clippy::useless_conversion,
            reason = "This is only useless for one branch of the macro (i.e. PathBuf)"
        )]
        let value = PathBuf::from(value);
        B::validate(&value).with_context(|| format!("validate base {:?}", B::type_name()))?;
        T::validate(&value).with_context(|| format!("validate type {:?}", T::type_name()))?;
        Ok(Self::new_unchecked(value))
    }
}

impl<B: Validator, T: Validator> FromStr for TypedPath<B, T> {
    type Err = Report;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

impl<B, T> AsRef<TypedPath<B, T>> for TypedPath<B, T> {
    fn as_ref(&self) -> &TypedPath<B, T> {
        self
    }
}

impl<B, T> From<TypedPath<B, T>> for std::path::PathBuf {
    fn from(value: TypedPath<B, T>) -> Self {
        value.inner
    }
}

impl<B, T> From<&TypedPath<B, T>> for std::path::PathBuf {
    fn from(value: &TypedPath<B, T>) -> Self {
        value.inner.clone()
    }
}

impl<B: Clone, T: Clone> From<&TypedPath<B, T>> for TypedPath<B, T> {
    fn from(value: &TypedPath<B, T>) -> Self {
        value.clone()
    }
}

impl TypedPath<Abs, Dir> {
    /// Get the current working directory for the process.
    pub fn current() -> Result<TypedPath<Abs, Dir>> {
        let cwd = std::env::current_dir().context("get current dir")?;
        Self::try_from(cwd).context("convert")
    }
}

impl<'de, B: Validator, T: Validator> Deserialize<'de> for TypedPath<B, T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let p = PathBuf::deserialize(deserializer)?;
        Self::try_from(p).map_err(serde::de::Error::custom)
    }
}

impl<B, T> Serialize for TypedPath<B, T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.inner.serialize(serializer)
    }
}

impl<B, T> std::fmt::Debug for TypedPath<B, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TypedPath::<{}, {}>::({:?})",
            type_name::<B>(),
            type_name::<T>(),
            self.inner
        )
    }
}

#[duplicate_item(
    ty method;
    [ File ] [ fs::is_file ];
    [ Dir ] [ fs::is_dir ];
    [ SomeType ] [ fs::exists ];
)]
impl<B> TypedPath<B, ty> {
    /// Validate that the item exists on disk and is the correct type.
    ///
    /// Returns false if the item does not exist, or if there is an error
    /// checking whether the item exists. To disambiguate this case,
    /// use `fs` methods.
    ///
    /// Note that this method, like any similar method, is very susceptible to
    /// TOCTOU (time-of-check/time-of-use) bugs.
    pub async fn exists(&self) -> bool {
        method(self.as_std_path()).await
    }
}

// All paths can be converted to generic paths infallibly.
duplicate! {
    [
        from_base;
        [ Abs ];
        [ Rel ];
        [ SomeBase ];
    ]
    duplicate!{
        [
            from_ty;
            [ Dir ];
            [ File ];
            [ SomeType ];
        ]
        impl TypedPath<from_base, from_ty> {
            /// Convert the type to a generic path.
            pub fn as_generic(&self) -> TypedPath<SomeBase, SomeType> {
                TypedPath::<SomeBase, SomeType>::new_unchecked(&self.inner)
            }
        }
    }
}

// All combinations of bases and types can be _fallibly_ converted to
// `Abs`/`Rel` bases and `Dir`/`File` types.
duplicate! {
    [
        from_base;
        [ Abs ];
        [ Rel ];
        [ SomeBase ];
    ]
    duplicate!{
        [
            from_ty;
            [ Dir ];
            [ File ];
            [ SomeType ];
        ]
        duplicate! {
            [
                to_base to_base_name;
                [ Abs ] [ abs ];
                [ Rel ] [ rel ];
            ]
            duplicate!{
                [
                    to_ty to_ty_name;
                    [ Dir ] [ dir ];
                    [ File ] [ file ];
                ]
                impl TypedPath<from_base, from_ty> {
                    paste! {
                        /// Try to convert into the specified type.
                        pub fn [<try_as_ to_base_name _ to_ty_name>](&self) -> Result<TypedPath<to_base, to_ty>> {
                            TypedPath::<to_base, to_ty>::try_from(&self.inner)
                        }
                    }
                }
            }
        }
    }
}

// Relative or generic bases of all types can be fallibly converted
// to absolute bases of the same or less generic type
// using the process CWD if they are not currently absolute.
duplicate! {
    [
        from_base;
        [ Rel ];
        [ SomeBase ];
    ]
    duplicate!{
        [
            from_ty to_ty to_ty_name;
            [ Dir ] [ Dir ] [ dir ];
            [ File ] [ File ] [ file ];
            [ SomeType ] [ SomeType ] [ generic ];
            [ SomeType ] [ Dir ] [ dir ];
            [ SomeType ] [ File ] [ file ];
        ]
        impl TypedPath<from_base, from_ty> {
            paste! {
                /// Try to convert into the specified type,
                /// using the current working directory to promote
                /// the path if needed.
                ///
                /// ## Fallibility
                ///
                /// Along with the standard fallibility via the `Validator`
                /// trait explained in the docs for [`TypedPath`], this method
                /// also fails if [`AbsDirPath::current`] fails.
                pub fn [<try_as_abs_ to_ty_name _using_cwd>](&self) -> Result<TypedPath<Abs, to_ty>> {
                    if let Ok(p) = TypedPath::<Abs, to_ty>::try_from(&self.inner) {
                        return Ok(p);
                    }

                    let cwd = AbsDirPath::current()?;
                    TypedPath::<Abs, to_ty>::try_from(&cwd.inner.join(&self.inner))
                }
            }
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
        [ TypedPath<Abs, SomeType> ];
        [ &TypedPath<Abs, Dir> ];
        [ &TypedPath<Abs, File> ];
        [ &TypedPath<Abs, SomeType> ];
    ]
    #[duplicate_item(
        ty_self ty_output;
        [ TypedPath<Abs, Dir> ] [ TypedPath<Rel, Dir> ];
        [ TypedPath<Abs, File> ] [ TypedPath<Rel, File> ];
        [ TypedPath<Abs, SomeType> ] [ TypedPath<Rel, SomeType> ];
        [ &TypedPath<Abs, Dir> ] [ TypedPath<Rel, Dir> ];
        [ &TypedPath<Abs, File> ] [ TypedPath<Rel, File> ];
        [ &TypedPath<Abs, SomeType> ] [ TypedPath<Rel, SomeType> ];
    )]
    impl RelativeTo<ty_other> for ty_self {
        type Output = Result<ty_output>;

        fn relative_to(&self, other: ty_other) -> Self::Output {
            self.inner
                .strip_prefix(&other.inner)
                .with_context(|| format!("make {:?} relative to {:?}", other.inner, self.inner))
                .and_then(TypedPath::try_from)
        }
    }
}

/// Creates and joins a path from the input.
///
/// ## Fallibility
///
/// This trait takes strings for path segments; this means we don't know
/// whether the inputs are actually valid for the path being joined.
///
/// These methods are fallible to reflect this fact. The intention is that
/// implementations of this trait construct a `TypedPath` using the inputs,
/// and in the course of doing so they run the [`Validator`] implementations
/// for that path.
///
/// For more details on how `Validator` works, view the docs for [`TypedPath`].
pub trait TryJoinWith {
    /// Join `dir` to `self` as a directory.
    ///
    /// If joining multiple items, consider [`TryJoinWith::try_join_dirs`]
    /// or [`TryJoinWith::try_join_combined`] as these are more efficient.
    fn try_join_dir(&self, dir: impl AsRef<str>) -> Result<AbsDirPath>;

    /// Join `file` to `self` as a file.
    ///
    /// If joining multiple items, consider [`TryJoinWith::try_join_dirs`]
    /// or [`TryJoinWith::try_join_combined`] as these are more efficient.
    fn try_join_file(&self, file: impl AsRef<str>) -> Result<AbsFilePath>;

    /// Join multiple directories to `self`.
    /// The overall path is checked at the end instead of piece by piece.
    fn try_join_dirs(&self, dirs: impl IntoIterator<Item = impl AsRef<str>>) -> Result<AbsDirPath>;

    /// Join multiple directories, followed by a file, to `self`.
    /// The overall path is checked at the end instead of piece by piece.
    fn try_join_combined(
        &self,
        dirs: impl IntoIterator<Item = impl AsRef<str>>,
        file: impl AsRef<str>,
    ) -> Result<AbsFilePath>;
}

impl TryJoinWith for TypedPath<Abs, Dir> {
    fn try_join_dir(&self, other: impl AsRef<str>) -> Result<AbsDirPath> {
        self.inner.join(other.as_ref()).pipe(AbsDirPath::try_from)
    }

    fn try_join_file(&self, other: impl AsRef<str>) -> Result<AbsFilePath> {
        self.inner.join(other.as_ref()).pipe(AbsFilePath::try_from)
    }

    fn try_join_dirs(&self, dirs: impl IntoIterator<Item = impl AsRef<str>>) -> Result<AbsDirPath> {
        let mut inner = self.inner.clone();
        for other in dirs {
            inner = inner.join(other.as_ref());
        }
        AbsDirPath::try_from(inner)
    }

    fn try_join_combined(
        &self,
        dirs: impl IntoIterator<Item = impl AsRef<str>>,
        file: impl AsRef<str>,
    ) -> Result<AbsFilePath> {
        let mut inner = self.inner.clone();
        for other in dirs {
            inner = inner.join(other.as_ref());
        }
        inner.join(file.as_ref()).pipe(AbsFilePath::try_from)
    }
}

/// Infallibly joins known valid paths together.
pub trait JoinWith<Other> {
    type Output;

    /// Join `other` to `self`.
    fn join(&self, other: Other) -> Self::Output;
}

// We can always join typed relative paths of any type with absolute dir paths,
// and the output is always an absolute path of the same type.
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
        self.as_std_path()
            .join(other.as_std_path())
            .pipe(TypedPath::new_unchecked)
    }
}

/// Fallible methods on [`TypedPath`] variants are powered by instances of
/// the `Validator` trait on the `Base` and `Type` generics.
/// For example, the `Validator` implementation for [`Rel`] validates that
/// the path appears to be a relative path (e.g., it doesn't start with a `/`
/// on unix systems).
///
/// This is what powers fallible functionality: in all cases, the operation
/// succeeds if _all_ validators succeed, and fails if they do not.
pub trait Validator {
    /// Validate that the inner path for a [`TypedPath`] type matches
    /// the constraints of the validator, or return an error.
    fn validate(path: &Path) -> Result<()>;

    /// The name of the validator, for use in error messages.
    fn type_name() -> &'static str {
        core::any::type_name::<Self>()
    }
}

impl Validator for Rel {
    fn validate(path: &Path) -> Result<()> {
        if !path.is_relative() {
            bail!("path is not relative: {path:?}");
        }
        Ok(())
    }
}

impl Validator for Abs {
    fn validate(path: &Path) -> Result<()> {
        if !path.is_absolute() {
            bail!("path is not absolute: {path:?}");
        }
        Ok(())
    }
}

// To comply with the contract of `TypedPath` these need validators,
// but the validators are currently unconditionally passing.
#[duplicate_item(
    ty_self;
    [ Dir ];
    [ File ];
    [ SomeType ];
    [ SomeBase ];
)]
impl Validator for ty_self {
    fn validate(_: &Path) -> Result<()> {
        Ok(())
    }
}
