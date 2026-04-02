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

#[cfg(test)]
pub(crate) fn render_prompt_trace_for_test(
    db_path: &Path,
    repository_root: &Path,
    commit_ref: &str,
    format: TraceFormat,
) -> Result<String> {
    let report = load_prompt_trace_report(db_path, repository_root, commit_ref)?;
    match format {
        TraceFormat::Text => Ok(render_prompt_trace_text(&report)),
        TraceFormat::Json => render_prompt_trace_json(&report),
    }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::Result;
    use serde_json::Value;

    use super::{
        load_prompt_trace_report, render_prompt_trace_json, render_prompt_trace_text,
        resolve_commit_reference, PromptTraceEntry, PromptTraceReport,
    };
    use crate::services::local_db::{apply_core_schema_migrations, LocalDatabaseTarget};

    #[test]
    fn render_prompt_trace_text_includes_metadata_and_prompt_rows() {
        // Test with NO_COLOR to ensure deterministic output assertions
        std::env::set_var("NO_COLOR", "1");
        let output = render_prompt_trace_text(&sample_report());
        std::env::remove_var("NO_COLOR");

        assert!(output.contains("Commit: abc1234def5678"));
        assert!(output.contains("Harness: claude_code"));
        assert!(output.contains("Branch: feature/prompts"));
        assert!(output.contains("Turn 2"));
        assert!(output.contains("2026-03-18T10:02:00Z"));
        assert!(output.contains("cwd: src"));
        assert!(output.contains("duration: 1m 30s"));
        assert!(output.contains("tools: 1"));
        assert!(output.contains("[truncated]"));
        assert!(output.contains("second prompt"));
    }

    #[test]
    fn render_prompt_trace_json_includes_original_length_for_truncated_prompts() -> Result<()> {
        let output = render_prompt_trace_json(&sample_report())?;
        let parsed: Value = serde_json::from_str(&output)?;

        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["command"], "trace");
        assert_eq!(parsed["subcommand"], "prompts");
        assert_eq!(parsed["prompt_count"], 2);
        assert_eq!(parsed["prompts"][0]["length"], 12);
        assert!(parsed["prompts"][0].get("original_length").is_none());
        assert_eq!(parsed["prompts"][1]["original_length"], 40);
        Ok(())
    }

    #[test]
    fn load_prompt_trace_report_resolves_unique_commit_prefix() -> Result<()> {
        let (db_path, repository_root) = seeded_prompt_db()?;

        let report = load_prompt_trace_report(&db_path, &repository_root, "abc1234")?;

        assert_eq!(report.commit_sha, "abc1234def5678");
        assert_eq!(report.prompts.len(), 2);
        Ok(())
    }

    #[test]
    fn load_prompt_trace_report_rejects_missing_commit() -> Result<()> {
        let (db_path, repository_root) = seeded_prompt_db()?;

        let error = load_prompt_trace_report(&db_path, &repository_root, "missing")
            .expect_err("missing commit should fail");
        assert!(error
            .to_string()
            .contains("No persisted commit matched 'missing'"));
        Ok(())
    }

    #[test]
    fn resolve_commit_reference_rejects_ambiguous_prefixes() -> Result<()> {
        let (db_path, _repository_root) = seeded_prompt_db()?;
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;

        let error = runtime
            .block_on(async {
                let location = db_path.to_str().expect("db path should be utf-8");
                let db = turso::Builder::new_local(location).build().await?;
                let conn = db.connect()?;
                conn.execute("PRAGMA foreign_keys = ON", ()).await?;
                resolve_commit_reference(&conn, 1_i64, "abc").await
            })
            .expect_err("ambiguous prefix should fail");
        assert!(error.to_string().contains("ambiguous"));
        Ok(())
    }

    fn sample_report() -> PromptTraceReport {
        PromptTraceReport {
            commit_sha: "abc1234def5678".to_string(),
            harness_type: "claude_code".to_string(),
            model_id: Some("claude-sonnet-4".to_string()),
            git_branch: Some("feature/prompts".to_string()),
            prompts: vec![
                PromptTraceEntry {
                    turn_number: 1,
                    prompt_text: "first prompt".to_string(),
                    prompt_length: 12,
                    is_truncated: false,
                    cwd: Some("src".to_string()),
                    tool_call_count: 3,
                    duration_ms: 120_000,
                    captured_at: "2026-03-18T10:00:00Z".to_string(),
                },
                PromptTraceEntry {
                    turn_number: 2,
                    prompt_text: "second prompt".to_string(),
                    prompt_length: 40,
                    is_truncated: true,
                    cwd: Some("src".to_string()),
                    tool_call_count: 1,
                    duration_ms: 90_000,
                    captured_at: "2026-03-18T10:02:00Z".to_string(),
                },
            ],
        }
    }

    fn seeded_prompt_db() -> Result<(PathBuf, PathBuf)> {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temp_root = std::env::temp_dir().join(format!("sce-trace-prompts-{suffix}"));
        std::fs::create_dir_all(&temp_root)?;
        let repository_root = temp_root.join("repo");
        std::fs::create_dir_all(&repository_root)?;
        let db_path = temp_root.join("local.db");

        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        runtime.block_on(async {
            apply_core_schema_migrations(LocalDatabaseTarget::Path(&db_path)).await?;

            let location = db_path.to_str().expect("db path should be utf-8");
            let db = turso::Builder::new_local(location).build().await?;
            let conn = db.connect()?;
            conn.execute("PRAGMA foreign_keys = ON", ()).await?;

            let canonical_root = repository_root
                .canonicalize()
                .unwrap_or_else(|_| repository_root.clone())
                .to_string_lossy()
                .to_string();

            conn.execute(
                "INSERT INTO repositories (id, canonical_root) VALUES (?1, ?2)",
                (1_i64, canonical_root.as_str()),
            )
            .await?;
            conn.execute(
                "INSERT INTO commits (id, repository_id, commit_sha, idempotency_key) VALUES (?1, ?2, ?3, ?4)",
                (1_i64, 1_i64, "abc1234def5678", "commit:abc1234def5678"),
            )
            .await?;
            conn.execute(
                "INSERT INTO commits (id, repository_id, commit_sha, idempotency_key) VALUES (?1, ?2, ?3, ?4)",
                (2_i64, 1_i64, "abc9999def5678", "commit:abc9999def5678"),
            )
            .await?;
            conn.execute(
                "INSERT INTO prompts (commit_id, prompt_text, prompt_length, is_truncated, turn_number, harness_type, model_id, cwd, git_branch, tool_call_count, duration_ms, captured_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                (1_i64, "first prompt", 12_i64, 0_i64, 1_i64, "claude_code", Some("claude-sonnet-4"), Some("src"), Some("feature/prompts"), 3_i64, 120_000_i64, "2026-03-18T10:00:00Z"),
            )
            .await?;
            conn.execute(
                "INSERT INTO prompts (commit_id, prompt_text, prompt_length, is_truncated, turn_number, harness_type, model_id, cwd, git_branch, tool_call_count, duration_ms, captured_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                (1_i64, "second prompt", 40_i64, 1_i64, 2_i64, "claude_code", Some("claude-sonnet-4"), Some("src"), Some("feature/prompts"), 1_i64, 90_000_i64, "2026-03-18T10:02:00Z"),
            )
            .await?;

            Result::<(), anyhow::Error>::Ok(())
        })?;

        Ok((db_path, repository_root))
    }
}
