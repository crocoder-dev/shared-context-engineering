use std::fmt;

use opentelemetry::logs::{AnyValue, LogRecord, Logger, Severity};
use opentelemetry_sdk::logs::SdkLogger;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::services::security::redact_sensitive_text;

pub const LOG_EVENT_TARGET: &str = "sce";
const EVENT_ID_FIELD: &str = "event_id";
const EVENT_MESSAGE_FIELD: &str = "event_message";
const FIELDS_FIELD: &str = "fields";
const EVENT_ID_ATTRIBUTE: &str = "sce.event_id";
const MAX_VALUE_CHARS: usize = 160;
const ALLOWED_FIELD_KEYS: &[&str] = &[
    "command",
    "component",
    "failure_class",
    "error_code",
    "error_class",
    "error_surface",
    "user_error",
];

pub fn is_exportable_event(metadata: &tracing::Metadata<'_>) -> bool {
    !metadata.is_span() && metadata.target() == LOG_EVENT_TARGET
}

fn sanitize_value(value: &str) -> Option<String> {
    if value.contains('/') || value.contains('\\') || value.contains('\n') {
        return None;
    }
    let redacted = redact_sensitive_text(value);
    if redacted != value {
        return None;
    }
    Some(redacted.chars().take(MAX_VALUE_CHARS).collect())
}

fn allowlisted_fields(fields_json: &str) -> Vec<(String, String)> {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(fields_json) else {
        return Vec::new();
    };
    ALLOWED_FIELD_KEYS
        .iter()
        .filter_map(|key| {
            let value = map.get(*key)?.as_str()?;
            Some(((*key).to_string(), sanitize_value(value)?))
        })
        .collect()
}

fn severity_for(level: Level) -> (Severity, &'static str) {
    match level {
        Level::ERROR => (Severity::Error, "ERROR"),
        Level::WARN => (Severity::Warn, "WARN"),
        Level::INFO => (Severity::Info, "INFO"),
        Level::DEBUG | Level::TRACE => (Severity::Debug, "DEBUG"),
    }
}

#[derive(Default)]
struct EventFields {
    event_id: Option<String>,
    message: Option<String>,
    fields_json: Option<String>,
}

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            EVENT_ID_FIELD => self.event_id = Some(value.to_string()),
            EVENT_MESSAGE_FIELD => self.message = Some(value.to_string()),
            FIELDS_FIELD => self.fields_json = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == FIELDS_FIELD {
            self.fields_json = Some(format!("{value:?}"));
        }
    }
}

pub struct LogBridgeLayer {
    logger: SdkLogger,
}

impl LogBridgeLayer {
    pub fn new(logger: SdkLogger) -> Self {
        Self { logger }
    }
}

impl<S> Layer<S> for LogBridgeLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        if !is_exportable_event(metadata) {
            return;
        }
        let mut captured = EventFields::default();
        event.record(&mut captured);
        let Some(event_id) = captured.event_id.as_deref().and_then(sanitize_value) else {
            return;
        };
        let body = captured
            .message
            .as_deref()
            .and_then(sanitize_value)
            .unwrap_or_else(|| event_id.clone());
        let (severity, severity_text) = severity_for(*metadata.level());

        let mut record = self.logger.create_log_record();
        record.set_severity_number(severity);
        record.set_severity_text(severity_text);
        record.set_body(AnyValue::String(body.into()));
        record.add_attribute(EVENT_ID_ATTRIBUTE, event_id);
        for (key, value) in captured
            .fields_json
            .as_deref()
            .map(allowlisted_fields)
            .unwrap_or_default()
        {
            record.add_attribute(format!("sce.{key}"), value);
        }
        self.logger.emit(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_keeps_only_approved_safe_fields() {
        let json = r#"{"command":"config","component":"observability","argv":"--token x","repo":"/home/u/repo","error_code":"SCE-ERR-RUNTIME","error_class":"runtime","user_error":"Bearer abc"}"#;
        let fields = allowlisted_fields(json);
        let keys = fields
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(keys, ["command", "component", "error_code", "error_class"]);
    }

    #[test]
    fn enabled_hint_for_sce_events_passes_export_filter() {
        use tracing_subscriber::filter::filter_fn;
        use tracing_subscriber::layer::SubscriberExt;

        let layer = tracing_subscriber::layer::Identity::new().with_filter(filter_fn(
            is_exportable_event as fn(&tracing::Metadata<'_>) -> bool,
        ));
        let dispatch = tracing::Dispatch::new(tracing_subscriber::Registry::default().with(layer));
        tracing::dispatcher::with_default(&dispatch, || {
            assert!(tracing::enabled!(target: "sce", Level::ERROR));
            assert!(!tracing::enabled!(target: "other", Level::ERROR));
        });
    }

    #[test]
    fn sanitize_rejects_paths_and_secrets_and_bounds_length() {
        assert_eq!(sanitize_value("/home/u/repo"), None);
        assert_eq!(sanitize_value("token=abc123"), None);
        assert_eq!(sanitize_value("Authorization: Bearer abc"), None);
        assert_eq!(
            sanitize_value(&"a".repeat(500)).map(|v| v.len()),
            Some(MAX_VALUE_CHARS)
        );
    }
}
