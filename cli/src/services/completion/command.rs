use crate::app::AppContext;
use crate::services::completion;

pub struct CompletionCommand {
    pub request: completion::CompletionRequest,
}

impl CompletionCommand {
    pub fn execute(&self, _context: &AppContext) -> String {
        completion::render_completion(self.request)
    }
}
