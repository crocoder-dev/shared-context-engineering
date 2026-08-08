//! Bounded source extraction and deterministic transformation for code changes.
//!
//! This module deliberately stops at destination-independent values. It copies
//! `diff_traces` rows out of a short source snapshot, then strictly normalizes
//! payloads and derives code-change metrics after that snapshot has ended.

use std::fmt::Write;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::services::{
    agent_trace_db::{normalize_diff_trace_payload, repository::RepositoryAgentTraceDb},
    agent_trace_dwh_db::AgentTraceDwhDb,
    db::TursoTransaction,
    etl::{
        read_watermark, run_with_source_contention_retry, upsert_watermark, validate_batch_size,
        TableBatchStats,
    },
    patch::{ParsedPatch, TouchedLineKind},
};

/// The source table represented by this pipeline.
pub const CODE_CHANGES_SOURCE_TABLE: &str = "diff_traces";

/// Default number of source diff traces processed by one batch.
pub const DEFAULT_CODE_CHANGES_ETL_BATCH_SIZE: u32 = 500;

/// One diff-trace row copied out of a short repository source snapshot.
///
/// Values are owned source copies. No parsing, hashing, or metric derivation
/// occurs while the source read transaction is open.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDiffTrace {
    pub id: i64,
    pub time_ms: i64,
    pub session_id: String,
    pub patch: String,
    pub model_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_version: Option<String>,
    pub payload_type: String,
}

/// Destination-independent metrics derived from a canonical parsed patch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodeChangeMetrics {
    pub files_changed: i64,
    pub lines_added: i64,
    pub lines_removed: i64,
}

/// One source diff trace after strict normalization and deterministic
/// transformation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransformedCodeChange {
    pub source_row_id: i64,
    pub session_id: String,
    pub time_ms: i64,
    pub model_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_version: Option<String>,
    pub payload_type: String,
    pub files_changed: i64,
    pub lines_added: i64,
    pub lines_removed: i64,
    pub patch_sha256: String,
}

/// Counts returned by one atomically loaded code-change batch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CodeChangesBatchStats {
    pub inserted: u64,
    pub already_present: u64,
    pub watermark: i64,
}

const SELECT_DIFF_TRACES_BATCH_SQL: &str =
    "SELECT id, time_ms, session_id, patch, model_id, tool_name, tool_version, payload_type
FROM diff_traces
WHERE id > ?1
ORDER BY id ASC
LIMIT ?2";

/// Extract one bounded, ascending `diff_traces` batch in a short read
/// transaction.
///
/// The transaction commits before this function returns, so callers can parse,
/// hash, and derive metrics without retaining a source snapshot. Only the
/// shared transient lock-contention retry is applied.
pub fn extract_diff_trace_batch(
    db: &RepositoryAgentTraceDb,
    watermark: i64,
    batch_size: u32,
) -> Result<Vec<SourceDiffTrace>> {
    validate_batch_size(batch_size, "diff trace extraction")?;

    run_with_source_contention_retry(
        |_attempt| {
            db.read_transaction(|txn| {
                txn.query_map(
                    SELECT_DIFF_TRACES_BATCH_SQL,
                    (watermark, i64::from(batch_size)),
                    source_diff_trace_from_row,
                )
            })
        },
        || db.rollback_best_effort(),
    )
}

fn source_diff_trace_from_row(row: &turso::Row) -> Result<SourceDiffTrace> {
    Ok(SourceDiffTrace {
        id: row.get(0).context("failed to read diff_traces.id")?,
        time_ms: row.get(1).context("failed to read diff_traces.time_ms")?,
        session_id: row
            .get(2)
            .context("failed to read diff_traces.session_id")?,
        patch: row.get(3).context("failed to read diff_traces.patch")?,
        model_id: row.get(4).context("failed to read diff_traces.model_id")?,
        tool_name: row.get(5).context("failed to read diff_traces.tool_name")?,
        tool_version: row
            .get(6)
            .context("failed to read diff_traces.tool_version")?,
        payload_type: row
            .get(7)
            .context("failed to read diff_traces.payload_type")?,
    })
}

