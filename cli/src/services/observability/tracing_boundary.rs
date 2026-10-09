use std::future::Future;

use tracing::instrument::{Instrument, WithSubscriber};
use tracing::Dispatch;
use tracing::Level;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{Layer, Registry};

use crate::services::config::LogLevel;
use crate::services::error::{CliError, FailureClass};

#[cfg(test)]
mod audit;

pub const SCE_TRACING_TARGET: &str = "sce";
pub const CONTENTION_EXHAUSTED_CAUSE: &str = "database busy (busy timeout exhausted)";
pub const UNCLASSIFIED_EVENT_ID: &str = "sce.unclassified";
pub const UNCLASSIFIED_LABEL: &str = "unclassified";
pub const RESILIENCE_RETRY_EVENT_ID: &str = "sce.resilience.retry";
pub const CONTENTION_EXHAUSTED_EVENT_ID: &str = "sce.agent_trace_db.contention_exhausted";

pub const TRACING_EVENT_IDS: &[&str] = &[
    "sce.agent_trace_db.contention_exhausted",
    "sce.agent_trace_db.passive_checkpoint_failed",
    "sce.app.start",
    "sce.command.completed",
    "sce.command.dispatch_end",
    "sce.command.dispatch_start",
    "sce.command.parsed",
    "sce.command.raw_args",
    "sce.config.file_discovered",
    "sce.config.invalid_config",
    "sce.error.dependency",
    "sce.error.parse",
    "sce.error.runtime",
    "sce.error.validation",
    "sce.hooks.claude_model_state.agent_trace_db_open_failed",
    "sce.hooks.claude_model_state.agent_trace_db_write_failed",
    "sce.hooks.claude_model_state.error",
    "sce.hooks.claude_mutation_scope.model_state_unavailable",
    "sce.hooks.claude_mutation_scope.pre_tool_use_fail_closed",
    "sce.hooks.codex.apply_patch.normalize_failed",
    "sce.hooks.codex.apply_patch.parse_failed",
    "sce.hooks.codex.apply_patch.path_resolution_failed",
    "sce.hooks.codex.error",
    "sce.hooks.codex_mutation_scope.pre_tool_use_fail_closed",
    "sce.hooks.commit_msg.ai_overlap_error",
    "sce.hooks.conversation_trace.agent_trace_db_batch_failed",
    "sce.hooks.conversation_trace.agent_trace_db_open_failed",
    "sce.hooks.conversation_trace.error",
    "sce.hooks.conversation_trace.payload_skipped",
    "sce.hooks.diff_trace.agent_trace_db_open_failed",
    "sce.hooks.diff_trace.agent_trace_db_time_invalid",
    "sce.hooks.diff_trace.agent_trace_db_write_failed",
    "sce.hooks.diff_trace.error",
    "sce.hooks.mutation_scope.marker_clear_after_durable_completion",
    "sce.hooks.mutation_scope.ref_reconciliation_advisory",
    "sce.hooks.opencode_mutation_scope.start_fail_closed",
    "sce.hooks.pi_mutation_scope.start_fail_closed",
    "sce.resilience.retry",
    "sce.unclassified",
];

pub fn classify_event_id(event_id: &str) -> &'static str {
    match TRACING_EVENT_IDS.binary_search(&event_id) {
        Ok(index) => TRACING_EVENT_IDS[index],
        Err(_) => UNCLASSIFIED_EVENT_ID,
    }
}

