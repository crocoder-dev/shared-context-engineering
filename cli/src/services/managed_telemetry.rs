use std::fmt;
use std::future::Future;
use std::time::Duration;

pub(crate) mod claude_helper;
pub mod log_export;
pub mod trace_export;

use crate::services::error::CliError;
use crate::services::observability::traits::{Logger, NoopTelemetry, Telemetry};
use trace_export::{ManagedSpanKind, ManagedTraceTelemetry};

use crate::services::telemetry_credential_storage::{
    load_telemetry_credential, TelemetryCredential,
};
use crate::services::telemetry_readiness::{self, ReadinessState};
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
const TELEMETRY_FALLBACK_EVENT_ID: &str = "sce.telemetry.fallback";
const TELEMETRY_FALLBACK_MESSAGE: &str =
    "Managed telemetry disabled for this invocation; continuing without export";
const FALLBACK_REASON_FIELD: &str = "reason";
const FALLBACK_ERROR_FIELD: &str = "error";

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TelemetryFallbackReason {
    NotConfigured,
    CredentialInaccessible,
    CredentialUnavailable,
    CredentialExpired,
    LookupTimeout,
    DatabaseError,
    PathResolutionError,
    ExporterInitFailed,
}

impl TelemetryFallbackReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::CredentialInaccessible => "credential_inaccessible",
            Self::CredentialUnavailable => "credential_unavailable",
            Self::CredentialExpired => "credential_expired",
            Self::LookupTimeout => "lookup_timeout",
            Self::DatabaseError => "database_error",
            Self::PathResolutionError => "path_resolution_error",
            Self::ExporterInitFailed => "exporter_init_failed",
        }
    }

    const fn is_failure(self) -> bool {
        !matches!(
            self,
            Self::NotConfigured
                | Self::CredentialInaccessible
                | Self::CredentialUnavailable
                | Self::CredentialExpired
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TelemetryFallback {
    pub(crate) reason: TelemetryFallbackReason,
    detail: Option<String>,
}

impl TelemetryFallback {
    const fn new(reason: TelemetryFallbackReason) -> Self {
        Self {
            reason,
            detail: None,
        }
    }

    fn from_storage_error(error: &TokenStorageError) -> Self {
        let reason = match error {
            TokenStorageError::CredentialAccessUnavailable => {
                return Self::new(TelemetryFallbackReason::CredentialInaccessible);
            }
            TokenStorageError::Database(_) => TelemetryFallbackReason::DatabaseError,
            TokenStorageError::PathResolution(_) => TelemetryFallbackReason::PathResolutionError,
        };
        Self {
            reason,
            detail: Some(error.to_string()),
        }
    }

    fn log(&self, logger: &impl Logger) {
        let mut fields = vec![(FALLBACK_REASON_FIELD, self.reason.as_str())];
        if let Some(detail) = &self.detail {
            fields.push((FALLBACK_ERROR_FIELD, detail.as_str()));
        }
        if self.reason.is_failure() {
            logger.warn(
                TELEMETRY_FALLBACK_EVENT_ID,
                TELEMETRY_FALLBACK_MESSAGE,
                &fields,
                None,
            );
        } else {
            logger.debug(
                TELEMETRY_FALLBACK_EVENT_ID,
                TELEMETRY_FALLBACK_MESSAGE,
                &fields,
                None,
            );
        }
    }
}

pub(crate) async fn load_credential_within_deadline<Fut>(
    lookup: Fut,
) -> Result<TelemetryCredential, TelemetryFallback>
where
    Fut: Future<Output = Result<Option<TelemetryCredential>, TokenStorageError>> + Send + 'static,
{
    match run_with_deadline(CREDENTIAL_LOOKUP_DEADLINE, lookup).await {
        None => Err(TelemetryFallback::new(
            TelemetryFallbackReason::LookupTimeout,
        )),
        Some(Err(error)) => Err(TelemetryFallback::from_storage_error(&error)),
        Some(Ok(None)) => Err(TelemetryFallback::new(
            TelemetryFallbackReason::CredentialUnavailable,
        )),
        Some(Ok(Some(credential))) => Ok(credential),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedTelemetryHealth {
    Healthy,
    Disabled(TelemetryFallbackReason),
}

pub(crate) async fn inspect_health() -> ManagedTelemetryHealth {
    let now = now_unix_seconds();
    let Ok(readiness) = telemetry_readiness::state(now) else {
        return ManagedTelemetryHealth::Disabled(TelemetryFallbackReason::PathResolutionError);
    };
    inspect_health_with(readiness, load_telemetry_credential(), now).await
}

pub(crate) async fn inspect_health_with<Fut>(
    readiness: ReadinessState,
    lookup: Fut,
    now_unix_seconds: u64,
) -> ManagedTelemetryHealth
where
    Fut: Future<Output = Result<Option<TelemetryCredential>, TokenStorageError>> + Send + 'static,
{
    match readiness {
        ReadinessState::NotConfigured => {
            ManagedTelemetryHealth::Disabled(TelemetryFallbackReason::NotConfigured)
        }
        ReadinessState::Expired => {
            ManagedTelemetryHealth::Disabled(TelemetryFallbackReason::CredentialExpired)
        }
        ReadinessState::Ready => match load_credential_within_deadline(lookup).await {
            Err(fallback) => ManagedTelemetryHealth::Disabled(fallback.reason),
            Ok(credential) => match select_mode(Some(credential), now_unix_seconds) {
                ManagedTelemetryMode::Managed { .. } => ManagedTelemetryHealth::Healthy,
                ManagedTelemetryMode::Noop => {
                    ManagedTelemetryHealth::Disabled(TelemetryFallbackReason::CredentialExpired)
                }
            },
        },
    }
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
    pub async fn resolve(span_kind: Option<ManagedSpanKind>, logger: &impl Logger) -> Self {
        let Some(span_kind) = span_kind else {
            return Self::Noop(NoopTelemetry);
        };
        let (telemetry, fallback) = Self::resolve_when_ready(
            telemetry_readiness::is_ready(now_unix_seconds()),
            load_telemetry_credential(),
            span_kind,
        )
        .await;
        if let Some(fallback) = fallback {
            fallback.log(logger);
        }
        telemetry
    }

    pub(crate) async fn resolve_when_ready<Fut>(
        ready: bool,
        lookup: Fut,
        span_kind: ManagedSpanKind,
    ) -> (Self, Option<TelemetryFallback>)
    where
        Fut: Future<Output = Result<Option<TelemetryCredential>, TokenStorageError>>
            + Send
            + 'static,
    {
        if !ready {
            return (
                Self::Noop(NoopTelemetry),
                Some(TelemetryFallback::new(
                    TelemetryFallbackReason::NotConfigured,
                )),
            );
        }
        Self::resolve_with(lookup, span_kind).await
    }

    pub(crate) async fn resolve_with<Fut>(
        lookup: Fut,
        span_kind: ManagedSpanKind,
    ) -> (Self, Option<TelemetryFallback>)
    where
        Fut: Future<Output = Result<Option<TelemetryCredential>, TokenStorageError>>
            + Send
            + 'static,
    {
        let credential = match load_credential_within_deadline(lookup).await {
            Ok(credential) => credential,
            Err(fallback) => return (Self::Noop(NoopTelemetry), Some(fallback)),
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return (Self::Noop(NoopTelemetry), None);
        };
        match select_mode(Some(credential), now_unix_seconds()) {
            ManagedTelemetryMode::Managed { bearer, endpoints } => {
                match ManagedTraceTelemetry::new(&bearer, endpoints, runtime, span_kind) {
                    Ok(telemetry) => (Self::Managed(telemetry), None),
                    Err(_) => (
                        Self::Noop(NoopTelemetry),
                        Some(TelemetryFallback::new(
                            TelemetryFallbackReason::ExporterInitFailed,
                        )),
                    ),
                }
            }
            ManagedTelemetryMode::Noop => (
                Self::Noop(NoopTelemetry),
                Some(TelemetryFallback::new(
                    TelemetryFallbackReason::CredentialExpired,
                )),
            ),
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

    #[tokio::test]
    async fn claude_hook_without_readiness_never_looks_up_credentials() {
        let looked_up = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = looked_up.clone();

        let (telemetry, fallback) = RuntimeTelemetry::resolve_when_ready(
            false,
            async move {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(None)
            },
            ManagedSpanKind::ClaudeHook,
        )
        .await;

        assert!(matches!(telemetry, RuntimeTelemetry::Noop(_)));
        let fallback = fallback.expect("disabled telemetry is diagnosed");
        assert_eq!(fallback.reason, TelemetryFallbackReason::NotConfigured);
        assert!(!fallback.reason.is_failure());
        assert!(!looked_up.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn claude_hook_with_inaccessible_credential_store_skips_quietly() {
        let (telemetry, fallback) = RuntimeTelemetry::resolve_when_ready(
            true,
            async { Err(TokenStorageError::CredentialAccessUnavailable) },
            ManagedSpanKind::ClaudeHook,
        )
        .await;

        assert!(matches!(telemetry, RuntimeTelemetry::Noop(_)));
        let fallback = fallback.expect("inaccessible credentials are diagnosed");
        assert_eq!(
            fallback.reason,
            TelemetryFallbackReason::CredentialInaccessible
        );
        assert!(!fallback.reason.is_failure());
        let output = telemetry
            .with_default_subscriber(&mut || async { Ok("hook-output".to_string()) })
            .await;
        assert_eq!(output.expect("hook result unaffected"), "hook-output");
    }

    #[tokio::test]
    async fn claude_hook_credential_failure_or_expiry_resolves_noop_with_reason() {
        let (failed, fallback) = RuntimeTelemetry::resolve_with(
            async { Err(TokenStorageError::Database("locked".to_string())) },
            ManagedSpanKind::ClaudeHook,
        )
        .await;
        assert!(matches!(failed, RuntimeTelemetry::Noop(_)));
        let fallback = fallback.expect("database failure is diagnosed");
        assert_eq!(fallback.reason, TelemetryFallbackReason::DatabaseError);
        assert!(fallback.reason.is_failure());
        assert!(fallback
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("locked")));

        let (path, fallback) = RuntimeTelemetry::resolve_with(
            async { Err(TokenStorageError::PathResolution("no home".to_string())) },
            ManagedSpanKind::ClaudeHook,
        )
        .await;
        assert!(matches!(path, RuntimeTelemetry::Noop(_)));
        assert_eq!(
            fallback.map(|fallback| fallback.reason),
            Some(TelemetryFallbackReason::PathResolutionError)
        );

        let (expired, fallback) = RuntimeTelemetry::resolve_with(
            async { Ok(Some(credential("bao-token", 1))) },
            ManagedSpanKind::ClaudeHook,
        )
        .await;
        assert!(matches!(expired, RuntimeTelemetry::Noop(_)));
        let fallback = fallback.expect("expiry is diagnosed");
        assert_eq!(fallback.reason, TelemetryFallbackReason::CredentialExpired);
        assert!(!fallback.reason.is_failure());
        assert_eq!(fallback.detail, None);

        let (missing, fallback) =
            RuntimeTelemetry::resolve_with(async { Ok(None) }, ManagedSpanKind::ClaudeHook).await;
        assert!(matches!(missing, RuntimeTelemetry::Noop(_)));
        assert_eq!(
            fallback.map(|fallback| fallback.reason),
            Some(TelemetryFallbackReason::CredentialUnavailable)
        );
    }

    fn tracked_lookup(
        result: Result<Option<TelemetryCredential>, TokenStorageError>,
    ) -> (
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        impl Future<Output = Result<Option<TelemetryCredential>, TokenStorageError>> + Send + 'static,
    ) {
        let looked_up = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = looked_up.clone();
        (looked_up, async move {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            result
        })
    }

    #[tokio::test]
    async fn health_without_readiness_or_with_expired_readiness_never_looks_up_credentials() {
        for (readiness, reason) in [
            (
                ReadinessState::NotConfigured,
                TelemetryFallbackReason::NotConfigured,
            ),
            (
                ReadinessState::Expired,
                TelemetryFallbackReason::CredentialExpired,
            ),
        ] {
            let (looked_up, lookup) = tracked_lookup(Ok(Some(credential("bao-token", NOW + 60))));
            let health = inspect_health_with(readiness, lookup, NOW).await;
            assert_eq!(health, ManagedTelemetryHealth::Disabled(reason));
            assert!(!looked_up.load(std::sync::atomic::Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn health_with_readiness_classifies_lookup_outcomes() {
        let cases: Vec<(
            Result<Option<TelemetryCredential>, TokenStorageError>,
            ManagedTelemetryHealth,
        )> = vec![
            (
                Ok(Some(credential("bao-token", NOW + 60))),
                ManagedTelemetryHealth::Healthy,
            ),
            (
                Err(TokenStorageError::CredentialAccessUnavailable),
                ManagedTelemetryHealth::Disabled(TelemetryFallbackReason::CredentialInaccessible),
            ),
            (
                Ok(None),
                ManagedTelemetryHealth::Disabled(TelemetryFallbackReason::CredentialUnavailable),
            ),
            (
                Ok(Some(credential("bao-token", NOW))),
                ManagedTelemetryHealth::Disabled(TelemetryFallbackReason::CredentialExpired),
            ),
            (
                Err(TokenStorageError::Database("locked".to_string())),
                ManagedTelemetryHealth::Disabled(TelemetryFallbackReason::DatabaseError),
            ),
            (
                Err(TokenStorageError::PathResolution("no home".to_string())),
                ManagedTelemetryHealth::Disabled(TelemetryFallbackReason::PathResolutionError),
            ),
        ];
        for (result, expected) in cases {
            let (looked_up, lookup) = tracked_lookup(result);
            let health = inspect_health_with(ReadinessState::Ready, lookup, NOW).await;
            assert_eq!(health, expected);
            assert!(looked_up.load(std::sync::atomic::Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn health_lookup_is_bounded_by_the_credential_lookup_deadline() {
        let started = std::time::Instant::now();
        let health = inspect_health_with(
            ReadinessState::Ready,
            std::future::pending::<Result<Option<TelemetryCredential>, TokenStorageError>>(),
            NOW,
        )
        .await;
        assert!(started.elapsed() < CREDENTIAL_LOOKUP_DEADLINE * 2);
        assert_eq!(
            health,
            ManagedTelemetryHealth::Disabled(TelemetryFallbackReason::LookupTimeout)
        );
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
