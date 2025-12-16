use clap::Args;
use clients::{Courier, Token, courier::v1::cache::CargoRestoreRequest};
use color_eyre::Result;
use derive_more::Debug;
use hurry::cargo::{CargoBuildArguments, Workspace, host_glibc_version};
use url::Url;

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Base URL for the Hurry API.
    #[arg(
        long = "api-url",
        env = "HURRY_API_URL",
        default_value = "https://app.hurry.build"
    )]
    #[debug("{api_url}")]
    api_url: Url,

    /// Authentication token for the Hurry API.
    #[arg(long = "api-token", env = "HURRY_API_TOKEN")]
    api_token: Token,

    /// Name of the package to display.
    #[arg(long)]
    name: Option<String>,

    /// Version of the package to display.
    #[arg(long)]
    version: Option<String>,
}

pub async fn exec(opts: Options) -> Result<()> {
    let args = CargoBuildArguments::empty();
    let workspace = Workspace::from_argv(args.clone()).await?;
    let units = workspace.units(&args).await?;
    let matching_units = units
        .into_iter()
        .filter(|unit| {
            let info = unit.info();
            if let Some(name) = &opts.name
                && &info.package_name != name
            {
                false
            } else if let Some(version) = &opts.version
                && &info.package_version != version
            {
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>();

    let courier = Courier::new(opts.api_url, opts.api_token)?;

    println!("Found {} matching units:", matching_units.len());
    for unit in matching_units {
        let info = unit.info().clone();
        println!("\n  UnitInfo: {{");
        println!("    name: {:?}", info.package_name);
        println!("    version: {:?}", info.package_version);
        println!("    unit hash: {:?}", info.unit_hash);

        match unit {
            hurry::cargo::UnitPlan::LibraryCrate(library_crate_unit_plan) => {
                println!("    library crate: {{");
                println!("      src path: {:?}", library_crate_unit_plan.src_path);
                println!("      outputs: [");
                for output in library_crate_unit_plan.outputs {
                    println!("        {output:?}");
                }
                println!("      ]");
                println!("    }}");
            }
            hurry::cargo::UnitPlan::BuildScriptCompilation(build_script_compilation_unit_plan) => {
                println!("    build script compilation: {{");
                println!(
                    "      src path: {:?}",
                    build_script_compilation_unit_plan.src_path
                );
                println!("    }}");
            }
            hurry::cargo::UnitPlan::BuildScriptExecution(build_script_execution_unit_plan) => {
                println!("    build script execution: {{");
                println!(
                    "      build script program name: {:?}",
                    build_script_execution_unit_plan.build_script_program_name
                );
                println!("    }}");
            }
        }

        let key = info.unit_hash.into();
        let mut cached = courier
            .cargo_cache_restore(CargoRestoreRequest::new([&key], host_glibc_version()?))
            .await?;

        match cached.take(&key) {
            Some(cached) => {
                println!("\n  Cached: {cached:?}");
            }
            None => {
                println!("\n  Cached: None");
            }
        }
        println!("  }}");
    }

    Ok(())
}
