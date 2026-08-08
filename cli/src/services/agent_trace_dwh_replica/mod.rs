//! Agent Trace DWH sync replica boundary.
//!
//! This module owns the canonical repository-scoped bridge lock proving
//! exactly one process owns the local `agent-trace-sync.db` Turso Sync
//! replica at a time, and the [`AgentTraceDwhReplica`] type that is the sole
//! owner of a Turso Sync builder for that replica. It never touches the
//! multiprocess-WAL source capture database
//! (`crate::services::agent_trace_db`), never enables that database's
//! `experimental_multiprocess_wal` flag, and never provisions the DWH schema
//! locally: it only verifies the schema an already-bootstrapped replica
//! reports.

mod lock;
mod replica;

#[allow(unused_imports)]
pub use lock::{BridgeLock, BridgeLockError};
#[allow(unused_imports)]
pub use replica::{AgentTraceDwhReplica, AgentTraceDwhReplicaConfig, AgentTraceDwhReplicaError};
