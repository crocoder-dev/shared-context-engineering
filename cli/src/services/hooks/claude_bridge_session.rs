use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

const MAX_LEADING_RECORDS: usize = 16;
const BRIDGE_SESSION_RECORD_TYPE: &str = "bridge-session";

/// Extract Claude's bridge-session identifier from the leading JSONL records.
///
/// Transcript access and parsing are fail-open. Only a bounded number of
/// records are read so discovery never scans a complete transcript.
pub fn extract_claude_bridge_session_id(transcript_path: &Path) -> Option<String> {
    extract_claude_bridge_session_id_from_reader(File::open(transcript_path).map(BufReader::new))
}

pub fn find_claude_bridge_chain_session_ids(
    transcript_path: &Path,
    bridge_session_id: &str,
) -> Vec<String> {
    let bridge_session_id = bridge_session_id.trim();
    if bridge_session_id.is_empty() {
        return Vec::new();
    }

    let directory = transcript_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let source_file_name = transcript_path.file_name();

    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };

    let mut session_ids = Vec::new();
    for entry in entries.flatten() {
        let candidate_path = entry.path();
        if candidate_path.file_name() == source_file_name
            || candidate_path
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("jsonl")
        {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }

        let Some(candidate_session_id) = candidate_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::trim)
            .filter(|session_id| !session_id.is_empty())
            .map(str::to_string)
        else {
            continue;
        };

        if extract_claude_bridge_session_id(&candidate_path).as_deref() != Some(bridge_session_id) {
            continue;
        }

        session_ids.push(candidate_session_id);
    }

    session_ids.sort();
    session_ids.dedup();
    session_ids
}

