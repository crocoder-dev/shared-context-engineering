use std::fmt;

use crate::services::telemetry_credential_storage::{
    load_telemetry_credential, TelemetryCredential,
};

pub const MANAGED_OTLP_ENDPOINT: &str = "https://otel.crocoderlab.dev";
pub const MANAGED_OTLP_PROTOCOL: &str = "http/protobuf";
pub const MANAGED_OTLP_TRACES_ENDPOINT: &str = "https://otel.crocoderlab.dev/v1/traces";
pub const MANAGED_OTLP_METRICS_ENDPOINT: &str = "https://otel.crocoderlab.dev/v1/metrics";
pub const MANAGED_OTLP_LOGS_ENDPOINT: &str = "https://otel.crocoderlab.dev/v1/logs";

#[allow(dead_code)]
const TRACES_PATH: &str = "/v1/traces";
#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn derive_signal_endpoint(base: &str, path: &str) -> String {
    format!("{}{path}", base.trim_end_matches('/'))
}

#[derive(Clone, Eq, PartialEq)]
#[allow(dead_code)]
pub struct ManagedBearerToken(String);

impl ManagedBearerToken {
    #[allow(dead_code)]
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

fn select_mode(
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

#[allow(dead_code)]
pub async fn resolve_managed_telemetry_mode() -> ManagedTelemetryMode {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    match load_telemetry_credential().await {
        Ok(credential) => select_mode(credential, now),
        Err(_) => ManagedTelemetryMode::Noop,
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