/// Derive checked destination-sized metrics from a canonical parsed patch.
pub fn derive_code_change_metrics(parsed: &ParsedPatch) -> Result<CodeChangeMetrics> {
    let files_changed = i64::try_from(parsed.files.len())
        .context("code-change files_changed does not fit in destination integer")?;
    let mut lines_added = 0_i64;
    let mut lines_removed = 0_i64;

    for file in &parsed.files {
        for hunk in &file.hunks {
            for line in &hunk.lines {
                match line.kind {
                    TouchedLineKind::Added => {
                        lines_added = lines_added
                            .checked_add(1)
                            .context("code-change lines_added overflowed destination integer")?;
                    }
                    TouchedLineKind::Removed => {
                        lines_removed = lines_removed
                            .checked_add(1)
                            .context("code-change lines_removed overflowed destination integer")?;
                    }
                }
            }
        }
    }

    Ok(CodeChangeMetrics {
        files_changed,
        lines_added,
        lines_removed,
    })
}

/// Transform an extracted source row after the source snapshot has ended.
///
/// Payload normalization is strict for both supported payload types. The hash
/// is computed from the exact original UTF-8 payload bytes, not normalized
/// patch output.
pub fn transform_code_change(source: &SourceDiffTrace) -> Result<TransformedCodeChange> {
    let parsed = normalize_diff_trace_payload(
        source.payload_type.as_str(),
        &source.patch,
        source.session_id.as_str(),
        source.time_ms,
        source.tool_version.as_deref(),
    )?;
    let metrics = derive_code_change_metrics(&parsed)?;

    Ok(TransformedCodeChange {
        source_row_id: source.id,
        session_id: source.session_id.clone(),
        time_ms: source.time_ms,
        model_id: source.model_id.clone(),
        tool_name: source.tool_name.clone(),
        tool_version: source.tool_version.clone(),
        payload_type: source.payload_type.clone(),
        files_changed: metrics.files_changed,
        lines_added: metrics.lines_added,
        lines_removed: metrics.lines_removed,
        patch_sha256: sha256_hex(source.patch.as_bytes()),
    })
}

/// Load one transformed source batch atomically into the DWH `code_changes`
/// table. Transformation happens before the destination transaction starts.
pub fn load_code_change_batch(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
    source_rows: &[SourceDiffTrace],
) -> Result<CodeChangesBatchStats> {
    let transformed = source_rows
        .iter()
        .map(transform_code_change)
        .collect::<Result<Vec<_>>>()?;
    load_transformed_code_change_batch(db, repository_id, source_instance_id, &transformed)
}

fn load_transformed_code_change_batch(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
    rows: &[TransformedCodeChange],
) -> Result<CodeChangesBatchStats> {
    load_transformed_code_change_batch_with_failure(
        db,
        repository_id,
        source_instance_id,
        rows,
        None,
    )
}

