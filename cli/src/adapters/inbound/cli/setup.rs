//! Inbound CLI rendering for the `setup` command's use-case reports.

use crate::application::use_cases::ensure_context_baseline::EnsureContextBaselineReport;
use crate::services::style::success;

/// Renders the result of `EnsureContextBaseline::execute` as the CLI's
/// existing "Context baseline ensured." success message.
pub(crate) fn render_context_baseline_report(_report: &EnsureContextBaselineReport) -> String {
    success("Context baseline ensured.")
}
