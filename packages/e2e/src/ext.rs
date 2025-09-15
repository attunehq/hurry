//! Extension traits

use cargo_metadata::{Artifact, Message, PackageId};

pub trait MessageIterExt<'a> {
    /// Iterate over the third-party artifacts in the set of messages.
    fn thirdparty_artifacts(self) -> impl Iterator<Item = &'a Artifact>;
}

impl<'a, I> MessageIterExt<'a> for I
where
    I: Iterator<Item = &'a Message>,
{
    fn thirdparty_artifacts(self) -> impl Iterator<Item = &'a Artifact> {
        self.into_iter().filter_map(|m| match &m {
            // This is not the full set of criteria that determines if an
            // artifact is third-party, but it's a good enough approximation for
            // now; as more precision is needed add more criteria.
            Message::CompilerArtifact(artifact) => {
                if artifact.package_id.repr.starts_with("registry+") {
                    Some(artifact)
                } else {
                    None
                }
            }
            _ => None,
        })
    }
}

pub trait ArtifactIterExt<'a> {
    /// Iterate over the package ID of each artifact.
    fn package_ids(self) -> impl Iterator<Item = &'a PackageId>;

    /// Iterate over the package ID and freshness of each artifact.
    fn freshness(self) -> impl Iterator<Item = (&'a PackageId, bool)>;
}

impl<'a, I> ArtifactIterExt<'a> for I
where
    I: Iterator<Item = &'a Artifact>,
{
    fn package_ids(self) -> impl Iterator<Item = &'a PackageId> {
        self.map(|artifact| &artifact.package_id)
    }

    fn freshness(self) -> impl Iterator<Item = (&'a PackageId, bool)> {
        self.map(|artifact| (&artifact.package_id, artifact.fresh))
    }
}