fn load_transformed_code_change_batch_with_failure(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
    rows: &[TransformedCodeChange],
    fail_after_row: Option<usize>,
) -> Result<CodeChangesBatchStats> {
    let Some(last_row) = rows.last() else {
        return Ok(CodeChangesBatchStats::default());
    };
    if rows.iter().any(|row| row.tool_name.is_none()) {
        bail!("diff_traces.tool_name cannot be null for code-change loading");
    }

    db.transaction(|txn| {
        ensure_lineage(txn, repository_id, source_instance_id)?;

        let mut stats = TableBatchStats {
            watermark: last_row.source_row_id,
            ..Default::default()
        };

        for (index, row) in rows.iter().enumerate() {
            let existing = txn.query_map(
                "SELECT session_id, time_ms, model_id, tool_name, tool_version, payload_type,
                        files_changed, lines_added, lines_removed, patch_sha256
                 FROM code_changes
                 WHERE repository_id = ?1 AND source_instance_id = ?2
                   AND source_diff_trace_id = ?3",
                (repository_id, source_instance_id, row.source_row_id),
                |db_row| {
                    Ok((
                        db_row.get::<String>(0)?,
                        db_row.get::<i64>(1)?,
                        db_row.get::<Option<String>>(2)?,
                        db_row.get::<String>(3)?,
                        db_row.get::<Option<String>>(4)?,
                        db_row.get::<String>(5)?,
                        db_row.get::<i64>(6)?,
                        db_row.get::<i64>(7)?,
                        db_row.get::<i64>(8)?,
                        db_row.get::<String>(9)?,
                    ))
                },
            )?;

            if let Some(existing) = existing.into_iter().next() {
                let incoming = (
                    row.session_id.clone(),
                    row.time_ms,
                    row.model_id.clone(),
                    row.tool_name.clone().expect("tool_name validated before transaction"),
                    row.tool_version.clone(),
                    row.payload_type.clone(),
                    row.files_changed,
                    row.lines_added,
                    row.lines_removed,
                    row.patch_sha256.clone(),
                );
                if existing != incoming {
                    bail!(
                        "code change integrity conflict for repository {repository_id}, source instance {source_instance_id}, source diff trace {}",
                        row.source_row_id
                    );
                }
                stats.already_present += 1;
            } else {
                txn.execute(
                    "INSERT INTO code_changes
                     (repository_id, source_instance_id, source_diff_trace_id, session_id,
                      time_ms, model_id, tool_name, tool_version, payload_type, files_changed,
                      lines_added, lines_removed, patch_sha256)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    (
                        repository_id,
                        source_instance_id,
                        row.source_row_id,
                        row.session_id.as_str(),
                        row.time_ms,
                        row.model_id.as_deref(),
                        row.tool_name
                            .as_deref()
                            .expect("tool_name validated before transaction"),
                        row.tool_version.as_deref(),
                        row.payload_type.as_str(),
                        row.files_changed,
                        row.lines_added,
                        row.lines_removed,
                        row.patch_sha256.as_str(),
                    ),
                )?;
                stats.inserted += 1;
            }

            if fail_after_row == Some(index + 1) {
                bail!(
                    "injected code changes destination failure after row {}",
                    index + 1
                );
            }
        }

        upsert_watermark(
            txn,
            repository_id,
            source_instance_id,
            CODE_CHANGES_SOURCE_TABLE,
            last_row.source_row_id,
        )?;

        Ok(CodeChangesBatchStats {
            inserted: stats.inserted,
            already_present: stats.already_present,
            watermark: stats.watermark,
        })
    })
}

fn ensure_lineage(
    txn: &TursoTransaction<'_, crate::services::agent_trace_dwh_db::AgentTraceDwhDbSpec>,
    repository_id: &str,
    source_instance_id: &str,
) -> Result<()> {
    txn.execute(
        "INSERT INTO repositories (repository_id) VALUES (?1)
         ON CONFLICT (repository_id) DO NOTHING",
        (repository_id,),
    )?;
    txn.execute(
        "INSERT INTO source_instances (repository_id, source_instance_id) VALUES (?1, ?2)
         ON CONFLICT (repository_id, source_instance_id) DO NOTHING",
        (repository_id, source_instance_id),
    )?;
    Ok(())
}

