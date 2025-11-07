use clap::Args;
use clients::{Courier, Token, courier::v1::cache::CargoRestoreRequest};
use color_eyre::Result;
use derive_more::Debug;
use hurry::cargo::{CargoBuildArguments, Profile, Workspace};
use url::Url;

#[derive(Clone, Args, Debug)]
pub struct Options {
    /// Base URL for the Courier instance.
    #[arg(
        long = "courier-url",
        env = "HURRY_COURIER_URL",
        default_value = "https://courier.staging.corp.attunehq.com"
    )]
    #[debug("{courier_url}")]
    courier_url: Url,

    /// Authentication token for the Courier instance.
    #[arg(long = "courier-token", env = "HURRY_COURIER_TOKEN")]
    courier_token: Token,

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
    let artifact_plan = workspace.artifact_plan(&Profile::Debug, args).await?;
    let matching_artifacts = artifact_plan
        .artifacts
        .into_iter()
        .filter(|artifact| {
            if let Some(name) = &opts.name
                && &artifact.package_name != name
            {
                false
            } else if let Some(version) = &opts.version
                && &artifact.package_version != version
            {
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>();

    let courier = Courier::new(opts.courier_url, opts.courier_token)?;

    println!("Found {} matching artifacts:", matching_artifacts.len());
    for artifact in matching_artifacts {
        println!("\n  ArtifactKey:");
        println!("    name: {:?}", artifact.package_name);
        println!("    version: {:?}", artifact.package_version);
        println!("    files:");
        println!("      library crate: [");
        for file in artifact.lib_files {
            println!("        {}", file);
        }
        println!("      ]");

        match artifact.build_script_files {
            Some(files) => {
                println!("      build script: [");
                println!("        compiled files: {}", files.compiled_dir);
                println!("        execution output: {}", files.output_dir);
                println!("      ]");
            }
            None => {
                println!("      build script: N/A");
            }
        }
        println!("    unit hashes:");
        println!(
            "      library crate: {:?}",
            artifact.library_crate_compilation_unit_hash
        );
        println!(
            "      build script compilation: {:?}",
            artifact
                .build_script_compilation_unit_hash
                .clone()
                .unwrap_or(String::from("N/A"))
        );
        println!(
            "      build script execution: {:?}",
            artifact
                .build_script_execution_unit_hash
                .clone()
                .unwrap_or(String::from("N/A"))
        );

        let cached = courier
            .cargo_cache_restore(
                CargoRestoreRequest::builder()
                    .package_name(artifact.package_name)
                    .package_version(artifact.package_version)
                    .target(artifact_plan.target.clone())
                    .library_crate_compilation_unit_hash(
                        artifact.library_crate_compilation_unit_hash,
                    )
                    .maybe_build_script_compilation_unit_hash(
                        artifact.build_script_compilation_unit_hash,
                    )
                    .maybe_build_script_execution_unit_hash(
                        artifact.build_script_execution_unit_hash,
                    )
                    .build(),
            )
            .await?;

        match cached {
            Some(mut cached) => {
                println!("\n  Cached:");
                println!("    Files ({}):", cached.artifacts.len());
                cached.artifacts.sort_by(|f1, f2| f1.path.cmp(&f2.path));
                for file in cached.artifacts {
                    println!("      Path: {}", file.path);
                    println!("        Mtime: {}", file.mtime_nanos);
                    println!("        Executable: {}", file.executable);
                    println!("        Key: {}", file.object_key);
                }
            }
            None => {
                println!("\n  Cached: None");
            }
        }
    }

    Ok(())
}
