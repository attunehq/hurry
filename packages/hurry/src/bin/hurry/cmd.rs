use clap::Args;
use derive_more::Debug;
use url::Url;

use clients::Token;

pub mod cache;
pub mod cargo;
pub mod cross;
pub mod daemon;
pub mod debug;

/// Shared options for Hurry build acceleration.
///
/// These options are common to both `cargo build` and `cross build` commands.
#[derive(Clone, Args, Debug)]
pub struct HurryOptions {
    /// Base URL for the Hurry API.
    #[arg(
        long = "hurry-api-url",
        env = "HURRY_API_URL",
        default_value = "https://app.hurry.build"
    )]
    #[debug("{api_url}")]
    pub api_url: Url,

    /// Authentication token for the Hurry API.
    // Note: this field is not _actually_ optional for `hurry` to operate; we're just telling clap
    // that it is so that if the user runs with the `-h` or `--help` arguments we can not require
    // the token in that case.
    #[arg(long = "hurry-api-token", env = "HURRY_API_TOKEN")]
    pub api_token: Option<Token>,

    /// Skip backing up the cache.
    #[arg(long = "hurry-skip-backup", default_value_t = false)]
    pub skip_backup: bool,

    /// Skip the build, only performing the cache actions.
    #[arg(long = "hurry-skip-build", default_value_t = false)]
    pub skip_build: bool,

    /// Skip restoring the cache.
    #[arg(long = "hurry-skip-restore", default_value_t = false)]
    pub skip_restore: bool,

    /// Upload artifacts asynchronously in the background instead of waiting.
    ///
    /// By default, hurry waits for uploads to complete before exiting.
    /// Use this flag to upload in the background and exit immediately after the
    /// build.
    #[arg(
        long = "hurry-async-upload",
        env = "HURRY_ASYNC_UPLOAD",
        default_value_t = false
    )]
    pub async_upload: bool,

    /// Show help for hurry-specific options.
    #[arg(long = "hurry-help", default_value_t = false)]
    pub help: bool,
}