/// Read the code-change watermark, treating an absent row as zero.
pub fn read_code_changes_watermark(
    db: &AgentTraceDwhDb,
    repository_id: &str,
    source_instance_id: &str,
) -> Result<i64> {
    read_watermark(
        db,
        repository_id,
        source_instance_id,
        CODE_CHANGES_SOURCE_TABLE,
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().fold(
        String::with_capacity(digest.len() * 2),
        |mut output, byte| {
            write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
            output
        },
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::services::agent_trace_db::{DiffTraceInsert, PAYLOAD_TYPE_PATCH};

    fn unique_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "sce-code-changes-etl-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("agent-trace.db")
    }

    fn clean(path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    fn valid_patch(path: &str, added: &str, removed: Option<&str>) -> String {
        let removed_line = removed.map(|line| format!("-{line}\n")).unwrap_or_default();
        format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,1 +1,1 @@\n{removed_line}+{added}\n"
        )
    }

    fn source_row(id: i64, patch: &str) -> SourceDiffTrace {
        SourceDiffTrace {
            id,
            time_ms: 1_000 + id,
            session_id: format!("session-{id}"),
            patch: patch.to_string(),
            model_id: Some(format!("provider/model-{id}")),
            tool_name: Some("opencode".to_string()),
            tool_version: Some("1.2.3".to_string()),
            payload_type: PAYLOAD_TYPE_PATCH.to_string(),
        }
    }

    #[test]
    fn code_changes_etl_extraction_uses_zero_watermark_and_ascending_bounded_ids() {
        let path = unique_path("extraction");
        let db = RepositoryAgentTraceDb::new_at(&path).expect("source DB should open");
        for id in 1..=3 {
            db.insert_diff_trace(DiffTraceInsert {
                time_ms: id,
                session_id: "session",
                patch: &valid_patch(&format!("file-{id}.rs"), "added", None),
                model_id: None,
                tool_name: "opencode",
                tool_version: None,
                payload_type: PAYLOAD_TYPE_PATCH,
            })
            .expect("diff trace insert should succeed");
        }

        let rows = extract_diff_trace_batch(&db, 0, 2).expect("bounded extraction should succeed");
        assert_eq!(
            rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(rows[0].time_ms, 1);
        assert_eq!(rows[0].session_id, "session");
        assert_eq!(rows[0].payload_type, PAYLOAD_TYPE_PATCH);
        assert!(extract_diff_trace_batch(&db, 3, 2)
            .expect("empty extraction should succeed")
            .is_empty());

        clean(&path);
    }

    #[test]
    fn code_changes_etl_extraction_rejects_zero_batch_size() {
        let path = unique_path("zero-batch");
        let db = RepositoryAgentTraceDb::new_at(&path).expect("source DB should open");
        let error = extract_diff_trace_batch(&db, 0, 0).expect_err("zero batch must fail");
        assert!(error.to_string().contains("batch_size"));
        clean(&path);
    }

    #[test]
    fn code_changes_metrics_count_files_and_all_touched_line_kinds() {
        let first = source_row(1, &valid_patch("one.rs", "new", Some("old")));
        let second = source_row(2, &valid_patch("two.rs", "another", None));
        let first_transformed = transform_code_change(&first).expect("first patch should parse");
        let second_transformed = transform_code_change(&second).expect("second patch should parse");

        assert_eq!(
            (
                first_transformed.files_changed,
                first_transformed.lines_added,
                first_transformed.lines_removed,
            ),
            (1, 1, 1)
        );
        assert_eq!(
            (
                second_transformed.files_changed,
                second_transformed.lines_added,
                second_transformed.lines_removed,
            ),
            (1, 1, 0)
        );
    }

    #[test]
    fn code_changes_metrics_are_checked_destination_integers() {
        let metrics = derive_code_change_metrics(&ParsedPatch { files: vec![] })
            .expect("empty metrics should fit destination integers");
        assert_eq!(
            metrics,
            CodeChangeMetrics {
                files_changed: 0,
                lines_added: 0,
                lines_removed: 0
            }
        );
    }

    #[test]
    fn code_changes_hash_uses_exact_original_payload_bytes_and_lowercase_hex() {
        let source = source_row(7, &valid_patch("hash.rs", "value", None));
        let transformed = transform_code_change(&source).expect("patch should parse");
        assert_eq!(
            transformed.patch_sha256,
            "16e02392f2925b987722f8981718dd6ea1ab26841ffc42ee55116e5fb33fbba3"
        );
        assert_eq!(
            transformed.patch_sha256,
            transformed.patch_sha256.to_lowercase()
        );
    }

    #[test]
    fn code_changes_transformation_preserves_source_metadata_and_rejects_future_payloads() {
        let source = source_row(3, &valid_patch("metadata.rs", "value", None));
        let transformed = transform_code_change(&source).expect("patch should parse");
        assert_eq!(transformed.source_row_id, source.id);
        assert_eq!(transformed.session_id, source.session_id);
        assert_eq!(transformed.time_ms, source.time_ms);
        assert_eq!(transformed.model_id, source.model_id);
        assert_eq!(transformed.tool_name, source.tool_name);
        assert_eq!(transformed.tool_version, source.tool_version);
        assert_eq!(transformed.payload_type, PAYLOAD_TYPE_PATCH);

        let mut future = source;
        future.payload_type = "future".to_string();
        let error = transform_code_change(&future).expect_err("future payload must fail strictly");
        assert!(error
            .to_string()
            .contains("unsupported diff-trace payload_type"));
    }

    #[test]
    fn code_changes_transformation_normalizes_structured_payloads_and_preserves_metadata() {
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "src/services/structured_patch/fixtures/edit_single_hunk/claude-post-tool-use.json",
        );
        let source = SourceDiffTrace {
            id: 9,
            time_ms: 2_000,
            session_id: "cc-session".to_string(),
            patch: fs::read_to_string(fixture_path).expect("structured fixture should be readable"),
            model_id: Some("claude/model".to_string()),
            tool_name: Some("claude".to_string()),
            tool_version: Some("4.0".to_string()),
            payload_type: "structured".to_string(),
        };

        let transformed =
            transform_code_change(&source).expect("structured payload should normalize");
        assert_eq!(transformed.payload_type, "structured");
        assert_eq!(transformed.session_id, "cc-session");
        assert_eq!(transformed.model_id, Some("claude/model".to_string()));
        assert_eq!(transformed.files_changed, 1);
        assert!(transformed.lines_added > 0);
    }

    #[test]
    fn code_changes_identity_inserts_full_content_and_matching_replay_is_already_present() {
        let dwh_path = unique_path("identity");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).expect("DWH DB should open");
        let row = source_row(7, &valid_patch("identity.rs", "new", Some("old")));

        let first =
            load_code_change_batch(&dwh, "repo-a", "instance-a", std::slice::from_ref(&row))
                .expect("first code-change load should insert");
        assert_eq!(first.inserted, 1);
        assert_eq!(first.already_present, 0);
        assert_eq!(first.watermark, 7);
        assert_eq!(
            read_code_changes_watermark(&dwh, "repo-a", "instance-a").unwrap(),
            7
        );

        let values = dwh
            .query_map(
                "SELECT source_instance_id, source_diff_trace_id, session_id, time_ms, model_id,
                        tool_name, tool_version, payload_type, files_changed, lines_added,
                        lines_removed, patch_sha256
                 FROM code_changes",
                (),
                |db_row| {
                    Ok((
                        db_row.get::<String>(0)?,
                        db_row.get::<i64>(1)?,
                        db_row.get::<String>(2)?,
                        db_row.get::<i64>(3)?,
                        db_row.get::<Option<String>>(4)?,
                        db_row.get::<String>(5)?,
                        db_row.get::<Option<String>>(6)?,
                        db_row.get::<String>(7)?,
                        db_row.get::<i64>(8)?,
                        db_row.get::<i64>(9)?,
                        db_row.get::<i64>(10)?,
                        db_row.get::<String>(11)?,
                    ))
                },
            )
            .expect("code-change content query should succeed");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0, "instance-a");
        assert_eq!(values[0].1, 7);
        assert_eq!(values[0].2, "session-7");
        assert_eq!(values[0].4, Some("provider/model-7".to_string()));
        assert_eq!(values[0].5, "opencode");
        assert_eq!(values[0].7, PAYLOAD_TYPE_PATCH);
        assert_eq!((values[0].8, values[0].9, values[0].10), (1, 1, 1));

        let replay = load_code_change_batch(&dwh, "repo-a", "instance-a", &[row])
            .expect("matching replay should succeed");
        assert_eq!(replay.inserted, 0);
        assert_eq!(replay.already_present, 1);
        assert_eq!(
            dwh.query_map("SELECT COUNT(*) FROM code_changes", (), |db_row| db_row
                .get::<i64>(0)
                .map_err(Into::into))
                .unwrap(),
            vec![1]
        );

        clean(&dwh_path);
    }

    #[test]
    fn code_changes_identity_conflict_fails_without_overwrite_or_watermark_change() {
        let dwh_path = unique_path("identity-conflict");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).expect("DWH DB should open");
        let original = source_row(1, &valid_patch("conflict.rs", "original", None));
        load_code_change_batch(
            &dwh,
            "repo-a",
            "instance-a",
            std::slice::from_ref(&original),
        )
        .expect("original code change should insert");

        let changed = source_row(1, &valid_patch("conflict.rs", "changed", None));
        let error = load_code_change_batch(&dwh, "repo-a", "instance-a", &[changed])
            .expect_err("changed synchronized content must fail");
        assert!(error.to_string().contains("code change integrity conflict"));
        assert_eq!(
            read_code_changes_watermark(&dwh, "repo-a", "instance-a").unwrap(),
            1
        );
        assert_eq!(
            dwh.query_map(
                "SELECT lines_added, patch_sha256 FROM code_changes",
                (),
                |db_row| { Ok((db_row.get::<i64>(0)?, db_row.get::<String>(1)?)) }
            )
            .unwrap()
            .len(),
            1
        );

        clean(&dwh_path);
    }

    #[test]
    fn code_changes_atomic_destination_failure_rolls_back_dimensions_facts_and_watermark() {
        let dwh_path = unique_path("atomic");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).expect("DWH DB should open");
        let rows = vec![
            transform_code_change(&source_row(1, &valid_patch("one.rs", "one", None))).unwrap(),
            transform_code_change(&source_row(2, &valid_patch("two.rs", "two", None))).unwrap(),
        ];

        load_transformed_code_change_batch_with_failure(
            &dwh,
            "repo-a",
            "instance-a",
            &rows,
            Some(1),
        )
        .expect_err("injected destination failure should roll back the batch");

        for table in [
            "repositories",
            "source_instances",
            "code_changes",
            "etl_watermarks",
        ] {
            assert_eq!(
                dwh.query_map(&format!("SELECT COUNT(*) FROM {table}"), (), |db_row| {
                    db_row.get::<i64>(0).map_err(Into::into)
                })
                .unwrap(),
                vec![0],
                "{table} should be rolled back"
            );
        }

        let replay = load_code_change_batch(
            &dwh,
            "repo-a",
            "instance-a",
            &[
                source_row(1, &valid_patch("one.rs", "one", None)),
                source_row(2, &valid_patch("two.rs", "two", None)),
            ],
        )
        .expect("the failed batch should be replayable");
        assert_eq!(replay.inserted, 2);
        assert_eq!(replay.watermark, 2);

        clean(&dwh_path);
    }

    #[test]
    fn code_changes_same_local_identity_coexists_across_source_instances() {
        let dwh_path = unique_path("lineage");
        let dwh = AgentTraceDwhDb::new_at(&dwh_path).expect("DWH DB should open");
        let row = source_row(1, &valid_patch("lineage.rs", "same local id", None));

        load_code_change_batch(&dwh, "repo-a", "instance-a", std::slice::from_ref(&row))
            .expect("first source instance should load");
        load_code_change_batch(&dwh, "repo-a", "instance-b", &[row])
            .expect("second source instance should load independently");

        assert_eq!(
            dwh.query_map(
                "SELECT COUNT(*) FROM code_changes WHERE repository_id = 'repo-a'",
                (),
                |db_row| db_row.get::<i64>(0).map_err(Into::into)
            )
            .unwrap(),
            vec![2]
        );
        assert_eq!(
            read_code_changes_watermark(&dwh, "repo-a", "instance-a").unwrap(),
            1
        );
        assert_eq!(
            read_code_changes_watermark(&dwh, "repo-a", "instance-b").unwrap(),
            1
        );

        clean(&dwh_path);
    }
}
