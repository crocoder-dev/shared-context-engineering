use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::services::error::CliError;
use crate::services::plan::extract::extract;
use crate::services::plan::init::{create, PlanName};
use crate::services::plan::validate::{validate, PlanDiagnostic};

pub const STRUCTURED_PLAN_PASS: &str = "structured plan: reference integrity holds";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanCheckRequest {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanInitRequest {
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanRequest {
    Check(PlanCheckRequest),
    Init(PlanInitRequest),
}

pub struct PlanCommand {
    pub request: PlanRequest,
}

impl PlanCommand {
    pub fn execute_with_stderr(&self, stderr: &mut impl Write) -> Result<String, CliError> {
        self.execute_with(std::env::current_dir, stderr)
    }

    pub(crate) fn execute_with(
        &self,
        current_dir: impl FnOnce() -> std::io::Result<PathBuf>,
        stderr: &mut impl Write,
    ) -> Result<String, CliError> {
        match &self.request {
            PlanRequest::Check(request) => check(&request.path),
            PlanRequest::Init(request) => {
                let name = PlanName::parse(&request.name)?;
                let start = current_dir()
                    .context("Failed to read the current directory")
                    .map_err(CliError::runtime)?;
                create(&name, &start, stderr)
            }
        }
    }
}

fn check(path: &Path) -> Result<String, CliError> {
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
