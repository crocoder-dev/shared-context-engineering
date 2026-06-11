use std::borrow::Cow;

pub struct HelpCommand;

pub struct HelpTextCommand {
    pub name: String,
    pub text: String,
}

impl HelpTextCommand {
    pub fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.name.as_str())
    }

    pub fn execute<C>(&self, _context: &C) -> String {
        self.text.clone()
    }
}
