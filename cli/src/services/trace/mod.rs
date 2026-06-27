//! Agent Trace database discovery, readiness probing, and stats services.
//!
//! Module is registered but not yet wired into the CLI command surface; the
//! `sce trace` command group is introduced in a later plan task.

pub mod discovery;

#[allow(unused_imports)]
pub use discovery::{discover_agent_trace_dbs, DiscoveredAgentTraceDb, Readiness};
