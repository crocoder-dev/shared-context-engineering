//! Agent Trace database discovery, readiness probing, and stats services.

pub mod command;
pub mod discovery;
pub mod render_list;
pub mod render_status;
pub mod render_status_all;
pub mod shell;
pub mod stats;
pub mod status;
pub mod status_all;

pub const NAME: &str = "trace";

#[allow(unused_imports)]
pub use discovery::{
    discover_agent_trace_dbs, resolve_agent_trace_db_identifier, DiscoveredAgentTraceDb, Readiness,
    ResolveAgentTraceDbError,
};

use crate::services::output_format::OutputFormat;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceSubcommandRequest {
    DbList { format: OutputFormat },
    DbShell { identifier: Option<String> },
    Status { all: bool, format: OutputFormat },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceRequest {
    pub subcommand: TraceSubcommandRequest,
}
