use anyhow::Context;
use camino::Utf8PathBuf;
use cargo_metadata::Metadata;
use tracing::{instrument, trace};
use walkdir::WalkDir;

#[derive(Debug)]
pub struct Workspace {
    pub metadata: Metadata,
}

impl Workspace {
    #[instrument(level = "debug")]
    pub fn open() -> anyhow::Result<Self> {
        // TODO: Should these be parsed higher up and passed in?
        let mut args = std::env::args().skip_while(|val| !val.starts_with("--manifest-path"));

        // TODO: Maybe we should just replicate this logic and perform it
        // statically using filesystem operations instead of shelling out? This
        // costs something on the order of 200ms, which is not _terrible_ but
        // feels much slower than if we just did our own filesystem reads.
        let mut cmd = cargo_metadata::MetadataCommand::new();
        match args.next() {
            Some(ref p) if p == "--manifest-path" => {
                cmd.manifest_path(args.next().expect("--manifest-path requires a value"));
            }
            Some(p) => {
                cmd.manifest_path(p.trim_start_matches("--manifest-path="));
            }
            None => {}
        }

        let metadata = cmd.exec().context("could not read cargo metadata")?;
        trace!(?metadata, "cargo metadata");
        Ok(Self { metadata })
    }

    #[instrument(level = "debug", skip(self))]
    pub fn source_files(&self) -> impl Iterator<Item = Result<walkdir::DirEntry, walkdir::Error>> {
        let packages = self.metadata.workspace_packages();
        trace!(?packages, "workspace packages");
        packages.into_iter().flat_map(|package| {
            trace!(?package, "getting source files of package");
            // TODO: The technically correct way to calculate the source files
            // of a target is to shell out to `rustc --emit=dep-info=-` with the
            // correct `rustc` flags (e.g. the `--extern` flags, which are
            // required to import macros defined in dependency crates which
            // might be used to add more source files to the target) and the
            // module root.
            //
            // Unfortunately, this is quite annoying:
            // - We need to get the correct flags for `rustc`. I'm not totally
            //   sure how to do this - I think we should be able to by parsing
            //   the output messages from `cargo build`? Or maybe we should be
            //   able to reconstruct them from `cargo metadata`? Or maybe we can
            //   use `cargo rustc`?
            // - Running `rustc` takes quite a long time. Maybe we can improve
            //   this by using some background daemon / file change notification
            //   trickery? Maybe we can run things in parallel?
            //
            // Instead, we approximate the source files in a module by taking
            // all the files in the folder of the crate root source file. This
            // is also the approximation that Cargo uses to determine "relevant"
            // files.
            //
            // TODO: We have not yet implemented Cargo's approximation logic
            // that handles things like `.gitignore`, `package.include`,
            // `package.exclude`, etc.
            //
            // See also:
            // - `dep-info` files:
            //   https://doc.rust-lang.org/cargo/reference/build-cache.html#dep-info-files
            // - `cargo build` output messages:
            //   https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages
            // - Cargo's source file discovery logic:
            //   https://docs.rs/cargo/latest/cargo/sources/path/struct.PathSource.html#method.list_files
            package.targets.iter().flat_map(|target| {
                let target_root = target.src_path.clone();
                let target_root_folder = target_root
                    .parent()
                    .expect("module root should be a file in a folder");
                WalkDir::new(target_root_folder).into_iter()
            })
        })
    }

    #[instrument(level = "debug", skip(self))]
    pub fn output_dir(&self) -> &Utf8PathBuf {
        &self.metadata.target_directory
    }
}
