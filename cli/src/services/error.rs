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

    pub fn code(self) -> &'static str {
        match self {
            Self::Parse => "SCE-ERR-PARSE",
            Self::Validation => "SCE-ERR-VALIDATION",
            Self::Runtime => "SCE-ERR-RUNTIME",
            Self::Dependency => "SCE-ERR-DEPENDENCY",
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

/// The typed origin of an automatic synchronization failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutomaticSyncFailureKind {
    Authentication,
    ControlPlane,
    Stream,
    Runtime,
}

/// Catalog of expected, deliberately-explained failures presented to the user
/// as a friendly diagnostic instead of a technical error chain.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::enum_variant_names)]
pub enum UserError {
    #[allow(dead_code)]
    NotAuthenticated,
    NotGitRepository,
    NotGitRemote {
        remote_name: String,
    },
    AutomaticSyncFailed {
        failure_kind: AutomaticSyncFailureKind,
        reason: String,
    },
}

impl UserError {
    pub fn class(&self) -> FailureClass {
        match self {
            Self::NotAuthenticated
            | Self::NotGitRepository
            | Self::NotGitRemote { .. }
            | Self::AutomaticSyncFailed { .. } => FailureClass::Runtime,
        }
    }

    #[allow(dead_code)]
    pub fn key(&self) -> &'static str {
        match self {
            Self::NotAuthenticated => "auth.not_authenticated",
            Self::NotGitRepository => "setup.not_git_repository",
            Self::NotGitRemote { .. } => "setup.not_git_remote",
            Self::AutomaticSyncFailed { failure_kind, .. } => match failure_kind {
                AutomaticSyncFailureKind::Authentication => "sync.automatic.authentication_failed",
                AutomaticSyncFailureKind::ControlPlane => "sync.automatic.control_plane_failed",
                AutomaticSyncFailureKind::Stream => "sync.automatic.stream_failed",
                AutomaticSyncFailureKind::Runtime => "sync.automatic.runtime_failed",
            },
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotAuthenticated => {
                "You are not logged in. Please log in using the `sce auth login` command."
                    .to_string()
            }
            Self::NotGitRepository => {
                "The target directory is not a Git repository. Please run `git init`, then retry."
                    .to_string()
            }
            Self::NotGitRemote { remote_name } => format!(
                "The Git repository has no configured URL for remote '{remote_name}'. Please run `git remote add <remote> <url>`, then retry."
            ),
            Self::AutomaticSyncFailed {
                failure_kind: AutomaticSyncFailureKind::Authentication,
                ..
            } => "Automatic synchronization failed: authentication is required. Run `sce auth login`, then manually retry with `sce sync`.".to_string(),
            Self::AutomaticSyncFailed {
                failure_kind: AutomaticSyncFailureKind::ControlPlane,
                reason,
            } => format!(
                "Automatic synchronization failed: {reason}. Check control-plane connectivity and availability, then manually retry with `sce sync`."
            ),
            Self::AutomaticSyncFailed {
                failure_kind: AutomaticSyncFailureKind::Stream,
                reason,
            } => format!(
                "Automatic synchronization failed: {reason}. Check Agent Trace data and connectivity, then manually retry with `sce sync`."
            ),
            Self::AutomaticSyncFailed {
                failure_kind: AutomaticSyncFailureKind::Runtime,
                reason,
            } => format!(
                "Automatic synchronization failed: {reason}. Check the local repository and Agent Trace configuration, then manually retry with `sce sync`."
            ),
        }
    }

    #[allow(dead_code)]
    pub fn automatic_sync_failure_kind(&self) -> Option<AutomaticSyncFailureKind> {
        match self {
            Self::AutomaticSyncFailed { failure_kind, .. } => Some(*failure_kind),
            Self::NotAuthenticated | Self::NotGitRepository | Self::NotGitRemote { .. } => None,
        }
    }

    #[allow(dead_code)]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::AutomaticSyncFailed { reason, .. } => Some(reason),
            Self::NotAuthenticated | Self::NotGitRepository | Self::NotGitRemote { .. } => None,
        }
    }
}

impl std::fmt::Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

/// Typed CLI-boundary error separating expected, user-facing failures from
/// internal failures whose technical `anyhow` source is preserved for
/// observability and diagnostic rendering.
#[derive(Debug)]
pub enum CliError {
    #[allow(dead_code)]
    User {
        error: UserError,
        source: Option<anyhow::Error>,
    },
    Internal {
        class: FailureClass,
        source: anyhow::Error,
    },
}

