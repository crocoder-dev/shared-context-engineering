use crate::services::completion;

pub struct CompletionCommand {
    pub request: completion::CompletionRequest,
}

impl CompletionCommand {
    pub fn execute<C>(&self, _context: &C) -> String {
        completion::render_completion(self.request)
    }
}
