use crate::services::error::ClassifiedError;
use crate::services::trace::discovery::discover_agent_trace_dbs;
use crate::services::trace::render_list;
use crate::services::trace::{TraceRequest, TraceSubcommandRequest};

pub struct TraceCommand {
    pub request: TraceRequest,
}

impl TraceCommand {
    pub fn execute<C>(&self, _context: &C) -> Result<String, ClassifiedError> {
        match &self.request.subcommand {
            TraceSubcommandRequest::DbList { format } => {
                let databases = discover_agent_trace_dbs()
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))?;
                render_list::render(&databases, *format)
                    .map_err(|error| ClassifiedError::runtime(format!("{error:#}")))
            }
            TraceSubcommandRequest::Status { all: true, .. } => {
                Ok(String::from("sce trace status --all: not implemented"))
            }
            TraceSubcommandRequest::Status { all: false, .. } => {
                Ok(String::from("sce trace status: not implemented"))
            }
        }
    }
}
