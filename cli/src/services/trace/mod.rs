//! Agent Trace database discovery, readiness probing, and stats services.

pub mod command;
pub mod discovery;
pub mod render_list;

pub const NAME: &str = "trace";

#[allow(unused_imports)]
pub use discovery::{discover_agent_trace_dbs, DiscoveredAgentTraceDb, Readiness};

use crate::services::output_format::OutputFormat;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceSubcommandRequest {
    DbList { format: OutputFormat },
    Status { all: bool, format: OutputFormat },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceRequest {
    pub subcommand: TraceSubcommandRequest,
}
