//! A single embedded integration asset to be installed into a repository.

/// An embedded asset destined for a repository-relative path within an
/// integration target's install root.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)] // consumed starting with the IntegrationAssetCatalog port (T02)
pub(crate) struct IntegrationAsset {
    pub(crate) relative_path: String,
    pub(crate) bytes: &'static [u8],
}