pub fn cli_error_tracing_event_id(error: &CliError) -> &'static str {
    match error.class() {
        FailureClass::Parse => "sce.error.parse",
        FailureClass::Validation => "sce.error.validation",
        FailureClass::Runtime => "sce.error.runtime",
        FailureClass::Dependency => "sce.error.dependency",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventId(&'static str);

impl EventId {
    pub const RESILIENCE_RETRY: Self = Self(RESILIENCE_RETRY_EVENT_ID);
    pub const CONTENTION_EXHAUSTED: Self = Self(CONTENTION_EXHAUSTED_EVENT_ID);

    pub fn classify(event_id: &str) -> Self {
        Self(classify_event_id(event_id))
    }

    pub fn from_cli_error(error: &CliError) -> Self {
        Self(cli_error_tracing_event_id(error))
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationClass {
    AuthRefreshToken,
    AgentTraceSyncIngestionState,
    DbOpenConnection,
    DbOpenEncryptedConnection,
    DbExecuteQuery,
    DbExecuteEncryptedQuery,
    DbQueryRows,
    DbQueryEncryptedRows,
    DbCheckpointWal,
    Unclassified,
}

impl OperationClass {
    pub const ALL: [Self; 10] = [
        Self::AuthRefreshToken,
        Self::AgentTraceSyncIngestionState,
        Self::DbOpenConnection,
        Self::DbOpenEncryptedConnection,
        Self::DbExecuteQuery,
        Self::DbExecuteEncryptedQuery,
        Self::DbQueryRows,
        Self::DbQueryEncryptedRows,
        Self::DbCheckpointWal,
        Self::Unclassified,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }

    pub fn classify(operation: &str) -> Self {
        match operation {
            "auth.refresh_token" => return Self::AuthRefreshToken,
            "agent_trace_sync.ingestion_state" => return Self::AgentTraceSyncIngestionState,
            _ => {}
        }

        let framed = |prefix: &str, suffix: &str| {
            operation.len() >= prefix.len() + suffix.len()
                && operation.starts_with(prefix)
                && operation.ends_with(suffix)
        };

        if framed("open encrypted ", " database connection") {
            Self::DbOpenEncryptedConnection
        } else if framed("open ", " database connection") {
            Self::DbOpenConnection
        } else if framed("execute encrypted ", " database query") {
            Self::DbExecuteEncryptedQuery
        } else if framed("execute ", " database query") {
            Self::DbExecuteQuery
        } else if framed("query and fetch encrypted ", " database rows") {
            Self::DbQueryEncryptedRows
        } else if framed("query and fetch ", " database rows") {
            Self::DbQueryRows
        } else if framed("checkpoint ", " database WAL") {
            Self::DbCheckpointWal
        } else {
            Self::Unclassified
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuthRefreshToken => "auth.refresh_token",
            Self::AgentTraceSyncIngestionState => "agent_trace_sync.ingestion_state",
            Self::DbOpenConnection => "db.open_connection",
            Self::DbOpenEncryptedConnection => "db.open_encrypted_connection",
            Self::DbExecuteQuery => "db.execute_query",
            Self::DbExecuteEncryptedQuery => "db.execute_encrypted_query",
            Self::DbQueryRows => "db.query_rows",
            Self::DbQueryEncryptedRows => "db.query_encrypted_rows",
            Self::DbCheckpointWal => "db.checkpoint_wal",
            Self::Unclassified => UNCLASSIFIED_LABEL,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DbName {
    LocalDb,
    AuthDb,
    RepositoryAgentTraceDb,
    Unclassified,
}

impl DbName {
    pub fn classify(db_name: &str) -> Self {
        match db_name {
            "local DB" => Self::LocalDb,
            "auth DB" => Self::AuthDb,
            "repository Agent Trace DB" => Self::RepositoryAgentTraceDb,
            _ => Self::Unclassified,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalDb => "local_db",
            Self::AuthDb => "auth_db",
            Self::RepositoryAgentTraceDb => "repository_agent_trace_db",
            Self::Unclassified => UNCLASSIFIED_LABEL,
        }
    }
}

pub fn emit_logger_event(level: LogLevel, event_id: EventId) {
    let enabled = match level {
        LogLevel::Error => tracing::enabled!(target: SCE_TRACING_TARGET, Level::ERROR),
        LogLevel::Warn => tracing::enabled!(target: SCE_TRACING_TARGET, Level::WARN),
        LogLevel::Info => tracing::enabled!(target: SCE_TRACING_TARGET, Level::INFO),
        LogLevel::Debug => tracing::enabled!(target: SCE_TRACING_TARGET, Level::DEBUG),
    };
    if !enabled {
        return;
    }

    let event_id = event_id.as_str();
    let log_level = level.as_str();
    match level {
        LogLevel::Error => tracing::error!(
            target: SCE_TRACING_TARGET,
            event_id,
            log_level,
            "sce log event"
        ),
        LogLevel::Warn => tracing::warn!(
            target: SCE_TRACING_TARGET,
            event_id,
            log_level,
            "sce log event"
        ),
        LogLevel::Info => tracing::info!(
            target: SCE_TRACING_TARGET,
            event_id,
            log_level,
            "sce log event"
        ),
        LogLevel::Debug => tracing::debug!(
            target: SCE_TRACING_TARGET,
            event_id,
            log_level,
            "sce log event"
        ),
    }
}

pub fn emit_retry_event(
    operation: OperationClass,
    attempt: u32,
    max_attempts: u32,
    timeout_ms: u64,
    backoff_ms: u64,
) {
    let event_id = EventId::RESILIENCE_RETRY.as_str();
    let operation = operation.as_str();
    tracing::warn!(
        target: SCE_TRACING_TARGET,
        event_id,
        operation,
        attempt,
        max_attempts,
        timeout_ms,
        backoff_ms,
        "Retrying operation after transient failure"
    );
}

pub fn emit_contention_event(
    database: DbName,
    operation: OperationClass,
    attempts: u32,
    busy_timeout_ms: u64,
    contention_deadline_ms: u64,
    elapsed_ms: u64,
) {
    let event_id = EventId::CONTENTION_EXHAUSTED.as_str();
    let db_name = database.as_str();
    let operation = operation.as_str();
    let cause = CONTENTION_EXHAUSTED_CAUSE;
    tracing::warn!(
        target: SCE_TRACING_TARGET,
        event_id,
        db_name,
        operation,
        attempts,
        busy_timeout_ms,
        contention_deadline_ms,
        elapsed_ms,
        cause,
        "Agent Trace DB write contention retries exhausted"
    );
}

#[derive(Clone)]
pub struct ScopedDispatch(Dispatch);

impl ScopedDispatch {
    pub fn from_layer<L>(layer: L) -> Self
    where
        L: Layer<Registry> + Send + Sync + 'static,
    {
        Self(Dispatch::new(Registry::default().with(layer)))
    }

    pub fn scope<F: Future>(&self, future: F) -> impl Future<Output = F::Output> {
        future.with_subscriber(self.0.clone())
    }
}

pub fn spawn_in_current_scope<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future.with_current_subscriber().in_current_span())
}

#[cfg(test)]
pub(crate) mod test_capture {
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    };

    use tracing::{
        field::{Field, Visit},
        span, Event, Metadata, Subscriber,
    };

    #[derive(Clone, Debug)]
    pub struct CapturedEvent {
        pub target: String,
        pub fields: Vec<(String, String)>,
    }

    impl CapturedEvent {
        pub fn field(&self, name: &str) -> Option<&str> {
            self.fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        }

        pub fn rendered(&self) -> String {
            let mut rendered = self.target.clone();
            for (key, value) in &self.fields {
                rendered.push(' ');
                rendered.push_str(key);
                rendered.push('=');
                rendered.push_str(value);
            }
            rendered
        }
    }

    #[derive(Default)]
    struct FieldCollector(Vec<(String, String)>);

    impl Visit for FieldCollector {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.push((field.name().to_string(), value.to_string()));
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0
                .push((field.name().to_string(), format!("{value:?}")));
        }
    }

    #[derive(Clone, Default)]
    pub struct CapturingSubscriber {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
        next_span_id: Arc<AtomicU64>,
    }

    impl CapturingSubscriber {
        pub fn events(&self) -> Vec<CapturedEvent> {
            self.events.lock().expect("capture lock").clone()
        }

        pub fn capture(run: impl FnOnce()) -> Vec<CapturedEvent> {
            let subscriber = Self::default();
            let handle = subscriber.clone();
            tracing::subscriber::with_default(subscriber, run);
            handle.events()
        }
    }

    impl Subscriber for CapturingSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _attributes: &span::Attributes<'_>) -> span::Id {
            span::Id::from_u64(self.next_span_id.fetch_add(1, Ordering::SeqCst) + 1)
        }

        fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

        fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut collector = FieldCollector::default();
            event.record(&mut collector);
            self.events
                .lock()
                .expect("capture lock")
                .push(CapturedEvent {
                    target: event.metadata().target().to_string(),
                    fields: collector.0,
                });
        }

        fn enter(&self, _span: &span::Id) {}

        fn exit(&self, _span: &span::Id) {}
    }
}

