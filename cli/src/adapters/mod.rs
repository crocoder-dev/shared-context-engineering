//! Adapters layer: inbound and outbound implementations of application ports.
//!
//! Adapters may depend on `crate::application` and, transitionally, on
//! `crate::services`. See `context/architecture.md` for the full
//! dependency-direction rules.

mod inbound;
mod outbound;
