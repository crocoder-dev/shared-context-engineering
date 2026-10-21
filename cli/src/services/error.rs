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

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::enum_variant_names)]
pub enum UserError {
    NotAuthenticated,
    NotGitRepository,
    NotGitRemote,
    AuthStorageUnavailable(AuthStorageFailure),
    LoginAuthStorageUnavailable(AuthStorageFailure),
    AuthDisabledByConfig(AuthPolicyConfigFile),
    UnexpectedFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthPolicyConfigFile {
    Global,
    Repository,
    ConfigFlag,
    ConfigEnv,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthStorageFailure {
    CredentialAccess,
    DatabaseLocked,
    Database,
}

impl UserError {
    pub fn class(&self) -> FailureClass {
        match self {
            Self::NotAuthenticated
            | Self::NotGitRepository
            | Self::NotGitRemote
            | Self::AuthStorageUnavailable(_)
            | Self::LoginAuthStorageUnavailable(_)
            | Self::AuthDisabledByConfig(_)
            | Self::UnexpectedFailure => FailureClass::Runtime,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Self::NotAuthenticated => "auth.not_authenticated",
            Self::NotGitRepository => "setup.not_git_repository",
            Self::NotGitRemote { .. } => "setup.not_git_remote",
            Self::AuthStorageUnavailable(_) | Self::LoginAuthStorageUnavailable(_) => {
                "auth.storage_unavailable"
            }
            Self::AuthDisabledByConfig(_) => "auth.disabled_by_config",
            Self::UnexpectedFailure => "general.unexpected_failure",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::NotAuthenticated => {
                "You are not logged in. Please log in using the `sce auth login` command."
            }
            Self::NotGitRepository => {
                "The target directory is not a Git repository. Please run `git init`, then retry."
            }
            Self::NotGitRemote => {
                "The Git repository has no configured remote URL. Please run `git remote add <name> <url>`, then retry."
            }
            Self::AuthStorageUnavailable(failure) => auth_storage_unavailable_message(
                CURRENT_CREDENTIAL_STORE_PLATFORM,
                AuthStorageOperation::Command,
                *failure,
            ),
            Self::LoginAuthStorageUnavailable(failure) => auth_storage_unavailable_message(
                CURRENT_CREDENTIAL_STORE_PLATFORM,
                AuthStorageOperation::Login,
                *failure,
            ),
            Self::AuthDisabledByConfig(file) => auth_disabled_by_config_message(*file),
            Self::UnexpectedFailure => {
                "An unexpected error occurred. Check the log files for more details."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredentialStorePlatform {
    Macos,
    Linux,
    Windows,
    Other,
}

impl CredentialStorePlatform {
    pub(crate) const fn store_name(self) -> &'static str {
        match self {
            Self::Macos => "macOS Keychain",
            Self::Linux => "the system keyring (Secret Service)",
            Self::Windows => "Windows Credential Manager",
            Self::Other => "the OS credential store",
        }
    }
}

pub(crate) const CURRENT_CREDENTIAL_STORE_PLATFORM: CredentialStorePlatform =
    if cfg!(target_os = "macos") {
        CredentialStorePlatform::Macos
    } else if cfg!(target_os = "linux") {
        CredentialStorePlatform::Linux
    } else if cfg!(target_os = "windows") {
        CredentialStorePlatform::Windows
    } else {
        CredentialStorePlatform::Other
    };

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthStorageOperation {
    Command,
    Login,
}

const MACOS_AUTH_STORAGE_UNAVAILABLE: &str = "Authentication storage is unavailable.

SCE could not access its encryption key in macOS Keychain.

To fix:
  1. Run `sce auth login` in an interactive terminal.
  2. When macOS asks for permission to access the SCE Keychain item,
     choose \"Always Allow\".
  3. Retry the original command, or run `sce doctor` to verify
     telemetry access.

If no prompt appears, verify that your login Keychain is unlocked
and retry using the same installed SCE executable.";

const MACOS_LOGIN_AUTH_STORAGE_UNAVAILABLE: &str = "Authentication storage is unavailable.

SCE could not access its encryption key in macOS Keychain, so `sce auth login`
could not use its stored session. Running login again does not help until
Keychain access is allowed.

To fix:
  1. Unlock your login Keychain (for example, in Keychain Access).
  2. Run `sce auth login` again in an interactive terminal.
  3. When macOS asks for permission to access the SCE Keychain item,
     choose \"Always Allow\" to authorize this SCE executable.

If no prompt appears, verify that your login Keychain is unlocked
and retry using the same installed SCE executable.";

const LINUX_AUTH_STORAGE_UNAVAILABLE: &str = "Authentication storage is unavailable.

SCE could not access its encryption key in the system keyring (Secret Service).

To fix:
  1. Make sure a Secret Service provider (such as GNOME Keyring or KWallet)
     is running and its default collection is unlocked.
  2. Run `sce auth login` in an interactive terminal.
  3. Retry the original command.";

const LINUX_LOGIN_AUTH_STORAGE_UNAVAILABLE: &str = "Authentication storage is unavailable.

SCE could not access its encryption key in the system keyring (Secret Service),
so `sce auth login` could not use its stored session. Running login again does
not help until the keyring is available.

To fix:
  1. Make sure a Secret Service provider (such as GNOME Keyring or KWallet)
     is running and its default collection is unlocked.
  2. Retry `sce auth login` in an interactive terminal.";

const WINDOWS_AUTH_STORAGE_UNAVAILABLE: &str = "Authentication storage is unavailable.

SCE could not access its encryption key in Windows Credential Manager.

To fix:
  1. Make sure you are signed in to your Windows user account and
     Windows Credential Manager is available.
  2. Run `sce auth login` in an interactive terminal.
  3. Retry the original command.";

const WINDOWS_LOGIN_AUTH_STORAGE_UNAVAILABLE: &str = "Authentication storage is unavailable.

SCE could not access its encryption key in Windows Credential Manager,
so `sce auth login` could not use its stored session. Running login again
does not help until Credential Manager is available.

To fix:
  1. Make sure you are signed in to your Windows user account and
     Windows Credential Manager is available.
  2. Retry `sce auth login` in an interactive terminal.";

const OTHER_AUTH_STORAGE_UNAVAILABLE: &str = "Authentication storage is unavailable.

SCE could not access its encryption key in the OS credential store.

To fix:
  1. Make sure a supported OS credential store is available and unlocked.
  2. Run `sce auth login` in an interactive terminal.
  3. Retry the original command.";

const OTHER_LOGIN_AUTH_STORAGE_UNAVAILABLE: &str = "Authentication storage is unavailable.

SCE could not access its encryption key in the OS credential store,
so `sce auth login` could not use its stored session. Running login again
does not help until the credential store is available.

To fix:
  1. Make sure a supported OS credential store is available and unlocked.
  2. Retry `sce auth login` in an interactive terminal.";

const AUTH_DATABASE_LOCKED: &str = "Authentication storage is unavailable.

The SCE authentication database is locked by another SCE process.

To fix:
  1. Wait for other running `sce` processes, including Claude Code hooks,
     to finish.
  2. Make sure every installed `sce` executable is the same version;
     older versions cannot share the authentication database.
  3. Retry the command.";

const AUTH_DATABASE_UNAVAILABLE: &str = "Authentication storage is unavailable.

SCE could not open or read its local authentication database.

To fix:
  1. Run `sce doctor` to check the SCE state directory.
  2. Retry the command.
  3. If the problem persists, check the SCE log files for details.";

const AUTH_DISABLED_BY_GLOBAL_CONFIG: &str =
    "Authentication is disabled by `auth.mode: disabled` in the global SCE config.

To use SCE authentication, set `auth.mode` to `optional` or `mandatory` in that file, then retry.";

const AUTH_DISABLED_BY_REPOSITORY_CONFIG: &str =
    "Authentication is disabled by `auth.mode: disabled` in the repository SCE config (`.sce/config.json`).

To use SCE authentication, set `auth.mode` to `optional` or `mandatory` in that file, then retry.";

const AUTH_DISABLED_BY_FLAG_CONFIG: &str =
    "Authentication is disabled by `auth.mode: disabled` in the SCE config file passed with `--config`.

To use SCE authentication, set `auth.mode` to `optional` or `mandatory` in that file, then retry.";

const AUTH_DISABLED_BY_ENV_CONFIG: &str =
    "Authentication is disabled by `auth.mode: disabled` in the SCE config file named by `SCE_CONFIG_FILE`.

To use SCE authentication, set `auth.mode` to `optional` or `mandatory` in that file, then retry.";

const fn auth_disabled_by_config_message(file: AuthPolicyConfigFile) -> &'static str {
    match file {
        AuthPolicyConfigFile::Global => AUTH_DISABLED_BY_GLOBAL_CONFIG,
        AuthPolicyConfigFile::Repository => AUTH_DISABLED_BY_REPOSITORY_CONFIG,
        AuthPolicyConfigFile::ConfigFlag => AUTH_DISABLED_BY_FLAG_CONFIG,
        AuthPolicyConfigFile::ConfigEnv => AUTH_DISABLED_BY_ENV_CONFIG,
    }
}

const fn auth_storage_unavailable_message(
    platform: CredentialStorePlatform,
    operation: AuthStorageOperation,
    failure: AuthStorageFailure,
) -> &'static str {
    match failure {
        AuthStorageFailure::CredentialAccess => credential_access_message(platform, operation),
        AuthStorageFailure::DatabaseLocked => AUTH_DATABASE_LOCKED,
        AuthStorageFailure::Database => AUTH_DATABASE_UNAVAILABLE,
    }
}

const fn credential_access_message(
    platform: CredentialStorePlatform,
    operation: AuthStorageOperation,
) -> &'static str {
    match (platform, operation) {
        (CredentialStorePlatform::Macos, AuthStorageOperation::Command) => {
            MACOS_AUTH_STORAGE_UNAVAILABLE
        }
        (CredentialStorePlatform::Macos, AuthStorageOperation::Login) => {
            MACOS_LOGIN_AUTH_STORAGE_UNAVAILABLE
        }
        (CredentialStorePlatform::Linux, AuthStorageOperation::Command) => {
            LINUX_AUTH_STORAGE_UNAVAILABLE
        }
        (CredentialStorePlatform::Linux, AuthStorageOperation::Login) => {
            LINUX_LOGIN_AUTH_STORAGE_UNAVAILABLE
        }
        (CredentialStorePlatform::Windows, AuthStorageOperation::Command) => {
            WINDOWS_AUTH_STORAGE_UNAVAILABLE
        }
        (CredentialStorePlatform::Windows, AuthStorageOperation::Login) => {
            WINDOWS_LOGIN_AUTH_STORAGE_UNAVAILABLE
        }
        (CredentialStorePlatform::Other, AuthStorageOperation::Command) => {
            OTHER_AUTH_STORAGE_UNAVAILABLE
        }
        (CredentialStorePlatform::Other, AuthStorageOperation::Login) => {
            OTHER_LOGIN_AUTH_STORAGE_UNAVAILABLE
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
        let error = CliError::User {
            error: UserError::NotAuthenticated,
            source: None,
        };

        assert_eq!(error.class(), FailureClass::Runtime);
        assert_eq!(error.code(), "SCE-ERR-RUNTIME");
        assert_eq!(UserError::NotAuthenticated.key(), "auth.not_authenticated");
        assert!(error.to_string().contains("You are not logged in"));
    }

    #[test]
    fn auth_disabled_by_config_has_stable_runtime_catalog_mapping_per_file() {
        for (file, location) in [
            (AuthPolicyConfigFile::Global, "in the global SCE config."),
            (
                AuthPolicyConfigFile::Repository,
                "in the repository SCE config (`.sce/config.json`).",
            ),
            (
                AuthPolicyConfigFile::ConfigFlag,
                "in the SCE config file passed with `--config`.",
            ),
            (
                AuthPolicyConfigFile::ConfigEnv,
                "in the SCE config file named by `SCE_CONFIG_FILE`.",
            ),
        ] {
            let user_error = UserError::AuthDisabledByConfig(file);
            let error = CliError::User {
                error: user_error.clone(),
                source: None,
            };

            assert_eq!(error.class(), FailureClass::Runtime);
            assert_eq!(error.code(), "SCE-ERR-RUNTIME");
            assert_eq!(user_error.key(), "auth.disabled_by_config");
            assert_eq!(
                user_error.message(),
                format!(
                    "Authentication is disabled by `auth.mode: disabled` {location}\n\nTo use SCE authentication, set `auth.mode` to `optional` or `mandatory` in that file, then retry."
                )
            );
            assert_eq!(error.to_string(), user_error.message());
        }
    }

    #[test]
    fn not_git_repository_has_stable_runtime_catalog_mapping() {
        let error = CliError::User {
            error: UserError::NotGitRepository,
            source: None,
        };

        assert_eq!(error.class(), FailureClass::Runtime);
        assert_eq!(error.code(), "SCE-ERR-RUNTIME");
        assert_eq!(
            UserError::NotGitRepository.key(),
            "setup.not_git_repository"
        );
        assert_eq!(error.to_string(), UserError::NotGitRepository.message());
    }

    #[test]
    fn unexpected_failure_has_stable_runtime_catalog_mapping() {
        let error = CliError::User {
            error: UserError::UnexpectedFailure,
            source: None,
        };

        assert_eq!(error.class(), FailureClass::Runtime);
        assert_eq!(error.code(), "SCE-ERR-RUNTIME");
        assert_eq!(
            UserError::UnexpectedFailure.key(),
            "general.unexpected_failure"
        );
        assert_eq!(
            UserError::UnexpectedFailure.message(),
            "An unexpected error occurred. Check the log files for more details."
        );
        assert_eq!(
            error.to_string(),
            "An unexpected error occurred. Check the log files for more details."
        );
    }

    #[test]
    fn unexpected_failure_has_one_static_safe_message() {
        assert_eq!(
            UserError::UnexpectedFailure.message(),
            "An unexpected error occurred. Check the log files for more details."
        );
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

    const ALL_PLATFORMS: [CredentialStorePlatform; 4] = [
        CredentialStorePlatform::Macos,
        CredentialStorePlatform::Linux,
        CredentialStorePlatform::Windows,
        CredentialStorePlatform::Other,
    ];
    const ALL_OPERATIONS: [AuthStorageOperation; 2] =
        [AuthStorageOperation::Command, AuthStorageOperation::Login];
    const ALL_FAILURES: [AuthStorageFailure; 3] = [
        AuthStorageFailure::CredentialAccess,
        AuthStorageFailure::DatabaseLocked,
        AuthStorageFailure::Database,
    ];

    #[test]
    fn auth_storage_unavailable_keeps_stable_key_class_and_exit_code() {
        for failure in ALL_FAILURES {
            for user_error in [
                UserError::AuthStorageUnavailable(failure),
                UserError::LoginAuthStorageUnavailable(failure),
            ] {
                let error = CliError::user_with_source(
                    user_error.clone(),
                    anyhow::anyhow!("keychain item secret-key-material denied"),
                );
                assert_eq!(user_error.key(), "auth.storage_unavailable");
                assert_eq!(error.class(), FailureClass::Runtime);
                assert_eq!(error.code(), "SCE-ERR-RUNTIME");
                assert_eq!(error.class().exit_code(), 4);
                assert_eq!(error.to_string(), user_error.message());
                assert!(!error.to_string().contains("secret-key-material"));
            }
        }
    }

    #[test]
    fn macos_auth_storage_guidance_explains_login_and_always_allow() {
        let message = auth_storage_unavailable_message(
            CredentialStorePlatform::Macos,
            AuthStorageOperation::Command,
            AuthStorageFailure::CredentialAccess,
        );
        assert!(message.starts_with("Authentication storage is unavailable.\n"));
        assert!(message.contains("macOS Keychain"));
        assert!(message.contains("Run `sce auth login` in an interactive terminal."));
        assert!(message.contains("choose \"Always Allow\"."));
        assert!(message.contains("Retry the original command, or run `sce doctor`"));
        assert!(message.contains("login Keychain is unlocked"));
    }

    #[test]
    fn macos_login_storage_guidance_is_not_circular() {
        let message = auth_storage_unavailable_message(
            CredentialStorePlatform::Macos,
            AuthStorageOperation::Login,
            AuthStorageFailure::CredentialAccess,
        );
        assert!(message.contains("Running login again does not help until"));
        assert!(message.contains("Unlock your login Keychain"));
        assert!(message.contains("\"Always Allow\" to authorize this SCE executable"));
        assert!(!message.contains("Retry the original command"));
    }

    #[test]
    fn database_failures_never_blame_the_credential_store() {
        for platform in ALL_PLATFORMS {
            for operation in ALL_OPERATIONS {
                let locked = auth_storage_unavailable_message(
                    platform,
                    operation,
                    AuthStorageFailure::DatabaseLocked,
                );
                assert!(locked.contains("locked by another SCE process"));
                let generic = auth_storage_unavailable_message(
                    platform,
                    operation,
                    AuthStorageFailure::Database,
                );
                assert!(
                    generic.contains("could not open or read its local authentication database")
                );
                for message in [locked, generic] {
                    assert!(message.starts_with("Authentication storage is unavailable.\n"));
                    assert!(
                        !message.contains("encryption key"),
                        "{platform:?}/{operation:?}"
                    );
                    assert!(!message.contains("Keychain"), "{platform:?}/{operation:?}");
                    assert!(
                        !message.contains("Always Allow"),
                        "{platform:?}/{operation:?}"
                    );
                    assert!(
                        !message.contains("sce auth login"),
                        "{platform:?}/{operation:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn non_macos_auth_storage_guidance_never_mentions_macos() {
        for platform in [
            CredentialStorePlatform::Linux,
            CredentialStorePlatform::Windows,
            CredentialStorePlatform::Other,
        ] {
            for operation in ALL_OPERATIONS {
                for failure in ALL_FAILURES {
                    let message = auth_storage_unavailable_message(platform, operation, failure);
                    assert!(message.starts_with("Authentication storage is unavailable.\n"));
                    assert!(!message.contains("macOS"), "{platform:?}");
                    assert!(!message.contains("Keychain"), "{platform:?}");
                    assert!(!message.contains("Always Allow"), "{platform:?}");
                }
                assert!(
                    auth_storage_unavailable_message(
                        platform,
                        operation,
                        AuthStorageFailure::CredentialAccess
                    )
                    .contains("sce auth login"),
                    "{platform:?}"
                );
            }
        }
    }

    #[test]
    fn auth_storage_guidance_never_recommends_destructive_repair() {
        for platform in ALL_PLATFORMS {
            for operation in ALL_OPERATIONS {
                for failure in ALL_FAILURES {
                    let message = auth_storage_unavailable_message(platform, operation, failure)
                        .to_lowercase();
                    for forbidden in ["auth.db", "delete", "regenerat", "reset", "permanent"] {
                        assert!(
                            !message.contains(forbidden),
                            "{platform:?}/{operation:?}/{failure:?} mentions {forbidden:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn current_platform_message_is_selected_for_display() {
        for failure in ALL_FAILURES {
            assert_eq!(
                UserError::AuthStorageUnavailable(failure).message(),
                auth_storage_unavailable_message(
                    CURRENT_CREDENTIAL_STORE_PLATFORM,
                    AuthStorageOperation::Command,
                    failure
                )
            );
            assert_eq!(
                UserError::LoginAuthStorageUnavailable(failure).message(),
                auth_storage_unavailable_message(
                    CURRENT_CREDENTIAL_STORE_PLATFORM,
                    AuthStorageOperation::Login,
                    failure
                )
            );
        }
        assert_eq!(
            CURRENT_CREDENTIAL_STORE_PLATFORM == CredentialStorePlatform::Macos,
            cfg!(target_os = "macos")
        );
    }
}
