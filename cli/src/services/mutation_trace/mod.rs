//! Pure Rust refinement of the verified `spec/mutation_cursor.qnt` mutation-
//! cursor protocol.
//!
//! This module represents the protocol's state and pure transitions with no
//! Git, database, filesystem, environment, network, or lock I/O. It is not
//! yet wired into any hook, command, or database call site: that
//! integration, along with the `coordinator.rs` (imperative shell),
//! `git_snapshot.rs` (isolated Git snapshot capture), and `store.rs`
//! (DB-backed CAS persistence) seams the target architecture will grow into,
//! is left to a later plan.

pub mod types;

#[cfg(test)]
mod tests;
