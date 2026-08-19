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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserFacingPresentation {
    message: String,
    reason: Option<String>,
}

impl UserFacingPresentation {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            reason: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    #[allow(dead_code)]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

#[derive(Debug)]
pub struct ClassifiedError {
    class: FailureClass,
    code: &'static str,
    message: String,
    user_facing_presentation: Option<UserFacingPresentation>,
}

impl ClassifiedError {
    pub fn parse(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Parse,
            code: "SCE-ERR-PARSE",
            message: message.into(),
            user_facing_presentation: None,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Validation,
            code: "SCE-ERR-VALIDATION",
            message: message.into(),
            user_facing_presentation: None,
        }
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Runtime,
            code: "SCE-ERR-RUNTIME",
            message: message.into(),
            user_facing_presentation: None,
        }
    }

    pub fn dependency(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Dependency,
            code: "SCE-ERR-DEPENDENCY",
            message: message.into(),
            user_facing_presentation: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_user_facing_message(mut self, message: impl Into<String>) -> Self {
        self.user_facing_presentation = Some(UserFacingPresentation::new(message));
        self
    }

    pub fn with_user_facing_presentation(mut self, presentation: UserFacingPresentation) -> Self {
        self.user_facing_presentation = Some(presentation);
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

    pub fn user_facing_presentation(&self) -> Option<&UserFacingPresentation> {
        self.user_facing_presentation.as_ref()
    }
}

impl std::fmt::Display for ClassifiedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ClassifiedError {}
