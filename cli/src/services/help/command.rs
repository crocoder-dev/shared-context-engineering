use std::borrow::Cow;

use crate::app::AppContext;

pub struct HelpCommand;

pub struct HelpTextCommand {
    pub name: String,
    pub text: String,
}

impl HelpTextCommand {
    pub fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.name.as_str())
    }

    pub fn execute(&self, _context: &AppContext) -> String {
        self.text.clone()
    }
}
