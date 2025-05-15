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
        // #[command(subcommand)]
        // command: Option<CargoCommand>,
        #[arg(
            num_args = ..,
            trailing_var_arg = true,
            allow_hyphen_values = true,
            // dont_delimit_trailing_values = true,
        )]
        argv: Vec<String>,
    },
    // /// Manage remote authentication
    // Auth,
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
