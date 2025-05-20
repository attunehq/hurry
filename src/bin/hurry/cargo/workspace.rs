use std::{io::BufRead, process::Command};

use anyhow::Context;
use camino::Utf8PathBuf;
use cargo_metadata::Metadata;
use tracing::{instrument, trace};

#[derive(Debug)]
pub struct Workspace {
    pub metadata: Metadata,
}

impl Workspace {
    #[instrument(level = "debug")]
    pub fn open() -> anyhow::Result<Self> {
        // TODO: Should these be parsed higher up and passed in?
        let mut args = std::env::args().skip_while(|val| !val.starts_with("--manifest-path"));

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
    pub fn source_files(&self) -> impl Iterator<Item = Utf8PathBuf> {
        let packages = self.metadata.workspace_packages();
        trace!(?packages, "workspace packages");
        packages.into_iter().flat_map(|package| {
            trace!(?package, "getting source files of package");
            package.targets.iter().flat_map(|target| {
                let target_root = target.src_path.clone();
                // TODO: Rather than shelling out to `rustc`, should we just
                // mimic the logic? This is quite complicated (involves macro
                // expansion, etc.) but might be faster.
                //
                // Alternatively, maybe we should do an approximation? For
                // example, Cargo's own logic for determining "relevant" source
                // files approximates by just looking in the directory. See:
                // https://docs.rs/cargo/latest/cargo/sources/path/struct.PathSource.html#method.list_files
                trace!(?target, "invoking rustc dep-info");
                let dep_info = Command::new("rustc")
                    .args(vec!["--emit=dep-info=-", target_root.as_str()])
                    .output()
                    .expect("could not run rustc crate source file loader")
                    .stdout;
                trace!(dep_info = ?String::from_utf8_lossy(&dep_info), "invoked rustc dep-info");
                // TODO: How do we handle directory paths that have newlines and
                // colons in them? I've tested what `rustc` does in this case
                // and it doesn't seem to escape. So theoretically you could
                // construct a crate source file that appears to be two
                // different source files.
                let parsed = dep_info
                    .lines()
                    .filter_map(|l| {
                        let l = l.expect("could not read rustc dep-info");
                        // Handle `# env-dep:` lines.
                        if l.starts_with("# ") || l.is_empty() {
                            return None;
                        }
                        Some(l.trim_end_matches(':').into())
                    })
                    .collect::<Vec<_>>();
                trace!(?parsed, "parsed rustc dep-info");
                parsed
            })
        })
    }

    #[instrument(level = "debug", skip(self))]
    pub fn output_dir(&self) -> &Utf8PathBuf {
        &self.metadata.target_directory
    }
}
