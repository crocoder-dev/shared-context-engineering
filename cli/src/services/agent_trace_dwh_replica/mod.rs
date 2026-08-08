//! Agent Trace DWH sync replica boundary.
//!
//! This module owns the canonical repository-scoped bridge lock proving
//! exactly one process owns the local `agent-trace-sync.db` Turso Sync
//! replica at a time. It never touches the multiprocess-WAL source capture
//! database (`crate::services::agent_trace_db`) and does not itself open a
//! Turso Sync connection: that is a later boundary built on top of the lock
//! established here.

mod lock;

#[allow(unused_imports)]
pub use lock::{BridgeLock, BridgeLockError};
