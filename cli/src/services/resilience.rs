use std::future::Future;
use std::thread;
use std::time::{Duration, Instant};

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

#[allow(dead_code)]
pub fn run_with_retry_sync<T, Op>(
    policy: RetryPolicy,
    operation_name: &str,
    retry_hint: &str,
    mut operation: Op,
) -> Result<T>
where
    Op: FnMut(u32) -> Result<T>,
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
        let outcome = operation(attempt);
        let timed_out = started_at.elapsed() >= policy.timeout();

        match (timed_out, outcome) {
            (false, Ok(value)) => return Ok(value),
            (true, _) => {
                last_error = format!("attempt {attempt} timed out after {}ms", policy.timeout_ms);
            }
            (false, Err(error)) => {
                last_error = error.to_string();
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
        thread::sleep(backoff);
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

    #[test]
    fn sync_retry_succeeds_after_transient_failures() {
        let policy = RetryPolicy {
            max_attempts: 3,
            timeout_ms: 1_000,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
        };
        let mut attempts = 0;

        let result =
            run_with_retry_sync(policy, "sync success", "retry the operation", |attempt| {
                attempts = attempt;
                if attempt < 3 {
                    return Err(anyhow!("transient failure {attempt}"));
                }

                Ok("ok")
            });

        assert_eq!(result.unwrap(), "ok");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn sync_retry_reports_exhausted_failures_with_guidance() {
        let policy = RetryPolicy {
            max_attempts: 2,
            timeout_ms: 1_000,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
        };
        let mut attempts = 0;

        let error =
            run_with_retry_sync::<(), _>(policy, "sync failure", "retry later", |attempt| {
                attempts = attempt;
                Err(anyhow!("nope {attempt}"))
            })
            .unwrap_err();

        assert_eq!(attempts, 2);
        let message = error.to_string();
        assert!(message.contains("Operation 'sync failure' failed after 2 attempt(s)"));
        assert!(message.contains("Last error: nope 2"));
        assert!(message.contains("Try: retry later"));
    }

    #[test]
    fn sync_retry_treats_slow_attempt_as_timeout() {
        let policy = RetryPolicy {
            max_attempts: 1,
            timeout_ms: 5,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
        };
        let mut attempts = 0;

        let error = run_with_retry_sync(
            policy,
            "sync timeout",
            "try again when the resource is available",
            |attempt| {
                attempts = attempt;
                thread::sleep(Duration::from_millis(20));
                Ok("late success")
            },
        )
        .unwrap_err();

        assert_eq!(attempts, 1);
        assert!(error
            .to_string()
            .contains("Last error: attempt 1 timed out after 5ms"));
    }

    #[test]
    fn retry_policy_backoff_is_exponential_and_capped() {
        let policy = RetryPolicy {
            max_attempts: 5,
            timeout_ms: 1_000,
            initial_backoff_ms: 5,
            max_backoff_ms: 12,
        };

        assert_eq!(policy.backoff_for_attempt(1), Duration::from_millis(0));
        assert_eq!(policy.backoff_for_attempt(2), Duration::from_millis(5));
        assert_eq!(policy.backoff_for_attempt(3), Duration::from_millis(10));
        assert_eq!(policy.backoff_for_attempt(4), Duration::from_millis(12));
        assert_eq!(policy.backoff_for_attempt(5), Duration::from_millis(12));
    }
}
