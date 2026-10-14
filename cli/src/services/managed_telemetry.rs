use std::fmt;
use std::future::Future;
use std::time::Duration;

pub(crate) mod claude_helper;
pub mod log_export;
pub mod trace_export;

use crate::services::error::CliError;
use crate::services::observability::traits::{NoopTelemetry, Telemetry};
use trace_export::ManagedTraceTelemetry;

use crate::services::telemetry_credential_storage::{
    load_telemetry_credential, TelemetryCredential,
};
use crate::services::token_storage::TokenStorageError;

pub const MANAGED_OTLP_ENDPOINT: &str = "https://otel.crocoderlab.dev";
pub const MANAGED_OTLP_PROTOCOL: &str = "http/protobuf";
pub const MANAGED_OTLP_TRACES_ENDPOINT: &str = "https://otel.crocoderlab.dev/v1/traces";
pub const MANAGED_OTLP_METRICS_ENDPOINT: &str = "https://otel.crocoderlab.dev/v1/metrics";
pub const MANAGED_OTLP_LOGS_ENDPOINT: &str = "https://otel.crocoderlab.dev/v1/logs";

#[cfg(test)]
const TRACES_PATH: &str = "/v1/traces";
#[cfg(test)]
const LOGS_PATH: &str = "/v1/logs";
const REDACTED: &str = "<redacted>";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedOtlpEndpoints {
    pub endpoint: &'static str,
    pub protocol: &'static str,
    pub traces_endpoint: &'static str,
    pub metrics_endpoint: &'static str,
    pub logs_endpoint: &'static str,
}

pub const MANAGED_OTLP_DEFAULTS: ManagedOtlpEndpoints = ManagedOtlpEndpoints {
    endpoint: MANAGED_OTLP_ENDPOINT,
    protocol: MANAGED_OTLP_PROTOCOL,
    traces_endpoint: MANAGED_OTLP_TRACES_ENDPOINT,
    metrics_endpoint: MANAGED_OTLP_METRICS_ENDPOINT,
    logs_endpoint: MANAGED_OTLP_LOGS_ENDPOINT,
};

#[cfg(test)]
pub fn derive_signal_endpoint(base: &str, path: &str) -> String {
    format!("{}{path}", base.trim_end_matches('/'))
}

#[derive(Clone, Eq, PartialEq)]
pub struct ManagedBearerToken(String);

impl ManagedBearerToken {
    pub fn authorization_header_value(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

impl fmt::Debug for ManagedBearerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ManagedBearerToken")
            .field(&REDACTED)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedTelemetryMode {
    Noop,
    Managed {
        bearer: ManagedBearerToken,
        endpoints: ManagedOtlpEndpoints,
    },
}

pub(crate) fn select_mode(
    credential: Option<TelemetryCredential>,
    now_unix_seconds: u64,
) -> ManagedTelemetryMode {
    match credential {
        Some(credential)
            if !credential.token.trim().is_empty()
                && credential.expires_at_unix_seconds > now_unix_seconds =>
        {
            ManagedTelemetryMode::Managed {
                bearer: ManagedBearerToken(credential.token),
                endpoints: MANAGED_OTLP_DEFAULTS,
            }
        }
        _ => ManagedTelemetryMode::Noop,
    }
}

pub const CREDENTIAL_LOOKUP_DEADLINE: Duration = Duration::from_secs(2);

pub(crate) async fn run_with_deadline<T, Fut>(deadline: Duration, work: Fut) -> Option<T>
where
    Fut: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let runtime = tokio::runtime::Handle::try_current().ok()?;
    let task = runtime.spawn(work);
    let abort = task.abort_handle();
    let joined = tokio::time::timeout(deadline, task).await;
    abort.abort();
    joined.ok()?.ok()
}

pub(crate) async fn load_credential_within_deadline<Fut>(lookup: Fut) -> Option<TelemetryCredential>
where
    Fut: Future<Output = Result<Option<TelemetryCredential>, TokenStorageError>> + Send + 'static,
{
    run_with_deadline(CREDENTIAL_LOOKUP_DEADLINE, lookup)
        .await
        .and_then(Result::ok)
        .flatten()
}

fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

pub enum RuntimeTelemetry {
    Noop(NoopTelemetry),
    Managed(ManagedTraceTelemetry),
}

impl RuntimeTelemetry {
    pub async fn resolve(managed_allowed: bool) -> Self {
        if !managed_allowed {
            return Self::Noop(NoopTelemetry);
        }
        Self::resolve_with(load_telemetry_credential()).await
    }

