use std::net::SocketAddr;
use std::time::Duration;

use super::resolve_flush_budget;

pub const TEST_RECEIVER_MODE: &str = "test-receiver";
pub const TEST_RECEIVER_ENDPOINT_ENV: &str = "SCE_TELEMETRY_TEST_RECEIVER_ENDPOINT";
const HEADER_OVERRIDE_ENVS: [&str; 2] = [
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
];
const HTTP_SCHEME: &str = "http://";
const TRACES_PATH: &str = "/v1/traces";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestReceiverConfig {
    pub endpoint: SocketAddr,
    pub flush_budget: Duration,
}

impl TestReceiverConfig {
    pub fn traces_endpoint(&self) -> String {
        format!("{HTTP_SCHEME}{}{TRACES_PATH}", self.endpoint)
    }
}

pub fn parse_loopback_endpoint(raw: &str) -> Option<SocketAddr> {
    let authority = raw.strip_prefix(HTTP_SCHEME)?;
    let authority = authority.strip_suffix('/').unwrap_or(authority);
    let address = authority.parse::<SocketAddr>().ok()?;
    (address.ip().is_loopback() && address.port() != 0).then_some(address)
}

pub fn resolve(env: &impl Fn(&str) -> Option<String>) -> Option<TestReceiverConfig> {
    if env(super::SCE_TELEMETRY_ENV).as_deref() != Some(TEST_RECEIVER_MODE) {
        return None;
    }
    let endpoint = parse_loopback_endpoint(&env(TEST_RECEIVER_ENDPOINT_ENV)?)?;
    if HEADER_OVERRIDE_ENVS
        .iter()
        .any(|key| env(key).is_some_and(|value| !value.is_empty()))
    {
        return None;
    }
    Some(TestReceiverConfig {
        endpoint,
        flush_budget: resolve_flush_budget(env),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn telemetry_test_receiver_accepts_only_plain_http_loopback_literals() {
        for accepted in [
            "http://127.0.0.1:4318",
            "http://127.0.0.1:4318/",
            "http://127.200.3.4:1",
            "http://[::1]:4318",
        ] {
            assert!(parse_loopback_endpoint(accepted).is_some(), "{accepted}");
        }
        for rejected in [
            "http://localhost:4318",
            "https://127.0.0.1:4318",
            "HTTP://127.0.0.1:4318",
            "http://10.0.0.1:4318",
            "http://192.0.2.1:4318",
            "http://0.0.0.0:4318",
            "http://127.0.0.1",
            "http://127.0.0.1:0",
            "http://127.0.0.1:4318/v1/traces",
            "http://127.0.0.1:4318?x=1",
            "http://user@127.0.0.1:4318",
            "http://[::ffff:127.0.0.1]:4318",
            "http://[::]:4318",
            "127.0.0.1:4318",
            "",
        ] {
            assert!(parse_loopback_endpoint(rejected).is_none(), "{rejected}");
        }
    }

    #[test]
    fn telemetry_test_receiver_resolution_is_pure_and_carries_no_credential() {
        let keys = Mutex::new(Vec::new());
        let base = env_of(&[
            ("SCE_TELEMETRY", "test-receiver"),
            (
                "SCE_TELEMETRY_TEST_RECEIVER_ENDPOINT",
                "http://127.0.0.1:4318",
            ),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "https://evil.example"),
            (
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                "https://evil.example/v1/traces",
            ),
            ("SCE_TELEMETRY_FLUSH_TIMEOUT_MS", "250"),
        ]);
        let recording = |key: &str| {
            keys.lock().unwrap().push(key.to_string());
            base(key)
        };
        let config = resolve(&recording).unwrap();
        assert_eq!(config.traces_endpoint(), "http://127.0.0.1:4318/v1/traces");
        assert_eq!(config.flush_budget, Duration::from_millis(250));
        let mut read = keys.lock().unwrap().clone();
        read.sort();
        read.dedup();
        assert_eq!(
            read,
            vec![
                "OTEL_EXPORTER_OTLP_HEADERS",
                "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
                "SCE_TELEMETRY",
                "SCE_TELEMETRY_FLUSH_TIMEOUT_MS",
                "SCE_TELEMETRY_TEST_RECEIVER_ENDPOINT",
            ]
        );
    }

    #[test]
    fn telemetry_test_receiver_resolution_requires_explicit_mode_and_valid_endpoint() {
        let endpoint = (
            "SCE_TELEMETRY_TEST_RECEIVER_ENDPOINT",
            "http://127.0.0.1:4318",
        );
        assert!(resolve(&env_of(&[endpoint])).is_none());
        assert!(resolve(&env_of(&[("SCE_TELEMETRY", "standalone"), endpoint])).is_none());
        assert!(resolve(&env_of(&[("SCE_TELEMETRY", "test-receiver")])).is_none());
        assert!(resolve(&env_of(&[
            ("SCE_TELEMETRY", "test-receiver"),
            (
                "SCE_TELEMETRY_TEST_RECEIVER_ENDPOINT",
                "http://localhost:4318"
            ),
        ]))
        .is_none());
        for header_key in HEADER_OVERRIDE_ENVS {
            assert!(
                resolve(&env_of(&[
                    ("SCE_TELEMETRY", "test-receiver"),
                    endpoint,
                    (header_key, "authorization=Bearer x"),
                ]))
                .is_none(),
                "{header_key}"
            );
        }
    }
}
