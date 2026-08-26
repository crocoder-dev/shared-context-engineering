//! Pure Rust domain representation for the refinement of the verified
//! `spec/mutation_cursor.qnt` mutation-cursor protocol.
//!
//! This module defines the protocol's domain/state types, pure accessors,
//! `prepare`/`commitAttempt` transition logic for all four boundary kinds,
//! attribution derivation, mutation-event materialization, snapshot-failure
//! taint, and database-failure external taint. Scope abandonment and
//! recovery are not yet implemented. No Git, database, filesystem,
//! environment, network, async, or lock I/O is performed here.
//! The module is not yet wired into any hook, command, or database call
//! site: that integration, along with the `coordinator.rs` (imperative
//! shell), `git_snapshot.rs` (isolated Git snapshot capture), and `store.rs`
//! (DB-backed CAS persistence) seams the target architecture will grow into,
//! is left for later work.

pub mod protocol;
pub mod types;

#[cfg(test)]
mod tests;
