use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{anyhow, Context};

use crate::services::error::CliError;
use crate::services::plan::claims;
use crate::services::plan::extract::{extract, Plan};
use crate::services::plan::init::{create, resolve_repository_root, PlanName};
use crate::services::plan::pr_description::{
    decision_brief_stdout_payload, parse_arguments, render_format, BriefFormat, PlanInput,
};
use crate::services::plan::validate::{validate, PlanDiagnostic};

pub const STRUCTURED_PLAN_PASS: &str = "structured plan: reference integrity holds";
const PR_DESCRIPTION_COMMAND: &str = "sce plan pr-description";
const REVIEW_COMMAND: &str = "sce plan review";
const REGULAR_BLOB_MODES: [&str; 2] = ["100644", "100755"];
const BLOB_TYPE: &str = "blob";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanCheckRequest {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanInitRequest {
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanPrDescriptionRequest {
    pub repo: String,
    pub head: String,
    pub paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewOutput {
    Brief,
    Claims,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanReviewRequest {
    pub repo: String,
    pub head: String,
    pub paths: Vec<String>,
    pub output: ReviewOutput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanRequest {
    Check(PlanCheckRequest),
    Init(PlanInitRequest),
    PrDescription(PlanPrDescriptionRequest),
    Review(PlanReviewRequest),
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
            PlanRequest::PrDescription(request) => pr_description(request, current_dir),
            PlanRequest::Review(request) => review(request, current_dir),
        }
    }
}

fn check(path: &Path) -> Result<String, CliError> {
    load_valid_plan(path, &path.display().to_string())?;
    Ok(STRUCTURED_PLAN_PASS.to_string())
}

fn pr_description(
    request: &PlanPrDescriptionRequest,
    current_dir: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<String, CliError> {
    let arguments = parse_arguments(&request.repo, &request.head, &request.paths)?;
    let start = current_dir()
        .context("Failed to read the current directory")
        .map_err(CliError::runtime)?;
    let root = resolve_repository_root(&start, PR_DESCRIPTION_COMMAND)?;
    let plans = arguments
        .paths
        .iter()
        .map(|path| {
            Ok(PlanInput {
                path: path.clone(),
                plan: load_valid_plan(&Path::new(&root).join(path), path)?,
            })
        })
        .collect::<Result<Vec<_>, CliError>>()?;
    render_format(&arguments, &plans, BriefFormat::IntentReview)
}

fn review(
    request: &PlanReviewRequest,
    current_dir: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<String, CliError> {
    let arguments = parse_arguments(&request.repo, &request.head, &request.paths)?;
    let start = current_dir()
        .context("Failed to read the current directory")
        .map_err(CliError::runtime)?;
    let root = resolve_repository_root(&start, REVIEW_COMMAND)?;
    verify_commit(&root, &arguments.head)?;
    let plans = arguments
        .paths
        .iter()
        .map(|path| {
            let source = read_committed_plan(&root, &arguments.head, path)?;
            Ok(PlanInput {
                path: path.clone(),
                plan: parse_valid_plan(&source, &format!("{}:{path}", arguments.head), path)?,
            })
        })
        .collect::<Result<Vec<_>, CliError>>()?;
    match request.output {
        ReviewOutput::Brief => render_format(&arguments, &plans, BriefFormat::DecisionBrief)
            .map(decision_brief_stdout_payload),
        ReviewOutput::Claims => claims::render(&arguments, &plans),
    }
}

fn run_git(root: &str, args: &[&str]) -> Result<Output, CliError> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| {
            format!(
                "Failed to run 'git {}'. Try: install Git and rerun '{REVIEW_COMMAND}'.",
                args.join(" ")
            )
        })
        .map_err(CliError::runtime)
}

fn verify_commit(root: &str, head: &str) -> Result<(), CliError> {
    let revision = format!("{head}^{{commit}}");
    let output = run_git(root, &["rev-parse", "--verify", "--quiet", &revision])?;
    if !output.status.success() {
        return Err(CliError::runtime(anyhow!(
            "Commit '{head}' is not available in the local repository. Try: fetch the commit, then rerun '{REVIEW_COMMAND}'."
        )));
    }
    Ok(())
}

fn read_committed_plan(root: &str, head: &str, path: &str) -> Result<String, CliError> {
    let listing = run_git(root, &["ls-tree", "-z", head, "--", path])?;
    if !listing.status.success() {
        return Err(CliError::runtime(anyhow!(
            "Failed to list plan file '{path}' at commit '{head}' ({}). Try: rerun '{REVIEW_COMMAND}' inside the repository that contains the commit.",
            String::from_utf8_lossy(&listing.stderr).trim()
        )));
    }

    let entry = listing
        .stdout
        .split(|byte| *byte == 0)
        .find_map(|entry| parse_tree_entry(entry).filter(|entry| entry.name == path))
        .ok_or_else(|| {
            CliError::runtime(anyhow!(
                "Plan file '{path}' does not exist at commit '{head}'. Try: commit the plan file, or pass a path that exists at '{head}'."
            ))
        })?;
    if entry.kind != BLOB_TYPE || !REGULAR_BLOB_MODES.contains(&entry.mode) {
        return Err(CliError::runtime(anyhow!(
            "Plan path '{path}' at commit '{head}' is not a regular file (mode {}, type {}). Try: commit the plan as a regular file, not a symlink or submodule.",
            entry.mode,
            entry.kind
        )));
    }

    let blob = run_git(root, &["cat-file", BLOB_TYPE, entry.object])?;
    if !blob.status.success() {
        return Err(CliError::runtime(anyhow!(
            "Failed to read plan file '{path}' at commit '{head}' ({}). Try: rerun '{REVIEW_COMMAND}' after 'git fsck' reports no missing objects.",
            String::from_utf8_lossy(&blob.stderr).trim()
        )));
    }
    String::from_utf8(blob.stdout).map_err(|_| {
        CliError::runtime(anyhow!(
            "Plan file '{path}' at commit '{head}' is not valid UTF-8. Try: commit the plan as UTF-8 text."
        ))
    })
}

struct TreeEntry<'a> {
    mode: &'a str,
    kind: &'a str,
    object: &'a str,
    name: &'a str,
}

fn parse_tree_entry(entry: &[u8]) -> Option<TreeEntry<'_>> {
    let entry = std::str::from_utf8(entry).ok()?;
    let (metadata, name) = entry.split_once('\t')?;
    let mut fields = metadata.split(' ');
    let (mode, kind, object) = (fields.next()?, fields.next()?, fields.next()?);
    if fields.next().is_some() {
        return None;
    }
    Some(TreeEntry {
        mode,
        kind,
        object,
        name,
    })
}

fn load_valid_plan(path: &Path, label: &str) -> Result<Plan, CliError> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read plan file '{}'", path.display()))
        .map_err(CliError::runtime)?;
    parse_valid_plan(&source, &path.display().to_string(), label)
}

fn parse_valid_plan(source: &str, source_name: &str, label: &str) -> Result<Plan, CliError> {
    let plan = extract(source)
        .with_context(|| format!("Failed to parse plan file '{source_name}'"))
        .map_err(CliError::runtime)?;
    let report = validate(&plan);

    if !report.diagnostics.is_empty() {
        return Err(CliError::validation(render_diagnostics(
            label,
            &report.diagnostics,
        )));
    }

    Ok(plan)
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
