#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureClass {
    Parse,
    Validation,
    Runtime,
    Dependency,
}

impl FailureClass {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Parse => 2,
            Self::Validation => 3,
            Self::Runtime => 4,
            Self::Dependency => 5,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Validation => "validation",
            Self::Runtime => "runtime",
            Self::Dependency => "dependency",
        }
    }

    pub fn default_try_guidance(self) -> &'static str {
        match self {
            Self::Parse => "run 'sce --help' to see valid usage.",
            Self::Validation => {
                "run the command-specific '--help' usage shown in the error and retry."
            }
            Self::Runtime => "inspect the runtime diagnostic details, then retry.",
            Self::Dependency => {
                "verify required runtime dependencies and environment setup, then retry."
            }
        }
    }
}

#[derive(Debug)]
pub struct ClassifiedError {
    class: FailureClass,
    code: &'static str,
    message: String,
    hint: Option<String>,
}

impl ClassifiedError {
    pub fn parse(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Parse,
            code: "SCE-ERR-PARSE",
            message: message.into(),
            hint: None,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Validation,
            code: "SCE-ERR-VALIDATION",
            message: message.into(),
            hint: None,
        }
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Runtime,
            code: "SCE-ERR-RUNTIME",
            message: message.into(),
            hint: None,
        }
    }

    pub fn dependency(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Dependency,
            code: "SCE-ERR-DEPENDENCY",
            message: message.into(),
            hint: None,
        }
    }

    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn class(&self) -> FailureClass {
        self.class
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// Builds a `runtime` error from converted `anyhow::Error` text, splitting
    /// a trailing `" Try: ..."` suffix into an explicit hint when present.
    ///
    /// This is the one intentional, construction-time exception to deciding
    /// remediation by inspecting message text: it exists at the anyhow/
    /// `ClassifiedError` conversion boundary specifically to avoid doubling
    /// `Try:` guidance for the dozen or so call sites that already bake
    /// remediation into their `anyhow::Error`/`String` text upstream.
    pub fn from_anyhow_text(message: impl Into<String>) -> Self {
        let message = message.into();
        match message.rsplit_once(" Try: ") {
            Some((body, hint)) => Self::runtime(body.to_string()).with_hint(hint.to_string()),
            None => Self::runtime(message),
        }
    }
}

impl std::fmt::Display for ClassifiedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ClassifiedError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_default_hint_to_none() {
        assert_eq!(ClassifiedError::parse("x").hint(), None);
        assert_eq!(ClassifiedError::validation("x").hint(), None);
        assert_eq!(ClassifiedError::runtime("x").hint(), None);
        assert_eq!(ClassifiedError::dependency("x").hint(), None);
    }

    #[test]
    fn with_hint_sets_an_explicit_hint() {
        let error = ClassifiedError::parse("x").with_hint("y");

        assert_eq!(error.hint(), Some("y"));
        assert_eq!(error.message(), "x");
    }

    #[test]
    fn with_hint_does_not_change_class_code_or_exit_code() {
        let error = ClassifiedError::parse("x");
        let hinted = ClassifiedError::parse("x").with_hint("y");

        assert_eq!(error.class(), hinted.class());
        assert_eq!(error.code(), hinted.code());
        assert_eq!(error.class().exit_code(), hinted.class().exit_code());
    }

    #[test]
    fn from_anyhow_text_splits_a_trailing_try_suffix_into_a_hint() {
        let error = ClassifiedError::from_anyhow_text("something failed. Try: rerun the command.");

        assert_eq!(error.message(), "something failed.");
        assert_eq!(error.hint(), Some("rerun the command."));
        assert_eq!(error.class(), FailureClass::Runtime);
    }

    #[test]
    fn from_anyhow_text_leaves_a_message_without_a_try_suffix_unhinted() {
        let error = ClassifiedError::from_anyhow_text("something failed with no guidance.");

        assert_eq!(error.message(), "something failed with no guidance.");
        assert_eq!(error.hint(), None);
    }

    #[test]
    fn from_anyhow_text_splits_on_the_last_try_occurrence() {
        let error = ClassifiedError::from_anyhow_text(
            "outer context: inner failed. Try: retry inner. Try: then retry outer.",
        );

        assert_eq!(
            error.message(),
            "outer context: inner failed. Try: retry inner."
        );
        assert_eq!(error.hint(), Some("then retry outer."));
    }
}
