//! A single embedded integration asset to be installed into a repository.

use std::borrow::Cow;

/// An embedded asset destined for a repository-relative path within an
/// integration target's install root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IntegrationAsset {
    pub(crate) relative_path: String,
    pub(crate) bytes: Cow<'static, [u8]>,
}
