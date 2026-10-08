use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

const MAX_LEADING_RECORDS: usize = 16;
const BRIDGE_SESSION_RECORD_TYPE: &str = "bridge-session";

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
