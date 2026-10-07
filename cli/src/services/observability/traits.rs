use std::future::Future;

use crate::services::error::CliError;

pub trait Logger: Send + Sync {
    fn info(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    );

    fn debug(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    );

    fn warn(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    );

    fn error(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    );

    fn log_cli_error(&self, error: &CliError, session_id: Option<&str>);
}

pub trait Telemetry: Send + Sync {
    async fn with_default_subscriber<F, Fut>(&self, action: &mut F) -> Result<String, CliError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<String, CliError>>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(dead_code)]
pub struct NoopLogger;

impl Logger for NoopLogger {
    fn info(
        &self,
        _event_id: &str,
        _message: &str,
        _fields: &[(&str, &str)],
        _session_id: Option<&str>,
    ) {
    }

    fn debug(
        &self,
        _event_id: &str,
        _message: &str,
        _fields: &[(&str, &str)],
        _session_id: Option<&str>,
    ) {
    }

    fn warn(
        &self,
        _event_id: &str,
        _message: &str,
        _fields: &[(&str, &str)],
        _session_id: Option<&str>,
    ) {
    }

    fn error(
        &self,
        _event_id: &str,
        _message: &str,
        _fields: &[(&str, &str)],
        _session_id: Option<&str>,
    ) {
    }

    fn log_cli_error(&self, _error: &CliError, _session_id: Option<&str>) {}
}

impl Logger for super::Logger {
    fn info(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        super::Logger::info(self, event_id, message, fields, session_id);
    }

    fn debug(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        super::Logger::debug(self, event_id, message, fields, session_id);
    }

    fn warn(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        super::Logger::warn(self, event_id, message, fields, session_id);
    }

    fn error(
        &self,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
        session_id: Option<&str>,
    ) {
        super::Logger::error(self, event_id, message, fields, session_id);
    }

    fn log_cli_error(&self, error: &CliError, session_id: Option<&str>) {
        super::Logger::log_cli_error(self, error, session_id);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoopTelemetry;

impl Telemetry for NoopTelemetry {
    async fn with_default_subscriber<F, Fut>(&self, action: &mut F) -> Result<String, CliError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<String, CliError>>,
    {
        action().await
    }
}
