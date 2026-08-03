//! Adapters layer: inbound and outbound implementations of application ports.
//!
//! Adapters may depend on `crate::application` and, transitionally, on
//! `crate::services`. See `context/architecture.md` for the full
//! dependency-direction rules.

pub(crate) mod inbound;
pub(crate) mod outbound;
