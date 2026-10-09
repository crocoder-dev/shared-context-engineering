use crate::services::error::{CliError, FailureClass};

pub const SCE_TRACING_TARGET: &str = "sce";
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
mod tests {
    use std::{fs, path::Path};

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

    const ALLOWED_TRACING_FIELDS: &[&str] = &[
        "target",
        "event_id",
        "log_level",
        "operation",
        "attempt",
        "max_attempts",
        "timeout_ms",
        "backoff_ms",
        "db_name",
        "attempts",
        "busy_timeout_ms",
        "contention_deadline_ms",
        "elapsed_ms",
        "cause",
    ];

    const EXPECTED_TRACING_SITES: &[(&str, usize)] = &[
        ("services/db/mod.rs", 1),
        ("services/observability.rs", 4),
        ("services/resilience.rs", 2),
    ];

    fn collect_rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).expect("read src dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                collect_rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    fn macro_invocation_bodies(source: &str) -> Vec<String> {
        let mut bodies = Vec::new();
        for level in ["error", "warn", "info", "debug", "trace", "event", "span"] {
            let needle = format!("{}::{level}!(", "tracing");
            let mut offset = 0;
            while let Some(found) = source[offset..].find(&needle) {
                let start = offset + found + needle.len();
                let mut depth = 1;
                let mut end = start;
                for (index, ch) in source[start..].char_indices() {
                    match ch {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                end = start + index;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                bodies.push(source[start..end].to_string());
                offset = end;
            }
        }
        bodies
    }

    fn top_level_arguments(body: &str) -> Vec<String> {
        let mut arguments = Vec::new();
        let mut current = String::new();
        let mut depth = 0_i32;
        let mut in_string = false;
        let mut previous = '\0';
        for ch in body.chars() {
            if in_string {
                current.push(ch);
                if ch == '"' && previous != '\\' {
                    in_string = false;
                }
            } else {
                match ch {
                    '"' => {
                        in_string = true;
                        current.push(ch);
                    }
                    '(' | '[' | '{' => {
                        depth += 1;
                        current.push(ch);
                    }
                    ')' | ']' | '}' => {
                        depth -= 1;
                        current.push(ch);
                    }
                    ',' if depth == 0 => arguments.push(std::mem::take(&mut current)),
                    _ => current.push(ch),
                }
            }
            previous = ch;
        }
        if !current.trim().is_empty() {
            arguments.push(current);
        }
        arguments
            .into_iter()
            .map(|argument| argument.trim().to_string())
            .filter(|argument| !argument.is_empty())
            .collect()
    }

    #[test]
    fn tracing_boundary_a_every_tracing_macro_site_records_only_classified_fields() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        collect_rust_files(&src, &mut files);
        files.sort();

        let mut inventory: Vec<(String, usize)> = Vec::new();
        for file in files {
            let relative = file
                .strip_prefix(&src)
                .expect("relative path")
                .to_string_lossy()
                .replace('\\', "/");
            if relative == "services/observability/tracing_boundary.rs" {
                continue;
            }
            let source = fs::read_to_string(&file).expect("read source");
            let bodies = macro_invocation_bodies(&source);
            if bodies.is_empty() {
                continue;
            }

            for body in &bodies {
                let arguments = top_level_arguments(body);
                let (message, fields) = arguments.split_last().expect("macro arguments");
                assert!(
                    message.starts_with('"'),
                    "{relative}: message argument must be a static literal, got {message}"
                );
                for field in fields.iter().filter(|field| !field.starts_with("target:")) {
                    let (name, value) = field.split_once('=').unwrap_or((field.as_str(), ""));
                    let name = name.trim();
                    assert!(
                        ALLOWED_TRACING_FIELDS.contains(&name),
                        "{relative}: tracing field {name:?} is not classified"
                    );
                    let value = value.trim();
                    assert!(
                        !value.starts_with('%') && !value.starts_with('?'),
                        "{relative}: tracing field {name:?} uses Display/Debug capture"
                    );
                }
            }
            inventory.push((relative, bodies.len()));
        }

        let expected: Vec<(String, usize)> = EXPECTED_TRACING_SITES
            .iter()
            .map(|(path, count)| ((*path).to_string(), *count))
            .collect();
        assert_eq!(inventory, expected);
    }
}