fn extract_claude_bridge_session_id_from_reader<R: BufRead>(
    reader: io::Result<R>,
) -> Option<String> {
    let reader = reader.ok()?;

    for line in reader.lines().take(MAX_LEADING_RECORDS) {
        let line = line.ok()?;
        let Ok(parsed) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        let Some(record) = parsed.as_object() else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some(BRIDGE_SESSION_RECORD_TYPE) {
            continue;
        }

        if let Some(bridge_session_id) = record
            .get("bridgeSessionId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(bridge_session_id.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Cursor,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sce-claude-bridge-{label}-{suffix}"));
        fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    fn transcript(bridge_session_id: &str, session_id: &str) -> String {
        format!(
            concat!(
                "{{\"type\":\"file-history-snapshot\",\"messageId\":\"msg-1\"}}\n",
                "{{\"type\":\"bridge-session\",\"sessionId\":\"{session_id}\",",
                "\"bridgeSessionId\":\"{bridge_session_id}\"}}\n",
                "{{\"type\":\"user\",\"sessionId\":\"{session_id}\"}}\n"
            ),
            bridge_session_id = bridge_session_id,
            session_id = session_id,
        )
    }

    #[test]
    fn extracts_bridge_session_id_from_real_shaped_leading_records() {
        let content = transcript("cse_bridge-123", "session-new");

        assert_eq!(
            extract_claude_bridge_session_id_from_reader(Ok(Cursor::new(content))),
            Some(String::from("cse_bridge-123"))
        );
    }

    #[test]
    fn bridge_extraction_fails_open_for_missing_unreadable_or_malformed_records() {
        let directory = unique_temp_dir("unreadable");
        let malformed = concat!(
            r#"{"type":"bridge-session","sessionId":"session-1","bridgeSessionId":42}"#,
            "\n"
        );

        assert_eq!(
            extract_claude_bridge_session_id(Path::new("/does/not/exist.jsonl")),
            None
        );
        assert_eq!(
            extract_claude_bridge_session_id_from_reader(Ok(Cursor::new(malformed))),
            None
        );
        assert_eq!(extract_claude_bridge_session_id(&directory), None);

        fs::remove_dir_all(directory).expect("temporary directory should be removed");
    }

    #[test]
    fn bridge_extraction_does_not_scan_beyond_the_leading_record_bound() {
        let mut content = String::new();
        for _ in 0..MAX_LEADING_RECORDS {
            content.push_str("{\"type\":\"user\"}\n");
        }
        content.push_str(&transcript("cse_too-late", "session-late"));

        assert_eq!(
            extract_claude_bridge_session_id_from_reader(Ok(Cursor::new(content))),
            None
        );
    }

    #[test]
    fn returns_every_chain_member_session_id_in_deterministic_order() {
        let directory = unique_temp_dir("chain-multi");
        let source = directory.join("session-current.jsonl");
        let member_b = directory.join("session-b.jsonl");
        let member_a = directory.join("session-a.jsonl");
        let unrelated = directory.join("session-unrelated.jsonl");

        fs::write(&source, transcript("cse_shared", "session-current"))
            .expect("source transcript should be written");
        fs::write(&member_b, transcript("cse_shared", "session-b"))
            .expect("member transcript should be written");
        fs::write(&member_a, transcript("cse_shared", "session-a"))
            .expect("member transcript should be written");
        fs::write(&unrelated, transcript("cse_other", "session-unrelated"))
            .expect("unrelated transcript should be written");

        assert_eq!(
            find_claude_bridge_chain_session_ids(&source, "cse_shared"),
            vec![String::from("session-a"), String::from("session-b")]
        );

        fs::remove_dir_all(directory).expect("temporary directory should be removed");
    }

    #[test]
    fn returns_the_single_chain_member_when_only_one_sibling_matches() {
        let directory = unique_temp_dir("chain-single");
        let source = directory.join("session-current.jsonl");
        let member = directory.join("session-only.jsonl");

        fs::write(&source, transcript("cse_shared", "session-current"))
            .expect("source transcript should be written");
        fs::write(&member, transcript("cse_shared", "session-only"))
            .expect("member transcript should be written");

        assert_eq!(
            find_claude_bridge_chain_session_ids(&source, "cse_shared"),
            vec![String::from("session-only")]
        );

        fs::remove_dir_all(directory).expect("temporary directory should be removed");
    }

    #[test]
    fn chain_discovery_fails_open_to_an_empty_result() {
        let directory = unique_temp_dir("chain-none");
        let source = directory.join("session-current.jsonl");
        let unrelated = directory.join("session-unrelated.jsonl");

        fs::write(&source, transcript("cse_shared", "session-current"))
            .expect("source transcript should be written");
        fs::write(&unrelated, transcript("cse_other", "session-unrelated"))
            .expect("unrelated transcript should be written");

        assert!(find_claude_bridge_chain_session_ids(&source, "cse_shared").is_empty());
        assert!(find_claude_bridge_chain_session_ids(&source, "   ").is_empty());
        assert!(find_claude_bridge_chain_session_ids(
            Path::new("/does/not/exist/session.jsonl"),
            "cse_shared"
        )
        .is_empty());

        fs::remove_dir_all(directory).expect("temporary directory should be removed");
    }

    #[test]
    fn chain_discovery_ignores_a_sibling_whose_bridge_record_is_past_the_leading_bound() {
        let directory = unique_temp_dir("chain-bounded");
        let source = directory.join("session-current.jsonl");
        let late = directory.join("session-late.jsonl");

        fs::write(&source, transcript("cse_shared", "session-current"))
            .expect("source transcript should be written");

        let mut late_content = String::new();
        for _ in 0..MAX_LEADING_RECORDS {
            late_content.push_str("{\"type\":\"user\"}\n");
        }
        late_content.push_str(&transcript("cse_shared", "session-late"));
        fs::write(&late, late_content).expect("late transcript should be written");

        assert!(find_claude_bridge_chain_session_ids(&source, "cse_shared").is_empty());

        fs::remove_dir_all(directory).expect("temporary directory should be removed");
    }
}
