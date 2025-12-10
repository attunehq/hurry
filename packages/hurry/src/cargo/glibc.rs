use std::ffi::CStr;

use clients::courier::v1::GLIBCVersion;
use color_eyre::{Result, eyre::Context as _};
use tap::Pipe as _;

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
        //
        // TODO: How do we tell if the unit doesn't compile against glibc at all
        // e.g. if it's `no_std`? Cargo and rustc don't seem to provide this
        // information. For `.so`s, we can probably look at the dynsymtab in the
        // binary, but I'm not sure about build script outputs.
        let version_ptr = unsafe { libc::gnu_get_libc_version() };
        let version_str = unsafe { CStr::from_ptr(version_ptr) };
        version_str
            .to_str()
            .context("parsing glibc version as UTF-8")?
            .parse::<GLIBCVersion>()
            .context("parsing glibc version")?
            .pipe(Some)
    } else {
        None
    }
    .pipe(Ok)
}
