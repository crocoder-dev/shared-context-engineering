//! Read-only incremental export readers for the Agent Trace capture streams.
//!
//! This module establishes the local read/export boundary: cursor in, owned
//! wire-compatible rows out. It performs no database mutation, holds no local
//! sync cursor, and makes no network calls.

use anyhow::{bail, Result};

/// Maximum number of rows a single export reader call may return.
pub const AGENT_TRACE_EXPORT_BATCH_SIZE: usize = 500;

/// Largest integer value that round-trips exactly through an IEEE-754 double
/// (`Number.MAX_SAFE_INTEGER`).
pub const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Rejects a negative cursor.
pub fn validate_cursor(cursor: i64) -> Result<()> {
    if cursor < 0 {
        bail!("agent trace export cursor must be >= 0, got {cursor}");
    }

    Ok(())
}

/// Rejects a zero limit or a limit above [`AGENT_TRACE_EXPORT_BATCH_SIZE`].
pub fn validate_limit(limit: usize) -> Result<()> {
    if limit == 0 {
        bail!("agent trace export limit must be greater than 0");
    }

    if limit > AGENT_TRACE_EXPORT_BATCH_SIZE {
        bail!(
            "agent trace export limit {limit} exceeds maximum batch size {AGENT_TRACE_EXPORT_BATCH_SIZE}"
        );
    }

    Ok(())
}

/// Rejects a value outside `0..=JS_MAX_SAFE_INTEGER`, the range an exportable
/// numeric field must stay within to survive JSON round-trip without
/// truncation or casting.
pub fn validate_js_safe_integer(value: i64) -> Result<()> {
    if !(0..=JS_MAX_SAFE_INTEGER).contains(&value) {
        bail!("agent trace export value {value} is outside the JS-safe-integer range 0..={JS_MAX_SAFE_INTEGER}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_cursor_rejects_negative() {
        let error = validate_cursor(-1).expect_err("negative cursor should error");
        assert!(error.to_string().contains("cursor"));
    }

    #[test]
    fn validate_cursor_accepts_zero_and_positive() {
        assert!(validate_cursor(0).is_ok());
        assert!(validate_cursor(42).is_ok());
    }

    #[test]
    fn validate_limit_rejects_zero() {
        let error = validate_limit(0).expect_err("zero limit should error");
        assert!(error.to_string().contains("limit"));
    }

    #[test]
    fn validate_limit_rejects_above_batch_size() {
        let error = validate_limit(AGENT_TRACE_EXPORT_BATCH_SIZE + 1)
            .expect_err("limit above batch size should error");
        assert!(error.to_string().contains("limit"));
    }

    #[test]
    fn validate_limit_accepts_batch_size() {
        assert!(validate_limit(AGENT_TRACE_EXPORT_BATCH_SIZE).is_ok());
    }

    #[test]
    fn validate_limit_accepts_one() {
        assert!(validate_limit(1).is_ok());
    }

    #[test]
    fn validate_js_safe_integer_accepts_zero() {
        assert!(validate_js_safe_integer(0).is_ok());
    }

    #[test]
    fn validate_js_safe_integer_accepts_max_safe_integer() {
        assert!(validate_js_safe_integer(JS_MAX_SAFE_INTEGER).is_ok());
    }

    #[test]
    fn validate_js_safe_integer_rejects_above_max_safe_integer() {
        let error = validate_js_safe_integer(JS_MAX_SAFE_INTEGER + 1)
            .expect_err("value above max safe integer should error");
        assert!(error.to_string().contains("JS-safe-integer"));
    }

    #[test]
    fn validate_js_safe_integer_rejects_negative() {
        let error = validate_js_safe_integer(-1).expect_err("negative value should error");
        assert!(error.to_string().contains("JS-safe-integer"));
    }
}
