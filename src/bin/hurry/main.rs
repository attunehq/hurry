use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tracing::{debug, instrument};
use tracing_flame::FlameLayer;
use tracing_subscriber::{
    Layer as _, fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

mod cargo;

#[derive(Parser)]
#[command(name = "hurry")]
#[command(about = "Really, really fast builds", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Emit flamegraph profiling data
    #[arg(short, long, hide(true))]
    profile: Option<String>,
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
    // Cache,
}

#[tokio::main]
#[instrument(level = "debug")]
async fn main() -> ExitCode {
    // Parse command line arguments.
    let cli = Cli::parse();

    // Configure logging.
    let (flame_layer, flame_guard) = if let Some(profile) = cli.profile {
        let (flame_layer, _flame_guard) = FlameLayer::with_file(profile).unwrap();
        (Some(flame_layer), Some(_flame_guard))
    } else {
        (None, None)
    };
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
                .pretty()
                .with_filter(tracing_subscriber::EnvFilter::from_default_env()),
        )
        .with(flame_layer)
        .init();

    // Execute the command.
    match cli.command {
        Command::Cargo { argv } => {
            debug!(?argv, "cargo");

            // TODO: Technically, we should parse the argv properly in case
            // this string is passed as some sort of configuration flag value.
            if argv.contains(&"build".to_string()) {
                match cargo::build(&argv).await {
                    Ok(exit_status) => {
                        // Flush flamegraph data.
                        if let Some(flame_guard) = flame_guard {
                            flame_guard.flush().unwrap();
                        }
                        exit_status
                            .code()
                            .map_or(ExitCode::FAILURE, |c| ExitCode::from(c as u8))
                    }
                    Err(e) => panic!("hurry cargo build failed: {:?}", e),
                }
            } else {
                match cargo::exec(&argv).await {
                    Ok(exit_status) => {
                        // Flush flamegraph data.
                        if let Some(flame_guard) = flame_guard {
                            flame_guard.flush().unwrap();
                        }
                        exit_status
                            .code()
                            .map_or(ExitCode::FAILURE, |c| ExitCode::from(c as u8))
                    }
                    Err(e) => panic!("hurry cargo {} failed: {:?}", argv.join(" "), e),
                }
            }
        }
    }
}
