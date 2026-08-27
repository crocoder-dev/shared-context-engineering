//! Model-based testing harness connecting the verified
//! `spec/mutation_cursor.qnt` model to the pure Rust refinement in
//! `super::protocol`/`super::types` via Quint Connect.
//!
//! Test-only (`#[cfg(test)]`, gated from `mutation_trace/mod.rs`): no
//! production code depends on this module, and it introduces no Git,
//! database, filesystem, environment, network, async, or lock I/O of its
//! own — every state transition is delegated to `super::protocol`'s pure
//! functions.

mod driver;
mod model;
mod tests;
