use crate::services::error::ClassifiedError;
use crate::services::trace::{TraceRequest, TraceSubcommandRequest};

pub struct TraceCommand {
    pub request: TraceRequest,
}

impl TraceCommand {
    #[allow(clippy::unnecessary_wraps)]
    pub fn execute<C>(&self, _context: &C) -> Result<String, ClassifiedError> {
        let label = match &self.request.subcommand {
            TraceSubcommandRequest::DbList { .. } => "trace db list",
            TraceSubcommandRequest::Status { all: true, .. } => "trace status --all",
            TraceSubcommandRequest::Status { all: false, .. } => "trace status",
        };

        Ok(format!("sce {label}: not implemented"))
    }
}
