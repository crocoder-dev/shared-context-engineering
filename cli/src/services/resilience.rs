use std::future::Future;
use std::thread;
use std::time::{Duration, Instant};

use std::fmt;

use anyhow::{anyhow, ensure, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryPolicyError {
    ZeroMaxAttempts,
    ZeroTimeout,
    MaxBackoffBelowInitial,
}

impl fmt::Display for RetryPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ZeroMaxAttempts => "Retry policy requires max_attempts >= 1",
            Self::ZeroTimeout => "Retry policy requires timeout_ms >= 1",
            Self::MaxBackoffBelowInitial => {
                "Retry policy requires max_backoff_ms >= initial_backoff_ms"
            }
        })
    }
}

impl std::error::Error for RetryPolicyError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    max_attempts: u32,
    timeout_ms: u64,
    initial_backoff_ms: u64,
    max_backoff_ms: u64,
}

impl RetryPolicy {
    pub const fn new(
        max_attempts: u32,
        timeout_ms: u64,
        initial_backoff_ms: u64,
        max_backoff_ms: u64,
    ) -> Result<Self, RetryPolicyError> {
        if max_attempts == 0 {
            return Err(RetryPolicyError::ZeroMaxAttempts);
        }
        if timeout_ms == 0 {
            return Err(RetryPolicyError::ZeroTimeout);
        }
        if max_backoff_ms < initial_backoff_ms {
            return Err(RetryPolicyError::MaxBackoffBelowInitial);
        }
        Ok(Self {
            max_attempts,
            timeout_ms,
            initial_backoff_ms,
            max_backoff_ms,
        })
    }

    pub const fn builtin(
        max_attempts: u32,
        timeout_ms: u64,
        initial_backoff_ms: u64,
        max_backoff_ms: u64,
    ) -> Self {
        match Self::new(max_attempts, timeout_ms, initial_backoff_ms, max_backoff_ms) {
            Ok(policy) => policy,
            Err(_) => panic!("invalid built-in retry policy"),
        }
    }

    pub const fn max_attempts(self) -> u32 {
        self.max_attempts
    }

    pub const fn timeout_ms(self) -> u64 {
        self.timeout_ms
    }

    pub const fn initial_backoff_ms(self) -> u64 {
        self.initial_backoff_ms
    }

    pub const fn max_backoff_ms(self) -> u64 {
        self.max_backoff_ms
    }

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

#[allow(
    dead_code,
    reason = "tracked for removal in cli-async-persistence-cleanup-pr3 T04"
)]
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

#[cfg(test)]
mod tests {
    use super::{RetryPolicy, RetryPolicyError};

    #[test]
    fn retry_policy_new_rejects_each_invalid_invariant() {
        assert_eq!(
            RetryPolicy::new(0, 100, 10, 20),
            Err(RetryPolicyError::ZeroMaxAttempts)
        );
        assert_eq!(
            RetryPolicy::new(1, 0, 10, 20),
            Err(RetryPolicyError::ZeroTimeout)
        );
        assert_eq!(
            RetryPolicy::new(1, 100, 21, 20),
            Err(RetryPolicyError::MaxBackoffBelowInitial)
        );
    }

    #[test]
    fn retry_policy_new_accepts_boundary_values() {
        let policy = RetryPolicy::new(1, 1, 0, 0).unwrap();
        assert_eq!(policy.max_attempts(), 1);
        assert_eq!(policy.timeout_ms(), 1);
        assert_eq!(policy.initial_backoff_ms(), 0);
        assert_eq!(policy.max_backoff_ms(), 0);
    }
}
