//! Embedded Agent Trace DB SQL shell core.

#![allow(dead_code)]

use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use turso::Value as TursoValue;

use crate::services::agent_trace_db::AgentTraceDb;
use crate::services::db::QueryRows;

const HELP_TEXT: &str = "Commands:\n  .help    Show this help\n  .tables  List tables\n  .exit    Exit the shell\n  .quit    Exit the shell\nSQL statements execute against the resolved Agent Trace DB.\n";
const TABLES_SQL: &str = "SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellTarget {
    pub alias: String,
    pub scope: String,
    pub identifier: String,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellExit {
    EndOfInput,
    DotCommand,
}

pub fn run_agent_trace_db_shell(
    target: &ShellTarget,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<ShellExit> {
    let db = AgentTraceDb::open_for_hooks_without_migrations_at(&target.path)
        .with_context(|| format!("failed to open Agent Trace DB '{}'", target.path.display()))?;
    db.ensure_schema_ready_for_hooks().with_context(|| {
        format!(
            "Agent Trace DB '{}' is not schema-ready",
            target.path.display()
        )
    })?;

    run_agent_trace_db_shell_with_db(&db, target, input, &mut output)
}

pub fn run_agent_trace_db_shell_with_db(
    db: &AgentTraceDb,
    target: &ShellTarget,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<ShellExit> {
    writeln!(output, "Agent Trace DB shell")?;
    writeln!(output, "alias: {}", target.alias)?;
    writeln!(output, "scope: {}", target.scope)?;
    writeln!(output, "identifier: {}", target.identifier)?;
    writeln!(output, "path: {}", target.path.display())?;
    writeln!(output, "Type .help for commands; .exit or .quit to exit.")?;

    for line in input.lines() {
        let line = line.context("failed to read Agent Trace DB shell input")?;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with('.') {
            match trimmed {
                ".help" => {
                    write!(output, "{HELP_TEXT}")?;
                    continue;
                }
                ".tables" => {
                    render_tables(db, &mut output)?;
                    continue;
                }
                ".exit" | ".quit" => return Ok(ShellExit::DotCommand),
                _ => {
                    writeln!(
                        output,
                        "Unknown command: {trimmed}. Run .help for supported commands."
                    )?;
                    continue;
                }
            }
        }

        for statement in split_sql_line(trimmed) {
            if statement.is_empty() {
                continue;
            }
            render_sql_result(db, statement, &mut output)?;
        }
    }

    Ok(ShellExit::EndOfInput)
}

fn split_sql_line(line: &str) -> impl Iterator<Item = &str> {
    line.split(';').map(str::trim)
}

fn render_sql_result(db: &AgentTraceDb, sql: &str, output: &mut impl Write) -> Result<()> {
    match execute_sql(db, sql) {
        Ok(ShellSqlResult::Query(rows)) => render_query_rows(&rows, output),
        Ok(ShellSqlResult::Statement { rows_affected }) => {
            writeln!(output, "OK ({rows_affected} rows affected)").map_err(Into::into)
        }
        Err(error) => writeln!(output, "SQL error: {error}").map_err(Into::into),
    }
}

fn render_tables(db: &AgentTraceDb, output: &mut impl Write) -> Result<()> {
    match db.query_values(TABLES_SQL, ()) {
        Ok(rows) => {
            for row in rows.rows {
                if let Some(TursoValue::Text(name)) = row.first() {
                    writeln!(output, "{name}")?;
                }
            }
            Ok(())
        }
        Err(error) => {
            writeln!(output, "SQL error: failed to list tables: {error}").map_err(Into::into)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum ShellSqlResult {
    Query(QueryRows),
    Statement { rows_affected: u64 },
}

fn execute_sql(db: &AgentTraceDb, sql: &str) -> Result<ShellSqlResult> {
    if is_query_sql(sql) {
        db.query_values(sql, ())
            .map(ShellSqlResult::Query)
            .with_context(|| format!("failed to query SQL: {sql}"))
    } else {
        db.execute(sql, ())
            .map(|rows_affected| ShellSqlResult::Statement { rows_affected })
            .with_context(|| format!("failed to execute SQL: {sql}"))
    }
}

fn is_query_sql(sql: &str) -> bool {
    let first_token = sql
        .trim_start()
        .split(|character: char| character.is_whitespace() || character == '(')
        .next()
        .unwrap_or_default();
    matches!(
        first_token.to_ascii_uppercase().as_str(),
        "SELECT" | "WITH" | "PRAGMA" | "EXPLAIN"
    )
}

fn render_query_rows(rows: &QueryRows, output: &mut impl Write) -> Result<()> {
    if rows.columns.is_empty() {
        writeln!(output, "OK (0 rows affected)")?;
        return Ok(());
    }

    writeln!(output, "{}", rows.columns.join(" | "))?;
    writeln!(
        output,
        "{}",
        rows.columns
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join(" | ")
    )?;

    for row in &rows.rows {
        let rendered = row.iter().map(render_value).collect::<Vec<_>>().join(" | ");
        writeln!(output, "{rendered}")?;
    }

    writeln!(output, "({} rows)", rows.rows.len())?;
    Ok(())
}

fn render_value(value: &TursoValue) -> String {
    match value {
        TursoValue::Null => String::from("NULL"),
        TursoValue::Integer(value) => value.to_string(),
        TursoValue::Real(value) => value.to_string(),
        TursoValue::Text(value) => value.clone(),
        TursoValue::Blob(value) => format!("x'{}'", encode_lower_hex(value)),
    }
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_db(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sce-trace-shell-{label}-{}-{nonce}.db",
            std::process::id()
        ))
    }

    fn shell_target(path: &Path) -> ShellTarget {
        ShellTarget {
            alias: String::from("agent_trace_0"),
            scope: String::from("legacy checkout"),
            identifier: String::from("018f2d7d-0000-7000-8000-000000000000"),
            path: path.to_path_buf(),
        }
    }

    fn run_shell(input: &str) -> String {
        let path = unique_temp_db("core");
        let db = AgentTraceDb::open_at(&path).expect("test DB should open");
        let mut output = Vec::new();
        run_agent_trace_db_shell_with_db(&db, &shell_target(&path), input.as_bytes(), &mut output)
            .expect("shell should run");
        String::from_utf8(output).expect("shell output should be UTF-8")
    }

    fn output_after_startup(output: &str) -> &str {
        output
            .split_once("Type .help for commands; .exit or .quit to exit.\n")
            .expect("shell startup prompt should be present")
            .1
    }

    #[test]
    fn shell_runs_query_and_exit_from_piped_input() {
        let output = run_shell("SELECT COUNT(*) AS diff_trace_count FROM diff_traces;\n.exit\n");

        assert!(output.contains("Agent Trace DB shell\n"));
        assert!(output.contains("alias: agent_trace_0\n"));
        assert!(output.contains("diff_trace_count\n---\n0\n(1 rows)\n"));
    }

    #[test]
    fn shell_renders_help_and_quit() {
        let output = run_shell(".help\n.quit\n");

        assert!(output.contains("Commands:\n  .help    Show this help\n  .tables  List tables\n  .exit    Exit the shell\n  .quit    Exit the shell\n"));
    }

    #[test]
    fn shell_tables_lists_table_names_in_deterministic_order() {
        let output =
            run_shell("CREATE TABLE IF NOT EXISTS z_shell_smoke (id INTEGER);\n.tables\n.exit\n");
        let table_lines = output_after_startup(&output)
            .lines()
            .skip_while(|line| !line.starts_with("OK ("))
            .skip(1)
            .collect::<Vec<_>>();

        assert!(table_lines.contains(&"__sce_migrations"));
        assert!(table_lines.contains(&"diff_traces"));
        assert!(table_lines.contains(&"z_shell_smoke"));

        let migrations_index = table_lines
            .iter()
            .position(|line| *line == "__sce_migrations")
            .expect("migration table should be listed");
        let diff_traces_index = table_lines
            .iter()
            .position(|line| *line == "diff_traces")
            .expect("diff traces table should be listed");
        let smoke_index = table_lines
            .iter()
            .position(|line| *line == "z_shell_smoke")
            .expect("test table should be listed");

        assert!(migrations_index < diff_traces_index);
        assert!(diff_traces_index < smoke_index);
        assert!(!table_lines.iter().any(|line| line.contains(" | ")));
        assert!(!table_lines.iter().any(|line| line.starts_with('(')));
    }

    #[test]
    fn shell_renders_malformed_sql_diagnostic_and_continues() {
        let output = run_shell("SELECT FROM;\nSELECT 1 AS ok;\n.exit\n");

        assert!(output.contains("SQL error: failed to query SQL: SELECT FROM"));
        assert!(output.contains("ok\n---\n1\n(1 rows)\n"));
    }

    #[test]
    fn shell_renders_non_query_statement_success() {
        let output = run_shell("CREATE TABLE IF NOT EXISTS shell_smoke (id INTEGER);\nINSERT INTO shell_smoke (id) VALUES (7);\nSELECT id FROM shell_smoke;\n.exit\n");

        assert!(output.contains("OK (0 rows affected)\n"));
        assert!(output.contains("OK (1 rows affected)\n"));
        assert!(output.contains("id\n---\n7\n(1 rows)\n"));
    }

    #[test]
    fn shell_opens_path_and_checks_schema_readiness() {
        let path = unique_temp_db("open-path");
        let db = AgentTraceDb::open_at(&path).expect("test DB should open");
        drop(db);

        let mut output = Vec::new();
        let exit =
            run_agent_trace_db_shell(&shell_target(&path), ".exit\n".as_bytes(), &mut output)
                .expect("shell should open ready DB");

        assert_eq!(exit, ShellExit::DotCommand);
        assert!(String::from_utf8(output)
            .expect("shell output should be UTF-8")
            .contains("path: "));
    }
}
