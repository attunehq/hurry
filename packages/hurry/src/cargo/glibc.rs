use std::{cmp::Ordering, ffi::CStr};

use color_eyre::{
    Result,
    eyre::{self, bail, eyre},
};
use tap::{Pipe as _, TryConv};

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct GLIBCVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

impl TryFrom<&str> for GLIBCVersion {
    type Error = eyre::Report;

    // For reference, see the full list of glibc versions[^1].
    //
    // [^1]: https://sourceware.org/glibc/wiki/Glibc%20Timeline
    fn try_from(s: &str) -> Result<Self> {
        let mut parts = s.split('.');
        let major = parts
            .next()
            .ok_or(eyre!("invalid glibc version"))?
            .parse()?;
        let minor = parts
            .next()
            .ok_or(eyre!("invalid glibc version"))?
            .parse()?;
        // Patch versions are optional, and default to zero for comparison purposes.
        let patch = parts.next().map(str::parse::<u32>).unwrap_or(Ok(0))?;
        // Make sure there are no remaining parts.
        if parts.next().is_some() {
            bail!("invalid glibc version");
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

impl Ord for GLIBCVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
    }
}

impl PartialOrd for GLIBCVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn host_glibc_version() -> Result<Option<GLIBCVersion>> {
    if cfg!(target_env = "gnu") {
        // TODO: Does this actually get the specific libc that rustc will
        // compile user code against? Maybe we have to run a special command to
        // resolve that libc? Or parse it out of the args? Or maybe this is
        // actually just up to how the system linker is configured?
        //
        // One thing to try:
        // ```
        // echo 'fn main() { println!("")}' | rustc -C link-args=-Wl,-Map=map.out -o foo -
        // ```
        let version_ptr = unsafe { libc::gnu_get_libc_version() };
        let version_str = unsafe { CStr::from_ptr(version_ptr) };
        version_str.to_str()?.try_conv::<GLIBCVersion>()?.pipe(Some)
    } else {
        None
    }
    .pipe(Ok)
}
