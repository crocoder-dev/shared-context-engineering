use std::future::Future;
use std::time::{Duration, Instant};

use anyhow::{anyhow, ensure, Result};

use crate::services::observability::tracing_boundary::{OperationClass, RESILIENCE_RETRY_EVENT_ID};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub timeout_ms: u64,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl RetryPolicy {
    fn timeout(self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }

    fn backoff_for_attempt(self, attempt: u32) -> Duration {
        if attempt <= 1 {
            return Duration::from_millis(0);
        }

        let exponent = (attempt - 2).min(20);
        let multiplier = 1_u64 << exponent;
        let backoff_ms = self
            .initial_backoff_ms
            .saturating_mul(multiplier)
            .min(self.max_backoff_ms);
        Duration::from_millis(backoff_ms)
    }
}

pub async fn run_with_retry<T, Op, Fut>(
    policy: RetryPolicy,
    operation_name: &str,
    retry_hint: &str,
    mut operation: Op,
) -> Result<T>
where
    Op: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    ensure!(
        policy.max_attempts > 0,
        "Retry policy requires max_attempts >= 1"
    );
    ensure!(
        policy.timeout_ms > 0,
        "Retry policy requires timeout_ms >= 1"
    );
    ensure!(
        policy.max_backoff_ms >= policy.initial_backoff_ms,
        "Retry policy requires max_backoff_ms >= initial_backoff_ms"
    );

    let mut last_error = String::new();

    for attempt in 1..=policy.max_attempts {
        let outcome = tokio::time::timeout(policy.timeout(), operation(attempt)).await;
        match outcome {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) => {
                last_error = error.to_string();
            }
            Err(_) => {
                last_error = format!("attempt {attempt} timed out after {}ms", policy.timeout_ms);
            }
        }

        if attempt == policy.max_attempts {
            break;
        }

        let backoff = policy.backoff_for_attempt(attempt + 1);
        tracing::warn!(
            event_id = RESILIENCE_RETRY_EVENT_ID,
            operation = OperationClass::classify(operation_name).as_str(),
            attempt,
            max_attempts = policy.max_attempts,
            timeout_ms = policy.timeout_ms,
            backoff_ms = u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX),
            "Retrying operation after transient failure"
        );
        tokio::time::sleep(backoff).await;
    }

    Err(anyhow!(
        "Operation '{operation_name}' failed after {} attempt(s) (timeout={}ms, backoff={}..{}ms). Last error: {}. Try: {}",
        policy.max_attempts,
        policy.timeout_ms,
        policy.initial_backoff_ms,
        policy.max_backoff_ms,
        last_error,
        retry_hint
    ))
}

/// One retryable attempt. Closures returning a future implement it directly;
/// operations that must lend an exclusive borrow (such as a transaction on a
/// single connection) implement it on a struct, because an `FnMut` closure
/// cannot return a future that borrows its own captured `&mut` state.
pub trait RetryOperation<T> {
    async fn run(&mut self, attempt: u32) -> Result<T>;
}

impl<T, F, Fut> RetryOperation<T> for F
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    async fn run(&mut self, attempt: u32) -> Result<T> {
        self(attempt).await
    }
}

pub async fn run_with_retry_elapsed<T, Op>(
    policy: RetryPolicy,
    operation_name: &str,
    retry_hint: &str,
    mut operation: Op,
) -> Result<T>
where
    Op: RetryOperation<T>,
{
    ensure!(
        policy.max_attempts > 0,
        "Retry policy requires max_attempts >= 1"
    );
    ensure!(
        policy.timeout_ms > 0,
        "Retry policy requires timeout_ms >= 1"
    );
    ensure!(
        policy.max_backoff_ms >= policy.initial_backoff_ms,
        "Retry policy requires max_backoff_ms >= initial_backoff_ms"
    );

    let mut last_error = String::new();

    for attempt in 1..=policy.max_attempts {
        let started_at = Instant::now();
        let outcome = operation.run(attempt).await;

        match outcome {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = if started_at.elapsed() >= policy.timeout() {
                    format!(
                        "attempt {attempt} exceeded {}ms and failed: {error}",
                        policy.timeout_ms
                    )
                } else {
                    error.to_string()
                };
            }
        }

        if attempt == policy.max_attempts {
            break;
        }

        let backoff = policy.backoff_for_attempt(attempt + 1);
        tracing::warn!(
            event_id = RESILIENCE_RETRY_EVENT_ID,
            operation = OperationClass::classify(operation_name).as_str(),
            attempt,
            max_attempts = policy.max_attempts,
            timeout_ms = policy.timeout_ms,
            backoff_ms = u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX),
            "Retrying operation after transient failure"
        );
        tokio::time::sleep(backoff).await;
    }

    Err(anyhow!(
        "Operation '{operation_name}' failed after {} attempt(s) (timeout={}ms, backoff={}..{}ms). Last error: {}. Try: {}",
        policy.max_attempts,
        policy.timeout_ms,
        policy.initial_backoff_ms,
        policy.max_backoff_ms,
        last_error,
        retry_hint
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::observability::tracing_boundary::test_capture::CapturingSubscriber;

    const SECRET: &str = "sk-live-SECRET-TOKEN-9f8e7d";
    const SENSITIVE_PATH: &str = "/home/victim/.config/sce/auth.json";

    #[test]
    fn tracing_boundary_a_retry_event_omits_error_text_and_classifies_operation() {
        let policy = RetryPolicy {
            max_attempts: 2,
            timeout_ms: 1_000,
            initial_backoff_ms: 1,
            max_backoff_ms: 1,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");

        let events = CapturingSubscriber::capture(|| {
            runtime.block_on(async {
                let _ = run_with_retry(policy, SENSITIVE_PATH, "retry later", |_| async {
                    Err::<(), _>(anyhow!("open {SENSITIVE_PATH} with {SECRET}"))
                })
                .await;
                let _ = run_with_retry(policy, "auth.refresh_token", "retry later", |_| async {
                    Err::<(), _>(anyhow!("denied {SECRET}"))
                })
                .await;
            });
        });

        assert_eq!(events.len(), 2);
        for event in &events {
            let rendered = event.rendered();
            for needle in [SECRET, SENSITIVE_PATH] {
                assert!(!rendered.contains(needle), "leaked {needle}: {rendered}");
            }
            assert!(event.field("error").is_none());
            assert_eq!(event.field("event_id"), Some("sce.resilience.retry"));
        }
        assert_eq!(events[0].field("operation"), Some("unclassified"));
        assert_eq!(events[1].field("operation"), Some("auth.refresh_token"));
    }
}
