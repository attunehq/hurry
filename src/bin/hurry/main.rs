use clap::{Parser, Subcommand};
use tracing::debug;
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

mod cargo;

#[derive(Parser)]
#[command(name = "hurry")]
#[command(about = "Really, really fast builds", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Subcommand)]
enum Command {
    /// Fast `cargo` builds
    #[command(dont_delimit_trailing_values = true)]
    Cargo {
        #[arg(
            num_args = ..,
            trailing_var_arg = true,
            allow_hyphen_values = true,
        )]
        argv: Vec<String>,
    },
    // TODO: /// Manage remote authentication
    // Auth,

    // TODO: Manage user cache, including busting it when it gets into a corrupt or weird state.
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_writer(std::io::stderr)
                .pretty(),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Cargo { argv } => {
            debug!(?argv, "cargo");

            // // Get the `--manifest-path` argument for `cargo {build,run}`.
            // //
            // // TODO: Should we do parsing this further up, and passing the flags
            // // downwards?
            // let mut args = std::env::args().skip_while(|val| !val.starts_with("--manifest-path"));
            // let mut cmd = cargo_metadata::MetadataCommand::new();
            // cmd.current_dir(dir);
            // match args.next() {
            //     Some(ref p) if p == "--manifest-path" => {
            //         cmd.manifest_path(args.next().expect("--manifest-path should provide a value"));
            //     }
            //     Some(p) => {
            //         cmd.manifest_path(p.trim_start_matches("--manifest-path="));
            //     }
            //     None => {}
            // }
            // let metadata = cmd.exec().context("could not get cargo metadata")?;
            // Ok(Self { metadata });

            // TODO: Technically, we should parse the argv properly in case
            // this string is passed as some sort of configuration flag value.
            if argv.contains(&"build".to_string()) {
                match cargo::build(&argv).await {
                    Ok(exit_status) => std::process::exit(exit_status.code().unwrap_or(1)),
                    Err(e) => panic!("hurry cargo build failed: {:?}", e),
                }
            } else {
                match cargo::exec(&argv).await {
                    Ok(exit_status) => std::process::exit(exit_status.code().unwrap_or(1)),
                    Err(e) => panic!("hurry cargo {} failed: {:?}", argv.join(" "), e),
                }
            }
        }
    }
}
