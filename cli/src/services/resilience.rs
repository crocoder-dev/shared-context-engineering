use std::future::Future;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};

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
            event_id = "sce.resilience.retry",
            operation = operation_name,
            attempt,
            max_attempts = policy.max_attempts,
            timeout_ms = policy.timeout_ms,
            backoff_ms = u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX),
            error = %last_error,
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
