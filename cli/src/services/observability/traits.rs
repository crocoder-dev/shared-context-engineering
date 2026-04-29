use crate::services::error::ClassifiedError;

pub trait Logger {
    fn info(&self, event_id: &str, message: &str, fields: &[(&str, &str)]);

    fn debug(&self, event_id: &str, message: &str, fields: &[(&str, &str)]);

    fn warn(&self, event_id: &str, message: &str, fields: &[(&str, &str)]);

    fn error(&self, event_id: &str, message: &str, fields: &[(&str, &str)]);

    fn log_classified_error(&self, error: &ClassifiedError);
}

pub trait Telemetry: Send + Sync {
    fn with_default_subscriber(
        &self,
        action: &mut dyn FnMut() -> Result<String, ClassifiedError>,
    ) -> Result<String, ClassifiedError>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoopLogger;

impl Logger for NoopLogger {
    fn info(&self, _event_id: &str, _message: &str, _fields: &[(&str, &str)]) {}

    fn debug(&self, _event_id: &str, _message: &str, _fields: &[(&str, &str)]) {}

    fn warn(&self, _event_id: &str, _message: &str, _fields: &[(&str, &str)]) {}

    fn error(&self, _event_id: &str, _message: &str, _fields: &[(&str, &str)]) {}

    fn log_classified_error(&self, _error: &ClassifiedError) {}
}

impl Logger for super::Logger {
    fn info(&self, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        super::Logger::info(self, event_id, message, fields);
    }

    fn debug(&self, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        super::Logger::debug(self, event_id, message, fields);
    }

    fn warn(&self, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        super::Logger::warn(self, event_id, message, fields);
    }

    fn error(&self, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        super::Logger::error(self, event_id, message, fields);
    }

    fn log_classified_error(&self, error: &ClassifiedError) {
        super::Logger::log_classified_error(self, error);
    }
}

impl Telemetry for super::TelemetryRuntime {
    fn with_default_subscriber(
        &self,
        action: &mut dyn FnMut() -> Result<String, ClassifiedError>,
    ) -> Result<String, ClassifiedError> {
        super::TelemetryRuntime::with_default_subscriber(self, action)
    }
}
