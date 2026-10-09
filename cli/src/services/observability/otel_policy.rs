use crate::services::error::FailureClass;
use crate::services::observability::tracing_boundary::OperationClass;
use crate::services::{
    auth_command, bash_policy, completion, config, doctor, help, hooks, setup, sync, version,
};

pub const OTEL_TARGET: &str = "sce::otel";

const KEY_COMMAND_NAME: &str = "sce.command.name";
const KEY_OUTCOME: &str = "sce.outcome";
const KEY_ERROR_CATEGORY: &str = "sce.error.category";
const KEY_OPERATION_TYPE: &str = "sce.operation.type";
const KEY_DURATION_MS: &str = "sce.duration_ms";
const KEY_COUNT: &str = "sce.count";
const KEY_ATTEMPTS: &str = "sce.attempts";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtelName {
    Command,
    MutationScopeCoordinate,
    WorktreeLock,
    Reconciliation,
    GitSnapshot,
    DbOperation,
    Sync,
}

impl OtelName {
    pub const ALL: [Self; 7] = [
        Self::Command,
        Self::MutationScopeCoordinate,
        Self::WorktreeLock,
        Self::Reconciliation,
        Self::GitSnapshot,
        Self::DbOperation,
        Self::Sync,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "sce.command",
            Self::MutationScopeCoordinate => "sce.mutation_scope.coordinate",
            Self::WorktreeLock => "sce.worktree.lock",
            Self::Reconciliation => "sce.reconciliation",
            Self::GitSnapshot => "sce.git.snapshot",
            Self::DbOperation => "sce.db.operation",
            Self::Sync => "sce.sync",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandName {
    Help,
    Auth,
    Config,
    Setup,
    Doctor,
    Hooks,
    Policy,
    Version,
    Completion,
    Sync,
}

impl CommandName {
    pub const ALL: [Self; 10] = [
        Self::Help,
        Self::Auth,
        Self::Config,
        Self::Setup,
        Self::Doctor,
        Self::Hooks,
        Self::Policy,
        Self::Version,
        Self::Completion,
        Self::Sync,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Help => help::NAME,
            Self::Auth => auth_command::NAME,
            Self::Config => config::NAME,
            Self::Setup => setup::NAME,
            Self::Doctor => doctor::NAME,
            Self::Hooks => hooks::NAME,
            Self::Policy => bash_policy::NAME,
            Self::Version => version::NAME,
            Self::Completion => completion::NAME,
            Self::Sync => sync::NAME,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Success,
    Failure,
    Cancelled,
    Timeout,
}

impl Outcome {
    pub const ALL: [Self; 4] = [Self::Success, Self::Failure, Self::Cancelled, Self::Timeout];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCategory {
    Parse,
    Validation,
    Runtime,
    Dependency,
    Unclassified,
}

impl ErrorCategory {
    pub const ALL: [Self; 5] = [
        Self::Parse,
        Self::Validation,
        Self::Runtime,
        Self::Dependency,
        Self::Unclassified,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Validation => "validation",
            Self::Runtime => "runtime",
            Self::Dependency => "dependency",
            Self::Unclassified => "unclassified",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }
}

impl From<FailureClass> for ErrorCategory {
    fn from(class: FailureClass) -> Self {
        match class {
            FailureClass::Parse => Self::Parse,
            FailureClass::Validation => Self::Validation,
            FailureClass::Runtime => Self::Runtime,
            FailureClass::Dependency => Self::Dependency,
        }
    }
}

pub type OperationType = OperationClass;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtelRawValue<'a> {
    Number(u64),
    Text(&'a str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtelValue {
    Number(u64),
    Static(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtelAttribute {
    CommandName(CommandName),
    Outcome(Outcome),
    ErrorCategory(ErrorCategory),
    OperationType(OperationType),
    DurationMs(u64),
    Count(u64),
    Attempts(u64),
}

impl OtelAttribute {
    #[cfg_attr(not(test), allow(dead_code))]
    pub const KEYS: [&'static str; 7] = [
        KEY_COMMAND_NAME,
        KEY_OUTCOME,
        KEY_ERROR_CATEGORY,
        KEY_OPERATION_TYPE,
        KEY_DURATION_MS,
        KEY_COUNT,
        KEY_ATTEMPTS,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::CommandName(_) => KEY_COMMAND_NAME,
            Self::Outcome(_) => KEY_OUTCOME,
            Self::ErrorCategory(_) => KEY_ERROR_CATEGORY,
            Self::OperationType(_) => KEY_OPERATION_TYPE,
            Self::DurationMs(_) => KEY_DURATION_MS,
            Self::Count(_) => KEY_COUNT,
            Self::Attempts(_) => KEY_ATTEMPTS,
        }
    }

    pub fn value(self) -> OtelValue {
        match self {
            Self::CommandName(value) => OtelValue::Static(value.as_str()),
            Self::Outcome(value) => OtelValue::Static(value.as_str()),
            Self::ErrorCategory(value) => OtelValue::Static(value.as_str()),
            Self::OperationType(value) => OtelValue::Static(value.as_str()),
            Self::DurationMs(value) | Self::Count(value) | Self::Attempts(value) => {
                OtelValue::Number(value)
            }
        }
    }

    pub fn admit(key: &str, value: OtelRawValue<'_>) -> Option<Self> {
        match (key, value) {
            (KEY_COMMAND_NAME, OtelRawValue::Text(text)) => {
                CommandName::parse(text).map(Self::CommandName)
            }
            (KEY_OUTCOME, OtelRawValue::Text(text)) => Outcome::parse(text).map(Self::Outcome),
            (KEY_ERROR_CATEGORY, OtelRawValue::Text(text)) => {
                ErrorCategory::parse(text).map(Self::ErrorCategory)
            }
            (KEY_OPERATION_TYPE, OtelRawValue::Text(text)) => {
                OperationType::parse(text).map(Self::OperationType)
            }
            (KEY_DURATION_MS, OtelRawValue::Number(number)) => Some(Self::DurationMs(number)),
            (KEY_COUNT, OtelRawValue::Number(number)) => Some(Self::Count(number)),
            (KEY_ATTEMPTS, OtelRawValue::Number(number)) => Some(Self::Attempts(number)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct OtelRecord {
    pub name: OtelName,
    pub attributes: Vec<OtelAttribute>,
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn admit_otel_record(
    target: &str,
    name: &str,
    attributes: &[(&str, OtelRawValue<'_>)],
) -> Option<OtelRecord> {
    if target != OTEL_TARGET {
        return None;
    }

    let name = OtelName::parse(name)?;
    let attributes = attributes
        .iter()
        .filter_map(|(key, value)| OtelAttribute::admit(key, *value))
        .collect();
    Some(OtelRecord { name, attributes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::config::LogLevel;
    use crate::services::observability::tracing_boundary::test_capture::CapturingSubscriber;
    use crate::services::observability::tracing_boundary::{
        emit_contention_event, emit_logger_event, emit_retry_event, DbName, EventId,
        SCE_TRACING_TARGET,
    };

    const SECRET: &str = "sk-live-SECRET-TOKEN-9f8e7d";
    const SENSITIVE_PATH: &str = "/home/victim/.ssh/id_ed25519";
    const WHITELIST_PASSING_SECRETS: [&str; 3] = ["hunter2", "password123", "secret_token_123"];

    fn passes_old_character_whitelist(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-'))
    }

    fn text_attributes() -> Vec<OtelAttribute> {
        let mut attributes = Vec::new();
        attributes.extend(CommandName::ALL.map(OtelAttribute::CommandName));
        attributes.extend(Outcome::ALL.map(OtelAttribute::Outcome));
        attributes.extend(ErrorCategory::ALL.map(OtelAttribute::ErrorCategory));
        attributes.extend(OperationType::ALL.map(OtelAttribute::OperationType));
        attributes
    }

    fn text_of(attribute: OtelAttribute) -> &'static str {
        match attribute.value() {
            OtelValue::Static(text) => text,
            OtelValue::Number(_) => panic!("expected text attribute"),
        }
    }

    #[test]
    fn tracing_boundary_b_admits_only_otel_target_and_enumerated_names() {
        let ok = admit_otel_record(
            OTEL_TARGET,
            "sce.command",
            &[
                ("sce.outcome", OtelRawValue::Text("success")),
                ("sce.duration_ms", OtelRawValue::Number(12)),
            ],
        )
        .expect("admitted");
        assert_eq!(ok.name, OtelName::Command);
        assert_eq!(
            ok.attributes,
            vec![
                OtelAttribute::Outcome(Outcome::Success),
                OtelAttribute::DurationMs(12)
            ]
        );

        assert!(admit_otel_record(SCE_TRACING_TARGET, "sce.command", &[]).is_none());
        assert!(admit_otel_record("sce::services::resilience", "sce.command", &[]).is_none());
        assert!(admit_otel_record(OTEL_TARGET, "sce.arbitrary", &[]).is_none());
        assert!(admit_otel_record(OTEL_TARGET, SECRET, &[]).is_none());
    }

    #[test]
    fn tracing_boundary_b_every_valid_enum_produces_expected_static_value() {
        let commands = [
            (CommandName::Help, "help"),
            (CommandName::Auth, "auth"),
            (CommandName::Config, "config"),
            (CommandName::Setup, "setup"),
            (CommandName::Doctor, "doctor"),
            (CommandName::Hooks, "hooks"),
            (CommandName::Policy, "policy"),
            (CommandName::Version, "version"),
            (CommandName::Completion, "completion"),
            (CommandName::Sync, "sync"),
        ];
        assert_eq!(commands.len(), CommandName::ALL.len());
        for (variant, expected) in commands {
            assert_eq!(variant.as_str(), expected);
            assert_eq!(CommandName::parse(expected), Some(variant));
            assert_eq!(
                OtelAttribute::CommandName(variant).value(),
                OtelValue::Static(expected)
            );
        }

        let outcomes = [
            (Outcome::Success, "success"),
            (Outcome::Failure, "failure"),
            (Outcome::Cancelled, "cancelled"),
            (Outcome::Timeout, "timeout"),
        ];
        assert_eq!(outcomes.len(), Outcome::ALL.len());
        for (variant, expected) in outcomes {
            assert_eq!(variant.as_str(), expected);
            assert_eq!(Outcome::parse(expected), Some(variant));
            assert_eq!(
                OtelAttribute::Outcome(variant).value(),
                OtelValue::Static(expected)
            );
        }

        assert_eq!(OtelAttribute::DurationMs(7).value(), OtelValue::Number(7));
        assert_eq!(OtelAttribute::Count(8).value(), OtelValue::Number(8));
        assert_eq!(OtelAttribute::Attempts(9).value(), OtelValue::Number(9));
    }

    #[test]
    fn tracing_boundary_b_every_valid_error_category_and_operation_type_produces_expected_static_value(
    ) {
        let categories = [
            (ErrorCategory::Parse, "parse"),
            (ErrorCategory::Validation, "validation"),
            (ErrorCategory::Runtime, "runtime"),
            (ErrorCategory::Dependency, "dependency"),
            (ErrorCategory::Unclassified, "unclassified"),
        ];
        assert_eq!(categories.len(), ErrorCategory::ALL.len());
        for (variant, expected) in categories {
            assert_eq!(variant.as_str(), expected);
            assert_eq!(ErrorCategory::parse(expected), Some(variant));
            assert_eq!(
                OtelAttribute::ErrorCategory(variant).value(),
                OtelValue::Static(expected)
            );
        }

        let operations = [
            (OperationType::AuthRefreshToken, "auth.refresh_token"),
            (
                OperationType::AgentTraceSyncIngestionState,
                "agent_trace_sync.ingestion_state",
            ),
            (OperationType::DbOpenConnection, "db.open_connection"),
            (
                OperationType::DbOpenEncryptedConnection,
                "db.open_encrypted_connection",
            ),
            (OperationType::DbExecuteQuery, "db.execute_query"),
            (
                OperationType::DbExecuteEncryptedQuery,
                "db.execute_encrypted_query",
            ),
            (OperationType::DbQueryRows, "db.query_rows"),
            (
                OperationType::DbQueryEncryptedRows,
                "db.query_encrypted_rows",
            ),
            (OperationType::DbCheckpointWal, "db.checkpoint_wal"),
            (OperationType::Unclassified, "unclassified"),
        ];
        assert_eq!(operations.len(), OperationType::ALL.len());
        for (variant, expected) in operations {
            assert_eq!(variant.as_str(), expected);
            assert_eq!(OperationType::parse(expected), Some(variant));
            assert_eq!(
                OtelAttribute::OperationType(variant).value(),
                OtelValue::Static(expected)
            );
        }

        assert_eq!(
            ErrorCategory::from(FailureClass::Parse),
            ErrorCategory::Parse
        );
        assert_eq!(
            ErrorCategory::from(FailureClass::Validation),
            ErrorCategory::Validation
        );
        assert_eq!(
            ErrorCategory::from(FailureClass::Runtime),
            ErrorCategory::Runtime
        );
        assert_eq!(
            ErrorCategory::from(FailureClass::Dependency),
            ErrorCategory::Dependency
        );
    }

    #[test]
    fn tracing_boundary_b_unknown_command_names_are_rejected() {
        for candidate in [
            "hunter2",
            "secret_token_123",
            "password123",
            "private_customer_name",
            "Sync",
            "SYNC",
            " sync",
            "sync ",
            "sync\n",
            "",
            SECRET,
            SENSITIVE_PATH,
        ] {
            assert_eq!(CommandName::parse(candidate), None, "{candidate:?}");
            assert_eq!(
                OtelAttribute::admit("sce.command.name", OtelRawValue::Text(candidate)),
                None,
                "{candidate:?}"
            );
        }
    }

    #[test]
    fn tracing_boundary_b_unknown_outcomes_cannot_be_exported() {
        for candidate in [
            "hunter2",
            "secret_token_123",
            "password123",
            "Success",
            "success ",
            "ok",
            "",
            SECRET,
            SENSITIVE_PATH,
        ] {
            assert_eq!(Outcome::parse(candidate), None, "{candidate:?}");
            assert_eq!(
                OtelAttribute::admit("sce.outcome", OtelRawValue::Text(candidate)),
                None,
                "{candidate:?}"
            );
        }
    }

    #[test]
    fn tracing_boundary_b_unknown_error_categories_cannot_be_exported() {
        for candidate in [
            "hunter2",
            "secret_token_123",
            "password123",
            "SCE-ERR-RUNTIME",
            "Runtime",
            "boom",
            "",
            SECRET,
            SENSITIVE_PATH,
        ] {
            assert_eq!(ErrorCategory::parse(candidate), None, "{candidate:?}");
            assert_eq!(
                OtelAttribute::admit("sce.error.category", OtelRawValue::Text(candidate)),
                None,
                "{candidate:?}"
            );
        }
    }

    #[test]
    fn tracing_boundary_b_unknown_operation_types_cannot_be_exported() {
        for candidate in [
            "hunter2",
            "secret_token_123",
            "password123",
            "private_customer_name",
            "db.execute_query;drop",
            "DB.EXECUTE_QUERY",
            "rm -rf /",
            "",
            SECRET,
            SENSITIVE_PATH,
        ] {
            assert_eq!(OperationType::parse(candidate), None, "{candidate:?}");
            assert_eq!(
                OtelAttribute::admit("sce.operation.type", OtelRawValue::Text(candidate)),
                None,
                "{candidate:?}"
            );
        }
    }

    #[test]
    fn tracing_boundary_b_secret_passing_old_character_whitelist_is_rejected() {
        for secret in WHITELIST_PASSING_SECRETS {
            assert!(
                passes_old_character_whitelist(secret),
                "{secret} must satisfy the legacy whitelist for this test to be meaningful"
            );
            for key in &OtelAttribute::KEYS[..4] {
                assert_eq!(
                    OtelAttribute::admit(key, OtelRawValue::Text(secret)),
                    None,
                    "{key}={secret}"
                );
            }
            let record = admit_otel_record(
                OTEL_TARGET,
                "sce.command",
                &[
                    ("sce.outcome", OtelRawValue::Text(secret)),
                    ("sce.command.name", OtelRawValue::Text(secret)),
                    ("sce.error.category", OtelRawValue::Text(secret)),
                    ("sce.operation.type", OtelRawValue::Text(secret)),
                ],
            )
            .expect("record admitted without attributes");
            assert!(record.attributes.is_empty());
        }
    }

    #[test]
    fn tracing_boundary_b_excludes_paths_credentials_exception_text_and_stack_traces() {
        let record = admit_otel_record(
            OTEL_TARGET,
            "sce.db.operation",
            &[
                ("exception.message", OtelRawValue::Text("boom")),
                ("exception.stacktrace", OtelRawValue::Text("at main")),
                ("exception.type", OtelRawValue::Text("io_error")),
                ("path", OtelRawValue::Text(SENSITIVE_PATH)),
                (
                    "http.request.header.authorization",
                    OtelRawValue::Text(SECRET),
                ),
                ("authorization", OtelRawValue::Text(SECRET)),
                ("password", OtelRawValue::Text("password123")),
                ("sce.error.category", OtelRawValue::Text(SENSITIVE_PATH)),
                ("sce.outcome", OtelRawValue::Text(SECRET)),
                ("sce.count", OtelRawValue::Number(3)),
                ("sce.operation.type", OtelRawValue::Text("db.execute_query")),
            ],
        )
        .expect("admitted");

        assert_eq!(
            record.attributes,
            vec![
                OtelAttribute::Count(3),
                OtelAttribute::OperationType(OperationType::DbExecuteQuery)
            ]
        );
        for key in OtelAttribute::KEYS {
            assert!(!key.contains("exception"));
            assert!(!key.contains("path"));
            assert!(!key.contains("authorization"));
        }
    }

    #[test]
    fn tracing_boundary_b_attribute_keys_and_values_cannot_be_mismatched() {
        let attributes = text_attributes();
        for key in &OtelAttribute::KEYS[..4] {
            for attribute in &attributes {
                let text = text_of(*attribute);
                match OtelAttribute::admit(key, OtelRawValue::Text(text)) {
                    Some(admitted) => {
                        assert_eq!(admitted.key(), *key, "{key} admitted {text}");
                        assert_eq!(admitted.value(), OtelValue::Static(text));
                    }
                    None => assert_ne!(attribute.key(), *key, "{key} must admit its own {text}"),
                }
            }
        }

        assert_eq!(
            OtelAttribute::admit("sce.outcome", OtelRawValue::Text("doctor")),
            None
        );
        assert_eq!(
            OtelAttribute::admit("sce.command.name", OtelRawValue::Text("success")),
            None
        );
        assert_eq!(
            OtelAttribute::admit("sce.error.category", OtelRawValue::Text("success")),
            None
        );
        assert_eq!(
            OtelAttribute::admit("sce.operation.type", OtelRawValue::Text("parse")),
            None
        );

        for key in &OtelAttribute::KEYS[..4] {
            assert_eq!(OtelAttribute::admit(key, OtelRawValue::Number(1)), None);
        }
        for key in &OtelAttribute::KEYS[4..] {
            assert_eq!(OtelAttribute::admit(key, OtelRawValue::Text("12")), None);
            assert_eq!(
                OtelAttribute::admit(key, OtelRawValue::Text("success")),
                None
            );
            assert!(OtelAttribute::admit(key, OtelRawValue::Number(12)).is_some());
        }
    }

    #[test]
    fn tracing_boundary_b_policy_names_and_keys_have_stable_shapes() {
        for name in OtelName::ALL {
            assert_eq!(OtelName::parse(name.as_str()), Some(name));
        }
        let samples = [
            OtelAttribute::CommandName(CommandName::Sync),
            OtelAttribute::Outcome(Outcome::Success),
            OtelAttribute::ErrorCategory(ErrorCategory::Runtime),
            OtelAttribute::OperationType(OperationType::DbQueryRows),
            OtelAttribute::DurationMs(1),
            OtelAttribute::Count(2),
            OtelAttribute::Attempts(3),
        ];
        let keys: Vec<_> = samples.iter().map(|sample| sample.key()).collect();
        assert_eq!(keys, OtelAttribute::KEYS.to_vec());
        for sample in samples {
            let raw = match sample.value() {
                OtelValue::Number(number) => OtelRawValue::Number(number),
                OtelValue::Static(text) => OtelRawValue::Text(text),
            };
            assert_eq!(OtelAttribute::admit(sample.key(), raw), Some(sample));
        }
    }

    #[test]
    fn tracing_boundary_b_sce_logger_events_cannot_enter_the_otel_export_surface() {
        let events = CapturingSubscriber::capture(|| {
            for level in [
                LogLevel::Error,
                LogLevel::Warn,
                LogLevel::Info,
                LogLevel::Debug,
            ] {
                emit_logger_event(level, EventId::classify("sce.app.start"));
                emit_logger_event(level, EventId::classify(SECRET));
            }
            emit_retry_event(OperationClass::DbExecuteQuery, 1, 3, 1_000, 10);
            emit_contention_event(
                DbName::AuthDb,
                OperationClass::DbOpenConnection,
                2,
                5,
                50,
                40,
            );
        });

        assert_eq!(events.len(), 10);
        assert_ne!(OTEL_TARGET, SCE_TRACING_TARGET);
        for event in &events {
            assert_eq!(event.target, SCE_TRACING_TARGET);
            let attributes: Vec<(&str, OtelRawValue<'_>)> = event
                .fields
                .iter()
                .map(|(key, value)| (key.as_str(), OtelRawValue::Text(value.as_str())))
                .collect();
            for name in OtelName::ALL {
                assert!(
                    admit_otel_record(&event.target, name.as_str(), &attributes).is_none(),
                    "{} admitted as {}",
                    event.rendered(),
                    name.as_str()
                );
            }
            let event_id = event.field("event_id").expect("event_id field");
            assert!(admit_otel_record(&event.target, event_id, &attributes).is_none());
        }
    }
}