impl CliError {
    #[allow(dead_code)]
    pub fn user(error: UserError) -> Self {
        Self::User {
            error,
            source: None,
        }
    }

    #[allow(dead_code)]
    pub fn user_with_source(error: UserError, source: impl Into<anyhow::Error>) -> Self {
        Self::User {
            error,
            source: Some(source.into()),
        }
    }

    pub fn internal(class: FailureClass, source: impl Into<anyhow::Error>) -> Self {
        Self::Internal {
            class,
            source: source.into(),
        }
    }

    pub fn runtime(source: impl Into<anyhow::Error>) -> Self {
        Self::internal(FailureClass::Runtime, source)
    }

    pub fn dependency(source: impl Into<anyhow::Error>) -> Self {
        Self::internal(FailureClass::Dependency, source)
    }

    /// Compatibility helper for parse failures still expressed as a plain
    /// message rather than a live `anyhow::Error` source.
    pub fn parse(message: impl Into<String>) -> Self {
        Self::internal(FailureClass::Parse, anyhow::Error::msg(message.into()))
    }

    /// Compatibility helper for validation failures still expressed as a
    /// plain message rather than a live `anyhow::Error` source.
    pub fn validation(message: impl Into<String>) -> Self {
        Self::internal(FailureClass::Validation, anyhow::Error::msg(message.into()))
    }

    pub fn class(&self) -> FailureClass {
        match self {
            Self::User { error, .. } => error.class(),
            Self::Internal { class, .. } => *class,
        }
    }

    pub fn code(&self) -> &'static str {
        self.class().code()
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User { error, .. } => write!(f, "{error}"),
            Self::Internal { source, .. } => write!(f, "{source:#}"),
        }
    }
}

impl std::error::Error for CliError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_class_code_maps_to_stable_sce_err_strings() {
        assert_eq!(FailureClass::Parse.code(), "SCE-ERR-PARSE");
        assert_eq!(FailureClass::Validation.code(), "SCE-ERR-VALIDATION");
        assert_eq!(FailureClass::Runtime.code(), "SCE-ERR-RUNTIME");
        assert_eq!(FailureClass::Dependency.code(), "SCE-ERR-DEPENDENCY");
    }

    #[test]
    fn user_error_not_authenticated_classifies_as_runtime() {
        let error = CliError::user(UserError::NotAuthenticated);

        assert_eq!(error.class(), FailureClass::Runtime);
        assert_eq!(error.code(), "SCE-ERR-RUNTIME");
        assert_eq!(UserError::NotAuthenticated.key(), "auth.not_authenticated");
        assert!(error.to_string().contains("You are not logged in"));
    }

    #[test]
    fn user_with_source_preserves_technical_source() {
        let error = CliError::user_with_source(
            UserError::NotAuthenticated,
            anyhow::anyhow!("missing credentials"),
        );

        let CliError::User { source, .. } = &error else {
            panic!("expected CliError::User");
        };
        assert_eq!(
            source.as_ref().expect("source preserved").to_string(),
            "missing credentials"
        );
        assert!(error.to_string().contains("You are not logged in"));
    }

    #[test]
    fn internal_error_renders_the_real_anyhow_chain() {
        let source = anyhow::anyhow!("root cause").context("failed to do the thing");
        let error = CliError::internal(FailureClass::Dependency, source);

        assert_eq!(error.class(), FailureClass::Dependency);
        assert_eq!(error.code(), "SCE-ERR-DEPENDENCY");
        assert_eq!(
            error.to_string(),
            "failed to do the thing: root cause".to_string()
        );
    }

    #[test]
    fn runtime_and_dependency_constructors_classify_correctly() {
        assert_eq!(
            CliError::runtime(anyhow::anyhow!("boom")).class(),
            FailureClass::Runtime
        );
        assert_eq!(
            CliError::dependency(anyhow::anyhow!("boom")).class(),
            FailureClass::Dependency
        );
    }

    #[test]
    fn parse_and_validation_compatibility_helpers_wrap_plain_messages() {
        let parse_error = CliError::parse("bad usage");
        let validation_error = CliError::validation("bad value");

        assert_eq!(parse_error.class(), FailureClass::Parse);
        assert_eq!(parse_error.to_string(), "bad usage");
        assert_eq!(validation_error.class(), FailureClass::Validation);
        assert_eq!(validation_error.to_string(), "bad value");
    }
}
