use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use turso::Builder;

use crate::services::local_db::ensure_agent_trace_local_db_ready_blocking;
use crate::services::output_format::OutputFormat;
use crate::services::style::{self};

pub const NAME: &str = "trace";

pub type TraceFormat = OutputFormat;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceRequest {
    pub subcommand: TraceSubcommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceSubcommand {
    Prompts(TracePromptsRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TracePromptsRequest {
    pub commit_sha: String,
    pub format: TraceFormat,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PromptTraceReport {
    commit_sha: String,
    harness_type: String,
    model_id: Option<String>,
    git_branch: Option<String>,
    prompts: Vec<PromptTraceEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PromptTraceEntry {
    turn_number: u32,
    prompt_text: String,
    prompt_length: usize,
    is_truncated: bool,
    cwd: Option<String>,
    tool_call_count: u32,
    duration_ms: i64,
    captured_at: String,
}

pub fn run_trace_subcommand(request: TraceRequest) -> Result<String> {
    match request.subcommand {
        TraceSubcommand::Prompts(request) => run_trace_prompts(&request),
    }
}

fn run_trace_prompts(request: &TracePromptsRequest) -> Result<String> {
    let working_dir = std::env::current_dir().context("Failed to determine current directory")?;
    let repository_root = resolve_repository_root(&working_dir)?;
    let db_path = ensure_agent_trace_local_db_ready_blocking()?;
    let report = load_prompt_trace_report(&db_path, &repository_root, &request.commit_sha)?;

    match request.format {
        TraceFormat::Text => Ok(render_prompt_trace_text(&report)),
        TraceFormat::Json => render_prompt_trace_json(&report),
    }
}

fn resolve_repository_root(working_dir: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(working_dir)
        .output()
        .context("Failed to execute 'git rev-parse --show-toplevel'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            String::new()
        } else {
            format!("git reported: {stderr}. ")
        };
        bail!(
            "Failed to resolve the current git repository root. {detail}Try: run 'sce trace prompts <commit-sha>' from inside a non-bare repository with persisted Agent Trace data."
        );
    }

    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn load_prompt_trace_report(
    db_path: &Path,
    repository_root: &Path,
    commit_ref: &str,
) -> Result<PromptTraceReport> {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    runtime.block_on(load_prompt_trace_report_async(
        db_path,
        repository_root,
        commit_ref,
    ))
}

async fn load_prompt_trace_report_async(
    db_path: &Path,
    repository_root: &Path,
    commit_ref: &str,
) -> Result<PromptTraceReport> {
    let db_location = db_path
        .to_str()
        .ok_or_else(|| anyhow!("Local DB path must be valid UTF-8: {}", db_path.display()))?;
    let db = Builder::new_local(db_location).build().await?;
    let conn = db.connect()?;
    conn.execute("PRAGMA foreign_keys = ON", ()).await?;

    let canonical_root = repository_root
        .canonicalize()
        .unwrap_or_else(|_| repository_root.to_path_buf())
        .to_string_lossy()
        .to_string();

    let repository_id = query_repository_id(&conn, &canonical_root).await?;
    let (commit_id, resolved_commit_sha) =
        resolve_commit_reference(&conn, repository_id, commit_ref).await?;

    let mut rows = conn
        .query(
            "SELECT prompt_text, prompt_length, is_truncated, turn_number, harness_type, model_id, cwd, git_branch, tool_call_count, duration_ms, captured_at FROM prompts WHERE commit_id = ?1 ORDER BY turn_number ASC, captured_at ASC",
            [commit_id],
        )
        .await?;

    let mut prompts = Vec::new();
    let mut harness_type = None;
    let mut model_id = None;
    let mut git_branch = None;

    while let Some(row) = rows.next().await? {
        let prompt_harness = required_text_column(&row, 4, "harness_type")?;
        if harness_type.is_none() {
            harness_type = Some(prompt_harness);
        }
        if model_id.is_none() {
            model_id = optional_text_column(&row, 5)?;
        }
        if git_branch.is_none() {
            git_branch = optional_text_column(&row, 7)?;
        }

        prompts.push(PromptTraceEntry {
            prompt_text: required_text_column(&row, 0, "prompt_text")?,
            prompt_length: usize::try_from(required_integer_column(&row, 1, "prompt_length")?)
                .context("prompt_length exceeded supported usize range")?,
            is_truncated: required_integer_column(&row, 2, "is_truncated")? != 0,
            turn_number: u32::try_from(required_integer_column(&row, 3, "turn_number")?)
                .context("turn_number exceeded supported u32 range")?,
            cwd: optional_text_column(&row, 6)?,
            tool_call_count: u32::try_from(required_integer_column(&row, 8, "tool_call_count")?)
                .context("tool_call_count exceeded supported u32 range")?,
            duration_ms: required_integer_column(&row, 9, "duration_ms")?,
            captured_at: required_text_column(&row, 10, "captured_at")?,
        });
    }

    if prompts.is_empty() {
        bail!(
            "No persisted prompt captures were found for commit '{resolved_commit_sha}'. Try: ensure the commit was created with prompt capture enabled, then rerun the command."
        );
    }

    Ok(PromptTraceReport {
        commit_sha: resolved_commit_sha,
        harness_type: harness_type.expect("prompt rows should set harness_type"),
        model_id,
        git_branch,
        prompts,
    })
}

async fn query_repository_id(conn: &turso::Connection, canonical_root: &str) -> Result<i64> {
    let mut rows = conn
        .query(
            "SELECT id FROM repositories WHERE canonical_root = ?1 LIMIT 1",
            [canonical_root],
        )
        .await?;

    let Some(row) = rows.next().await? else {
        bail!(
            "No persisted Agent Trace data was found for repository '{canonical_root}'. Try: create a traced commit in this repository before querying prompts."
        );
    };

    required_integer_column(&row, 0, "repository_id")
}

async fn resolve_commit_reference(
    conn: &turso::Connection,
    repository_id: i64,
    commit_ref: &str,
) -> Result<(i64, String)> {
    let like_pattern = format!("{commit_ref}%");
    let mut rows = conn
        .query(
            "SELECT id, commit_sha FROM commits WHERE repository_id = ?1 AND (commit_sha = ?2 OR commit_sha LIKE ?3) ORDER BY CASE WHEN commit_sha = ?2 THEN 0 ELSE 1 END, commit_sha ASC LIMIT 3",
            (repository_id, commit_ref, like_pattern.as_str()),
        )
        .await?;

    let mut matches = Vec::new();
    while let Some(row) = rows.next().await? {
        matches.push((
            required_integer_column(&row, 0, "commit_id")?,
            required_text_column(&row, 1, "commit_sha")?,
        ));
    }

    match matches.as_slice() {
        [] => bail!(
            "No persisted commit matched '{commit_ref}'. Try: pass a full commit SHA or a unique persisted prefix."
        ),
        [(commit_id, commit_sha)] => Ok((*commit_id, commit_sha.clone())),
        [(.., first_sha), (.., second_sha), ..] => bail!(
            "Commit reference '{commit_ref}' is ambiguous between '{first_sha}' and '{second_sha}'. Try: rerun with a longer commit SHA prefix."
        ),
    }
}

fn render_prompt_trace_text(report: &PromptTraceReport) -> String {
    let mut lines = vec![
        format!(
            "{}: {}",
            style::label("Commit"),
            style::value(&report.commit_sha)
        ),
        format!(
            "{}: {}",
            style::label("Harness"),
            style::value(&report.harness_type)
        ),
        format!(
            "{}: {}",
            style::label("Model"),
            style::value(report.model_id.as_deref().unwrap_or("unknown"))
        ),
        format!(
            "{}: {}",
            style::label("Branch"),
            style::value(report.git_branch.as_deref().unwrap_or("unknown"))
        ),
        format!(
            "{}: {}",
            style::label("Total prompts"),
            style::value(&report.prompts.len().to_string())
        ),
        String::new(),
    ];

    for (index, prompt) in report.prompts.iter().enumerate() {
        let mut header = format!(
            "{} {}  {}  cwd: {}  duration: {}  tools: {}",
            style::label(&format!("Turn {}", prompt.turn_number)),
            style::value(&prompt.captured_at),
            style::label("cwd:"),
            style::value(prompt.cwd.as_deref().unwrap_or("unknown")),
            style::value(&format_duration_ms(prompt.duration_ms)),
            style::value(&prompt.tool_call_count.to_string())
        );
        if prompt.is_truncated {
            use std::fmt::Write;
            // Writing to String buffer cannot fail in practice
            write!(&mut header, "  {}", style::value("[truncated]"))
                .expect("writing to String buffer should never fail");
        }
        lines.push(header);
        lines.push(format!("  {}", prompt.prompt_text.replace('\n', "\n  ")));
        if index + 1 != report.prompts.len() {
            lines.push(String::new());
        }
    }

    lines.join("\n")
}

fn render_prompt_trace_json(report: &PromptTraceReport) -> Result<String> {
    let payload = json!({
        "status": "ok",
        "command": NAME,
        "subcommand": "prompts",
        "commit": report.commit_sha,
        "harness": report.harness_type,
        "model": report.model_id,
        "branch": report.git_branch,
        "prompt_count": report.prompts.len(),
        "prompts": report.prompts.iter().map(|prompt| {
            let mut value = json!({
                "turn_number": prompt.turn_number,
                "text": prompt.prompt_text,
                "length": prompt.prompt_text.len(),
                "cwd": prompt.cwd,
                "duration_ms": prompt.duration_ms,
                "tool_call_count": prompt.tool_call_count,
                "captured_at": prompt.captured_at,
                "is_truncated": prompt.is_truncated,
            });
            if prompt.is_truncated {
                value["original_length"] = json!(prompt.prompt_length);
            }
            value
        }).collect::<Vec<_>>(),
    });

    serde_json::to_string_pretty(&payload)
        .context("failed to serialize trace prompts report to JSON")
}

fn format_duration_ms(duration_ms: i64) -> String {
    if duration_ms <= 0 {
        return String::from("0s");
    }

    let total_seconds = duration_ms / 1_000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;

    if minutes > 0 && seconds > 0 {
        format!("{minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{total_seconds}s")
    }
}

fn required_text_column(row: &turso::Row, index: usize, label: &str) -> Result<String> {
    let value = row.get_value(index)?;
    value
        .as_text()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("{label} column returned a non-text value"))
}

fn optional_text_column(row: &turso::Row, index: usize) -> Result<Option<String>> {
    let value = row.get_value(index)?;
    Ok(value.as_text().map(ToOwned::to_owned))
}

fn required_integer_column(row: &turso::Row, index: usize, label: &str) -> Result<i64> {
    let value = row.get_value(index)?;
    value
        .as_integer()
        .copied()
        .ok_or_else(|| anyhow!("{label} column returned a non-integer value"))
}