#[cfg(test)]
pub(crate) mod test_spans {
    use std::future::Future;

    use tracing::instrument::Instrument;
    use tracing::Level;

    use crate::services::observability::otel_policy::{OtelName, OTEL_TARGET};

    pub fn in_otel_span<F: Future>(name: OtelName, future: F) -> impl Future<Output = F::Output> {
        let span = match name {
            OtelName::Command => tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.command"),
            OtelName::MutationScopeCoordinate => {
                tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.mutation_scope.coordinate")
            }
            OtelName::WorktreeLock => {
                tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.worktree.lock")
            }
            OtelName::Reconciliation => {
                tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.reconciliation")
            }
            OtelName::GitSnapshot => {
                tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.git.snapshot")
            }
            OtelName::DbOperation => {
                tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.db.operation")
            }
            OtelName::Sync => tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.sync"),
        };
        future.instrument(span)
    }

    pub fn emit_raw_boundary_b_inputs(secret: &'static str) {
        let allowed = tracing::span!(
            target: OTEL_TARGET,
            Level::INFO,
            "sce.command",
            "sce.outcome" = "success",
            "sce.duration_ms" = 5u64,
            "sce.command.name" = secret,
            "sce.error.category" = "parse",
            "sce.arbitrary_key" = secret,
            password = secret,
            "otel.status_code" = "error",
            "otel.status_description" = secret
        );
        {
            let _entered = allowed.enter();
            tracing::event!(target: OTEL_TARGET, Level::ERROR, exception = secret, "exception message");
        }
        drop(allowed);
        drop(tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.arbitrary"));
        drop(tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.command", "otel.name" = secret));
        drop(tracing::span!(target: "sce", Level::INFO, "sce.command", "sce.outcome" = "success"));
        drop(tracing::span!(target: "sce::services::other", Level::INFO, "sce.command"));
    }

    pub fn thread_default_is_none() -> bool {
        tracing::dispatcher::get_default(|dispatch| {
            dispatch.is::<tracing::subscriber::NoSubscriber>()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::test_capture::CapturingSubscriber;
    use super::*;
    use crate::services::config::{LogFormat, LogLevel};
    use crate::services::error::UserError;
    use crate::services::observability::{Logger, ObservabilityConfig};

    const SECRET: &str = "sk-live-SECRET-TOKEN-9f8e7d";
    const SENSITIVE_PATH: &str = "/home/victim/.ssh/id_ed25519";

    fn memory_logger() -> Logger {
        Logger {
            config: ObservabilityConfig {
                level: LogLevel::Debug,
                format: LogFormat::Text,
                log_to_file: false,
            },
            log_dir: None,
            log_file_retention_limit: 1,
        }
    }

    fn assert_clean(events: &[super::test_capture::CapturedEvent], forbidden: &[&str]) {
        assert!(!events.is_empty(), "expected captured events");
        for event in events {
            let rendered = event.rendered();
            for needle in forbidden {
                assert!(
                    !rendered.contains(needle),
                    "captured event leaked {needle:?}: {rendered}"
                );
            }
            for banned in ["event_message", "error_source", "last_error"] {
                assert!(event.field(banned).is_none(), "banned field {banned}");
            }
            assert!(event.field("fields").is_none(), "banned field fields");
            assert!(event.field("error").is_none(), "banned field error");
        }
    }

    #[test]
    fn tracing_boundary_a_registry_is_sorted_unique_and_well_shaped() {
        for pair in TRACING_EVENT_IDS.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{} must sort before {}",
                pair[0],
                pair[1]
            );
        }
        for id in TRACING_EVENT_IDS {
            assert!(
                (1..=64).contains(&id.len())
                    && id
                        .bytes()
                        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.')),
                "event id {id:?} violates shape"
            );
        }
        assert!(TRACING_EVENT_IDS.contains(&UNCLASSIFIED_EVENT_ID));
        assert!(TRACING_EVENT_IDS.contains(&RESILIENCE_RETRY_EVENT_ID));
        assert!(TRACING_EVENT_IDS.contains(&CONTENTION_EXHAUSTED_EVENT_ID));
    }

    #[test]
    fn tracing_boundary_a_unknown_and_dynamic_event_ids_become_unclassified() {
        assert_eq!(classify_event_id("sce.app.start"), "sce.app.start");
        assert_eq!(classify_event_id("sce.test.error"), UNCLASSIFIED_EVENT_ID);
        assert_eq!(classify_event_id(SECRET), UNCLASSIFIED_EVENT_ID);
        assert_eq!(classify_event_id(SENSITIVE_PATH), UNCLASSIFIED_EVENT_ID);
        assert_eq!(
            classify_event_id("sce.error.SCE-ERR-RUNTIME"),
            UNCLASSIFIED_EVENT_ID
        );
    }

    #[test]
    fn tracing_boundary_a_logger_events_carry_only_classified_values() {
        let logger = memory_logger();
        let secret_message = format!("token={SECRET} path={SENSITIVE_PATH}");
        let secret_fields = [
            ("operation", SECRET),
            ("db_name", SENSITIVE_PATH),
            ("detail", "password=hunter2"),
        ];

        let events = CapturingSubscriber::capture(|| {
            logger.info(SECRET, &secret_message, &secret_fields, None);
            logger.warn(SENSITIVE_PATH, &secret_message, &secret_fields, None);
            logger.error("sce.unknown.event", &secret_message, &secret_fields, None);
            logger.debug("sce.app.start", &secret_message, &secret_fields, None);
        });

        assert_eq!(events.len(), 4);
        assert_clean(
            &events,
            &[SECRET, SENSITIVE_PATH, "hunter2", "password", "token="],
        );
        let ids: Vec<_> = events.iter().map(|e| e.field("event_id")).collect();
        assert_eq!(
            ids,
            vec![
                Some(UNCLASSIFIED_EVENT_ID),
                Some(UNCLASSIFIED_EVENT_ID),
                Some(UNCLASSIFIED_EVENT_ID),
                Some("sce.app.start"),
            ]
        );
    }

    #[test]
    fn tracing_boundary_a_cli_errors_map_event_id_from_typed_class_without_source_chain() {
        let logger = memory_logger();
        let error = CliError::user_with_source(
            UserError::NotAuthenticated,
            anyhow::anyhow!("open {SENSITIVE_PATH}: token {SECRET}")
                .context(format!("outer {SECRET}")),
        );

        let events = CapturingSubscriber::capture(|| logger.log_cli_error(&error, None));

        assert_eq!(events.len(), 1);
        assert_clean(&events, &[SECRET, SENSITIVE_PATH, "outer"]);
        assert_eq!(events[0].field("event_id"), Some("sce.error.runtime"));
    }

    #[test]
    fn tracing_boundary_a_local_log_output_keeps_raw_event_ids() {
        let logger = memory_logger();
        let line = logger.render_line(LogLevel::Info, "sce.test.raw", "message", &[("k", "v")]);
        assert!(line.contains("event_id=sce.test.raw"));
        assert!(line.contains("message=message"));
        assert!(line.contains("k=v"));
    }

    #[test]
    fn tracing_boundary_a_operation_and_db_name_classify_to_enums() {
        assert_eq!(
            OperationClass::classify("open local DB database connection"),
            OperationClass::DbOpenConnection
        );
        assert_eq!(
            OperationClass::classify("open encrypted auth DB database connection"),
            OperationClass::DbOpenEncryptedConnection
        );
        assert_eq!(
            OperationClass::classify("auth.refresh_token"),
            OperationClass::AuthRefreshToken
        );
        assert_eq!(
            OperationClass::classify(&format!("rm -rf {SENSITIVE_PATH}")),
            OperationClass::Unclassified
        );
        assert_eq!(OperationClass::classify(""), OperationClass::Unclassified);
        assert_eq!(DbName::classify("auth DB"), DbName::AuthDb);
        assert_eq!(DbName::classify(SECRET), DbName::Unclassified);
        assert_eq!(
            OperationClass::classify(SECRET).as_str(),
            UNCLASSIFIED_LABEL
        );
    }

    #[test]
    fn tracing_boundary_a_emission_functions_accept_only_closed_types() {
        let _: fn(LogLevel, EventId) = emit_logger_event;
        let _: fn(OperationClass, u32, u32, u64, u64) = emit_retry_event;
        let _: fn(DbName, OperationClass, u32, u64, u64, u64) = emit_contention_event;
    }

    #[test]
    fn tracing_boundary_a_event_ids_only_come_from_the_classified_registry() {
        assert_eq!(EventId::classify("sce.app.start").as_str(), "sce.app.start");
        assert_eq!(EventId::classify(SECRET).as_str(), UNCLASSIFIED_EVENT_ID);
        assert_eq!(
            EventId::classify(SENSITIVE_PATH).as_str(),
            UNCLASSIFIED_EVENT_ID
        );
        assert_eq!(
            EventId::classify("sce.unknown.event").as_str(),
            UNCLASSIFIED_EVENT_ID
        );
        assert_eq!(
            EventId::RESILIENCE_RETRY.as_str(),
            RESILIENCE_RETRY_EVENT_ID
        );
        assert_eq!(
            EventId::CONTENTION_EXHAUSTED.as_str(),
            CONTENTION_EXHAUSTED_EVENT_ID
        );
        let error = CliError::user_with_source(UserError::NotAuthenticated, anyhow::anyhow!("x"));
        assert_eq!(
            EventId::from_cli_error(&error).as_str(),
            "sce.error.runtime"
        );
    }

    #[test]
    fn tracing_boundary_a_logger_emitter_records_exact_target_and_closed_fields() {
        let events = CapturingSubscriber::capture(|| {
            for level in [
                LogLevel::Error,
                LogLevel::Warn,
                LogLevel::Info,
                LogLevel::Debug,
            ] {
                emit_logger_event(level, EventId::classify("sce.app.start"));
            }
        });

        assert_eq!(events.len(), 4);
        for (event, level) in events.iter().zip(["error", "warn", "info", "debug"]) {
            assert_eq!(event.target, SCE_TRACING_TARGET);
            let names: Vec<_> = event.fields.iter().map(|(key, _)| key.as_str()).collect();
            assert_eq!(names, vec!["message", "event_id", "log_level"]);
            assert_eq!(event.field("event_id"), Some("sce.app.start"));
            assert_eq!(event.field("log_level"), Some(level));
            assert_eq!(event.field("message"), Some("sce log event"));
        }
    }

    #[test]
    fn tracing_boundary_a_retry_and_contention_emitters_record_exact_closed_fields() {
        let events = CapturingSubscriber::capture(|| {
            emit_retry_event(OperationClass::DbQueryRows, 2, 5, 1_500, 40);
            emit_contention_event(
                DbName::RepositoryAgentTraceDb,
                OperationClass::DbCheckpointWal,
                3,
                5_000,
                30_000,
                12_345,
            );
        });

        assert_eq!(events.len(), 2);
        let retry = &events[0];
        assert_eq!(retry.target, SCE_TRACING_TARGET);
        let names: Vec<_> = retry.fields.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "message",
                "event_id",
                "operation",
                "attempt",
                "max_attempts",
                "timeout_ms",
                "backoff_ms"
            ]
        );
        assert_eq!(retry.field("event_id"), Some("sce.resilience.retry"));
        assert_eq!(retry.field("operation"), Some("db.query_rows"));
        assert_eq!(retry.field("attempt"), Some("2"));
        assert_eq!(retry.field("max_attempts"), Some("5"));
        assert_eq!(retry.field("timeout_ms"), Some("1500"));
        assert_eq!(retry.field("backoff_ms"), Some("40"));
        assert_eq!(
            retry.field("message"),
            Some("Retrying operation after transient failure")
        );

        let contention = &events[1];
        assert_eq!(contention.target, SCE_TRACING_TARGET);
        let names: Vec<_> = contention
            .fields
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "message",
                "event_id",
                "db_name",
                "operation",
                "attempts",
                "busy_timeout_ms",
                "contention_deadline_ms",
                "elapsed_ms",
                "cause"
            ]
        );
        assert_eq!(
            contention.field("event_id"),
            Some("sce.agent_trace_db.contention_exhausted")
        );
        assert_eq!(
            contention.field("db_name"),
            Some("repository_agent_trace_db")
        );
        assert_eq!(contention.field("operation"), Some("db.checkpoint_wal"));
        assert_eq!(contention.field("attempts"), Some("3"));
        assert_eq!(contention.field("busy_timeout_ms"), Some("5000"));
        assert_eq!(contention.field("contention_deadline_ms"), Some("30000"));
        assert_eq!(contention.field("elapsed_ms"), Some("12345"));
        assert_eq!(
            contention.field("cause"),
            Some("database busy (busy timeout exhausted)")
        );
    }

    #[test]
    fn tracing_boundary_a_emitters_only_export_values_from_closed_sets() {
        let events = CapturingSubscriber::capture(|| {
            for operation in OperationClass::ALL {
                emit_retry_event(operation, 1, 2, 3, 4);
                for database in [
                    DbName::LocalDb,
                    DbName::AuthDb,
                    DbName::RepositoryAgentTraceDb,
                    DbName::Unclassified,
                ] {
                    emit_contention_event(database, operation, 1, 2, 3, 4);
                }
            }
        });

        let operations: Vec<_> = OperationClass::ALL.iter().map(|o| o.as_str()).collect();
        let databases = [
            "local_db",
            "auth_db",
            "repository_agent_trace_db",
            UNCLASSIFIED_LABEL,
        ];
        assert_eq!(events.len(), OperationClass::ALL.len() * 5);
        for event in &events {
            assert!(operations.contains(&event.field("operation").expect("operation")));
            if let Some(database) = event.field("db_name") {
                assert!(databases.contains(&database));
            }
        }
    }
}
