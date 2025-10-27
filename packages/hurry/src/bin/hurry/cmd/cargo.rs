use clap::Subcommand;
use color_eyre::Result;

pub mod build;
pub mod passthrough;
pub mod run;

/// Execute a cargo command by dispatching based on the first argument.
pub async fn exec(args: Vec<String>) -> Result<()> {
    use hurry::cargo;

    // If no args, passthrough to cargo (shows help)
    if args.is_empty() {
        return cargo::invoke("", Vec::<String>::new()).await;
    }

    let first = &args[0];

    // Check if it's a flag (starts with -)
    if first.starts_with('-') {
        // Flags like --version, --help, etc. - passthrough everything
        return cargo::invoke(&args[0], &args[1..]).await;
    }

    // It's a subcommand - check if we have special handling
    let subcommand = first;
    let rest = args[1..].to_vec();

    match subcommand.as_str() {
        "build" => {
            // Use hurry's optimized build with caching
            // Parse args using a temporary Command wrapper
            use clap::{Command as ClapCommand, Parser};

            #[derive(Parser)]
            struct BuildCommand {
                #[clap(flatten)]
                opts: build::Options,
            }

            let cmd = BuildCommand::try_parse_from(
                std::iter::once("build".to_string()).chain(rest.into_iter()),
            )?;
            build::exec(cmd.opts).await
        }
        // All other commands: passthrough to cargo
        _ => cargo::invoke(subcommand, rest).await,
    }
}

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
