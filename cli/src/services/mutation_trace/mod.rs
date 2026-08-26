//! Pure Rust domain representation for the refinement of the verified
//! `spec/mutation_cursor.qnt` mutation-cursor protocol.
//!
//! This module currently defines the protocol's domain/state types and pure
//! accessors; transition logic (`prepare`/`commitAttempt` and the
//! attribution/failure/recovery actions) is not yet implemented. No Git,
//! database, filesystem, environment, network, async, or lock I/O is
//! performed here. The module is not yet wired into any hook, command, or
//! database call site: that integration, along with the `coordinator.rs`
//! (imperative shell), `git_snapshot.rs` (isolated Git snapshot capture),
//! and `store.rs` (DB-backed CAS persistence) seams the target architecture
//! will grow into, is left for later work.

pub mod types;

#[cfg(test)]
mod tests;
