use anyhow::{bail, Result};
use serde_json::json;

pub const NAME: &str = "observability";

const ENV_LOG_LEVEL: &str = "SCE_LOG_LEVEL";
const ENV_LOG_FORMAT: &str = "SCE_LOG_FORMAT";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogFormat {
    Text,
    Json,
}

impl LogFormat {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => bail!(
                "Invalid {} '{}'. Valid values: text, json.",
                ENV_LOG_FORMAT,
                raw
            ),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "error" => Ok(Self::Error),
            "warn" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            _ => bail!(
                "Invalid {} '{}'. Valid values: error, warn, info, debug.",
                ENV_LOG_LEVEL,
                raw
            ),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }

    fn severity(self) -> u8 {
        match self {
            Self::Error => 1,
            Self::Warn => 2,
            Self::Info => 3,
            Self::Debug => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservabilityConfig {
    pub level: LogLevel,
    pub format: LogFormat,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            format: LogFormat::Text,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Logger {
    config: ObservabilityConfig,
}

impl Logger {
    pub fn from_env() -> Result<Self> {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    fn from_env_lookup<F>(lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut config = ObservabilityConfig::default();

        if let Some(raw) = lookup(ENV_LOG_LEVEL) {
            config.level = LogLevel::parse(&raw)?;
        }

        if let Some(raw) = lookup(ENV_LOG_FORMAT) {
            config.format = LogFormat::parse(&raw)?;
        }

        Ok(Self { config })
    }

    pub fn info(&self, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        self.log(LogLevel::Info, event_id, message, fields);
    }

    pub fn error(&self, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        self.log(LogLevel::Error, event_id, message, fields);
    }

    fn log(&self, level: LogLevel, event_id: &str, message: &str, fields: &[(&str, &str)]) {
        if !self.enabled(level) {
            return;
        }

        eprintln!("{}", self.render_line(level, event_id, message, fields));
    }

    fn enabled(&self, level: LogLevel) -> bool {
        level.severity() <= self.config.level.severity()
    }

    fn render_line(
        &self,
        level: LogLevel,
        event_id: &str,
        message: &str,
        fields: &[(&str, &str)],
    ) -> String {
        match self.config.format {
            LogFormat::Text => {
                let mut line = format!(
                    "log_format={} level={} event_id={} message={}",
                    self.config.format.as_str(),
                    level.as_str(),
                    event_id,
                    message
                );

                for (key, value) in fields {
                    line.push(' ');
                    line.push_str(key);
                    line.push('=');
                    line.push_str(value);
                }

                line
            }
            LogFormat::Json => {
                let details = fields
                    .iter()
                    .map(|(key, value)| {
                        (
                            (*key).to_string(),
                            serde_json::Value::String((*value).to_string()),
                        )
                    })
                    .collect::<serde_json::Map<String, serde_json::Value>>();
                json!({
                    "log_format": self.config.format.as_str(),
                    "level": level.as_str(),
                    "event_id": event_id,
                    "message": message,
                    "fields": details,
                })
                .to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LogFormat, LogLevel, Logger};

    #[test]
    fn logger_defaults_to_info_text() {
        let logger = Logger::from_env_lookup(|_| None).expect("logger should parse defaults");
        let line = logger.render_line(LogLevel::Info, "sce.test.event", "hello", &[]);
        assert_eq!(
            line,
            "log_format=text level=info event_id=sce.test.event message=hello"
        );
    }

    #[test]
    fn logger_parses_env_level_and_format() {
        let logger = Logger::from_env_lookup(|key| match key {
            "SCE_LOG_LEVEL" => Some("debug".to_string()),
            "SCE_LOG_FORMAT" => Some("json".to_string()),
            _ => None,
        })
        .expect("logger should parse env");

        let line = logger.render_line(
            LogLevel::Info,
            "sce.test.event",
            "hello",
            &[("command", "setup")],
        );
        assert_eq!(
            line,
            "{\"event_id\":\"sce.test.event\",\"fields\":{\"command\":\"setup\"},\"level\":\"info\",\"log_format\":\"json\",\"message\":\"hello\"}"
        );
    }

    #[test]
    fn logger_rejects_invalid_level() {
        let error = Logger::from_env_lookup(|key| {
            if key == "SCE_LOG_LEVEL" {
                return Some("trace".to_string());
            }
            None
        })
        .expect_err("invalid level should fail");

        assert_eq!(
            error.to_string(),
            "Invalid SCE_LOG_LEVEL 'trace'. Valid values: error, warn, info, debug."
        );
    }

    #[test]
    fn logger_level_filtering_is_threshold_based() {
        let logger = Logger::from_env_lookup(|key| {
            if key == "SCE_LOG_LEVEL" {
                return Some("warn".to_string());
            }
            None
        })
        .expect("logger should parse warn level");

        assert!(logger.enabled(LogLevel::Error));
        assert!(logger.enabled(LogLevel::Warn));
        assert!(!logger.enabled(LogLevel::Info));
        assert!(!logger.enabled(LogLevel::Debug));
    }

    #[test]
    fn log_format_parser_accepts_documented_values() {
        assert_eq!(
            LogFormat::parse("text").expect("text should parse"),
            LogFormat::Text
        );
        assert_eq!(
            LogFormat::parse("json").expect("json should parse"),
            LogFormat::Json
        );
    }
}
