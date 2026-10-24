use std::path::PathBuf;

use anyhow::Context;

use crate::services::error::CliError;
use crate::services::plan::extract::extract;
use crate::services::plan::validate::{validate, PlanDiagnostic};

pub const STRUCTURED_PLAN_PASS: &str = "structured plan: reference integrity holds";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanCheckRequest {
    pub path: PathBuf,
}

pub struct PlanCommand {
    pub request: PlanCheckRequest,
}

impl PlanCommand {
    pub fn execute(&self) -> Result<String, CliError> {
        let path = &self.request.path;
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read plan file '{}'", path.display()))
            .map_err(CliError::runtime)?;
        let plan = extract(&source)
            .with_context(|| format!("Failed to parse plan file '{}'", path.display()))
            .map_err(CliError::runtime)?;
        let report = validate(&plan);

        if !report.diagnostics.is_empty() {
            return Err(CliError::validation(render_diagnostics(
                &path.display().to_string(),
                &report.diagnostics,
            )));
        }

        Ok(STRUCTURED_PLAN_PASS.to_string())
    }
}

fn render_diagnostics(path: &str, diagnostics: &[PlanDiagnostic]) -> String {
    let mut lines = vec![format!("Plan structure check failed for '{path}':")];
    lines.extend(diagnostics.iter().map(|diagnostic| match diagnostic.line {
        Some(line) => format!("{path}:{line}: {}", diagnostic.message),
        None => format!("{path}: {}", diagnostic.message),
    }));
    lines.push(format!(
        "Try: fix the reported plan defects, then rerun 'sce plan check {path}'."
    ));
    lines.join("\n")
}
