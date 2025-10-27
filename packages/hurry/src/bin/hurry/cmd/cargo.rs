use clap::Subcommand;

pub mod build;
pub mod passthrough;
pub mod run;

/// Supported cargo subcommands.
#[derive(Clone, Subcommand)]
#[command(disable_help_subcommand = true)]
pub enum Command {
    /// Fast `cargo` builds with caching.
    Build(build::Options),

    /// Execute `cargo run`.
    Run(run::Options),

    /// Check the current package.
    Check(passthrough::Options),

    /// Remove the target directory.
    Clean(passthrough::Options),

    /// Build documentation.
    Doc(passthrough::Options),

    /// Run tests.
    Test(passthrough::Options),

    /// Run benchmarks.
    Bench(passthrough::Options),

    /// Add dependencies to manifest.
    Add(passthrough::Options),

    /// Remove dependencies from manifest.
    Remove(passthrough::Options),

    /// Create a new cargo package.
    New(passthrough::Options),

    /// Create a new cargo package in existing directory.
    Init(passthrough::Options),

    /// Update dependencies in Cargo.lock.
    Update(passthrough::Options),

    /// Search registry for crates.
    Search(passthrough::Options),

    /// Package and upload to registry.
    Publish(passthrough::Options),

    /// Install a Rust binary.
    Install(passthrough::Options),

    /// Uninstall a Rust binary.
    Uninstall(passthrough::Options),

    /// Fetch dependencies from network.
    Fetch(passthrough::Options),

    /// Automatically fix lint warnings.
    Fix(passthrough::Options),

    /// Compile a package with custom flags.
    Rustc(passthrough::Options),

    /// Build documentation with custom flags.
    Rustdoc(passthrough::Options),

    /// Display dependency tree.
    Tree(passthrough::Options),

    /// Vendor all dependencies locally.
    Vendor(passthrough::Options),

    /// Assemble into distributable tarball.
    Package(passthrough::Options),

    /// Print fully qualified package specification.
    Pkgid(passthrough::Options),

    /// Output resolved dependencies metadata.
    Metadata(passthrough::Options),

    /// Print Cargo.toml file location.
    #[command(name = "locate-project")]
    LocateProject(passthrough::Options),

    /// Inspect configuration values.
    Config(passthrough::Options),

    /// Manage crate owners on registry.
    Owner(passthrough::Options),

    /// Log in to a registry.
    Login(passthrough::Options),

    /// Remove API token from registry.
    Logout(passthrough::Options),

    /// Remove pushed crate from index.
    Yank(passthrough::Options),

    /// Show version information.
    Version(passthrough::Options),

    /// Generate and display reports.
    Report(passthrough::Options),

    /// Generate Cargo.lock file.
    #[command(name = "generate-lockfile")]
    GenerateLockfile(passthrough::Options),

    /// Display information about a package.
    Info(passthrough::Options),

    /// Display help for a cargo command.
    Help(passthrough::Options),

    /// Pass through any other cargo command (including plugins).
    #[command(external_subcommand)]
    External(Vec<String>),
}