    pub(crate) async fn resolve_with<Fut>(lookup: Fut) -> Self
    where
        Fut: Future<Output = Result<Option<TelemetryCredential>, TokenStorageError>>
            + Send
            + 'static,
    {
        let credential = load_credential_within_deadline(lookup).await;
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return Self::Noop(NoopTelemetry);
        };
        match select_mode(credential, now_unix_seconds()) {
            ManagedTelemetryMode::Managed { bearer, endpoints } => {
                ManagedTraceTelemetry::new(&bearer, endpoints, runtime)
                    .map_or(Self::Noop(NoopTelemetry), Self::Managed)
            }
            ManagedTelemetryMode::Noop => Self::Noop(NoopTelemetry),
        }
    }

    pub async fn shutdown(self) {
        if let Self::Managed(telemetry) = self {
            telemetry.shutdown().await;
        }
    }
}

impl Telemetry for RuntimeTelemetry {
    async fn with_default_subscriber<F, Fut>(&self, action: &mut F) -> Result<String, CliError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<String, CliError>>,
    {
        match self {
            Self::Noop(telemetry) => telemetry.with_default_subscriber(action).await,
            Self::Managed(telemetry) => telemetry.with_default_subscriber(action).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000;

    fn credential(token: &str, expires_at_unix_seconds: u64) -> TelemetryCredential {
        TelemetryCredential {
            token: token.to_string(),
            accessor: "local-accessor".to_string(),
            expires_at_unix_seconds,
            identity: crate::services::telemetry_credential_storage::TelemetryIdentity {
                user_id: "user_a".to_string(),
                workspace_id: "workspace_a".to_string(),
            },
        }
    }

    #[test]
    fn managed_defaults_match_canonical_collector_contract() {
        assert_eq!(
            MANAGED_OTLP_DEFAULTS.endpoint,
            "https://otel.crocoderlab.dev"
        );
        assert_eq!(MANAGED_OTLP_DEFAULTS.protocol, "http/protobuf");
        assert_eq!(
            MANAGED_OTLP_DEFAULTS.traces_endpoint,
            "https://otel.crocoderlab.dev/v1/traces"
        );
        assert_eq!(
            MANAGED_OTLP_DEFAULTS.metrics_endpoint,
            "https://otel.crocoderlab.dev/v1/metrics"
        );
        assert_eq!(
            MANAGED_OTLP_DEFAULTS.logs_endpoint,
            "https://otel.crocoderlab.dev/v1/logs"
        );
    }

    #[test]
    fn signal_endpoints_derive_from_base_endpoint() {
        assert_eq!(
            derive_signal_endpoint(MANAGED_OTLP_ENDPOINT, TRACES_PATH),
            MANAGED_OTLP_TRACES_ENDPOINT
        );
        assert_eq!(
            derive_signal_endpoint("https://otel.crocoderlab.dev/", LOGS_PATH),
            MANAGED_OTLP_LOGS_ENDPOINT
        );
    }

    #[test]
    fn valid_unexpired_credential_selects_managed_mode_with_bearer_only() {
        let mode = select_mode(Some(credential("bao-token", NOW + 60)), NOW);
        match mode {
            ManagedTelemetryMode::Managed { bearer, endpoints } => {
                assert_eq!(bearer.authorization_header_value(), "Bearer bao-token");
                assert_eq!(endpoints, MANAGED_OTLP_DEFAULTS);
                assert!(!format!("{bearer:?}").contains("bao-token"));
            }
            ManagedTelemetryMode::Noop => panic!("expected managed mode"),
        }
    }

    #[test]
    fn absent_expired_or_blank_credential_selects_noop() {
        assert_eq!(select_mode(None, NOW), ManagedTelemetryMode::Noop);
        assert_eq!(
            select_mode(Some(credential("bao-token", NOW)), NOW),
            ManagedTelemetryMode::Noop
        );
        assert_eq!(
            select_mode(Some(credential("  ", NOW + 60)), NOW),
            ManagedTelemetryMode::Noop
        );
    }
}
