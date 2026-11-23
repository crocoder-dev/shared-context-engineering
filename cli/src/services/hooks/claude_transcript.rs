use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

pub const CLAUDE_TRANSCRIPT_TAIL_WINDOW_BYTES: u64 = 4 * 1024 * 1024;
pub const CLAUDE_TRANSCRIPT_MAX_RECORD_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum ClaudeTranscriptReadError {
    Open { path: PathBuf, source: io::Error },
    Seek(io::Error),
    Read(io::Error),
}

impl fmt::Display for ClaudeTranscriptReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { path, source } => write!(
                f,
                "failed to open Claude transcript '{}': {source}",
                path.display()
            ),
            Self::Seek(source) => write!(f, "failed to seek Claude transcript: {source}"),
            Self::Read(source) => write!(f, "failed to read Claude transcript: {source}"),
        }
    }
}

impl std::error::Error for ClaudeTranscriptReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. } | Self::Seek(source) | Self::Read(source) => Some(source),
        }
    }
}

pub fn find_claude_transcript_tool_use_model(
    transcript_path: &Path,
    tool_use_id: &str,
) -> Result<Option<String>, ClaudeTranscriptReadError> {
    let file = File::open(transcript_path).map_err(|source| ClaudeTranscriptReadError::Open {
        path: transcript_path.to_path_buf(),
        source,
    })?;
    find_tool_use_model_in(file, tool_use_id)
}

pub(crate) fn find_tool_use_model_in<R: Read + Seek>(
    mut source: R,
    tool_use_id: &str,
) -> Result<Option<String>, ClaudeTranscriptReadError> {
    let length = source
        .seek(SeekFrom::End(0))
        .map_err(ClaudeTranscriptReadError::Seek)?;
    let window_start = length.saturating_sub(CLAUDE_TRANSCRIPT_TAIL_WINDOW_BYTES);
    let read_start = window_start.saturating_sub(1);
    source
        .seek(SeekFrom::Start(read_start))
        .map_err(ClaudeTranscriptReadError::Seek)?;
    let mut reader = BufReader::new(source.take(length - read_start));

    let mut record = Vec::new();
    if window_start > 0 {
        read_capped_record(&mut reader, &mut record).map_err(ClaudeTranscriptReadError::Read)?;
    }

    while let Some(outcome) =
        read_capped_record(&mut reader, &mut record).map_err(ClaudeTranscriptReadError::Read)?
    {
        if outcome == RecordRead::Oversized {
            continue;
        }
        let Ok(parsed) = serde_json::from_slice::<Value>(&record) else {
            continue;
        };
        if let Some(message) = matching_assistant_message(&parsed, tool_use_id) {
            return Ok(message
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string));
        }
    }

    Ok(None)
}

pub const CLAUDE_SUBAGENT_META_MAX_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClaudeSubagentMeta {
    pub agent_type: Option<String>,
    pub parent_tool_use_id: Option<String>,
}

#[derive(Debug)]
pub enum ClaudeSubagentMetaReadError {
    Open { path: PathBuf, source: io::Error },
    Read(io::Error),
    Oversized,
    Malformed(serde_json::Error),
}

impl fmt::Display for ClaudeSubagentMetaReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { path, source } => write!(
                f,
                "failed to open Claude subagent meta file '{}': {source}",
                path.display()
            ),
            Self::Read(source) => write!(f, "failed to read Claude subagent meta file: {source}"),
            Self::Oversized => write!(
                f,
                "Claude subagent meta file exceeds {CLAUDE_SUBAGENT_META_MAX_BYTES} bytes"
            ),
            Self::Malformed(source) => {
                write!(
                    f,
                    "Claude subagent meta file is not a JSON object: {source}"
                )
            }
        }
    }
}

impl std::error::Error for ClaudeSubagentMetaReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. } | Self::Read(source) => Some(source),
            Self::Malformed(source) => Some(source),
            Self::Oversized => None,
        }
    }
}

pub fn read_claude_subagent_meta(
    meta_path: &Path,
) -> Result<ClaudeSubagentMeta, ClaudeSubagentMetaReadError> {
    let file = File::open(meta_path).map_err(|source| ClaudeSubagentMetaReadError::Open {
        path: meta_path.to_path_buf(),
        source,
    })?;
    read_subagent_meta_from(file)
}

pub(crate) fn read_subagent_meta_from<R: Read>(
    source: R,
) -> Result<ClaudeSubagentMeta, ClaudeSubagentMetaReadError> {
    let mut contents = Vec::new();
    source
        .take(CLAUDE_SUBAGENT_META_MAX_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(ClaudeSubagentMetaReadError::Read)?;
    if contents.len() as u64 > CLAUDE_SUBAGENT_META_MAX_BYTES {
        return Err(ClaudeSubagentMetaReadError::Oversized);
    }

    let meta = serde_json::from_slice::<serde_json::Map<String, Value>>(&contents)
        .map_err(ClaudeSubagentMetaReadError::Malformed)?;
    let string_field = |key: &str| {
        meta.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Ok(ClaudeSubagentMeta {
        agent_type: string_field("agentType"),
        parent_tool_use_id: string_field("toolUseId"),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordRead {
    Complete,
    Oversized,
}

fn read_capped_record<R: BufRead>(
    reader: &mut R,
    record: &mut Vec<u8>,
) -> io::Result<Option<RecordRead>> {
    record.clear();
    let mut oversized = false;
    let mut consumed_any = false;

    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            break;
        }
        consumed_any = true;

        let newline = available.iter().position(|byte| *byte == b'\n');
        let chunk = &available[..newline.unwrap_or(available.len())];
        if !oversized {
            if record.len() + chunk.len() > CLAUDE_TRANSCRIPT_MAX_RECORD_BYTES {
                oversized = true;
                record.clear();
            } else {
                record.extend_from_slice(chunk);
            }
        }

        let consumed = newline.map_or(available.len(), |index| index + 1);
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }

    if !consumed_any {
        return Ok(None);
    }
    Ok(Some(if oversized {
        RecordRead::Oversized
    } else {
        RecordRead::Complete
    }))
}

fn matching_assistant_message<'a>(
    record: &'a Value,
    tool_use_id: &str,
) -> Option<&'a serde_json::Map<String, Value>> {
    let record = record.as_object()?;

    let message = if let Some(message) = record.get("message").and_then(Value::as_object) {
        let is_assistant = record
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "assistant")
            || message
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "assistant");
        if !is_assistant {
            return None;
        }
        message
    } else {
        if record.get("role").and_then(Value::as_str) != Some("assistant") {
            return None;
        }
        record
    };

    let content = message.get("content").and_then(Value::as_array)?;
    let has_matching_tool_use = content.iter().any(|block| {
        block.as_object().is_some_and(|block| {
            block
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "tool_use")
                && block
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == tool_use_id)
        })
    });
    has_matching_tool_use.then_some(message)
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read, Seek, SeekFrom};
    use std::time::{Duration, Instant};

    use super::{
        find_claude_transcript_tool_use_model, find_tool_use_model_in, read_claude_subagent_meta,
        read_subagent_meta_from, ClaudeSubagentMeta, ClaudeSubagentMetaReadError,
        ClaudeTranscriptReadError, CLAUDE_SUBAGENT_META_MAX_BYTES,
        CLAUDE_TRANSCRIPT_MAX_RECORD_BYTES, CLAUDE_TRANSCRIPT_TAIL_WINDOW_BYTES,
    };

    const MIB: usize = 1024 * 1024;

    fn tool_use_record(tool_use_id: &str, model: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "model": model,
                "content": [{"type": "tool_use", "id": tool_use_id, "name": "Write"}]
            }
        })
        .to_string()
    }

    fn tool_use_record_of_length(tool_use_id: &str, model: &str, length: usize) -> String {
        let empty = serde_json::json!({
            "type": "assistant",
            "pad": "",
            "message": {
                "role": "assistant",
                "model": model,
                "content": [{"type": "tool_use", "id": tool_use_id, "name": "Write"}]
            }
        })
        .to_string();
        let record = empty.replacen(
            "\"pad\":\"\"",
            &format!("\"pad\":\"{}\"", "x".repeat(length - empty.len())),
            1,
        );
        assert_eq!(record.len(), length);
        record
    }

    fn filler(length: usize) -> String {
        let mut out = String::new();
        while out.len() < length {
            let remaining = length - out.len();
            let record = if remaining > 1100 {
                format!("{{\"type\":\"user\",\"pad\":\"{}\"}}\n", "y".repeat(1000))
            } else {
                let overhead = "{\"type\":\"user\",\"pad\":\"\"}\n".len();
                if remaining > overhead {
                    format!(
                        "{{\"type\":\"user\",\"pad\":\"{}\"}}\n",
                        "y".repeat(remaining - overhead)
                    )
                } else {
                    format!("{}\n", " ".repeat(remaining - 1))
                }
            };
            out.push_str(&record);
        }
        assert_eq!(out.len(), length);
        out
    }

    fn lookup(bytes: Vec<u8>, tool_use_id: &str) -> Option<String> {
        find_tool_use_model_in(Cursor::new(bytes), tool_use_id).expect("in-memory read succeeds")
    }

    struct CountingReader<R> {
        inner: R,
        bytes_read: u64,
    }

    impl<R: Read> Read for CountingReader<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let read = self.inner.read(buf)?;
            self.bytes_read += read as u64;
            Ok(read)
        }
    }

    impl<R: Seek> Seek for CountingReader<R> {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("disk gone"))
        }
    }

    impl Seek for FailingReader {
        fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
            Ok(64)
        }
    }

    #[test]
    fn claude_transcript_bounds_record_starting_exactly_at_window_start_is_found() {
        let record = tool_use_record("toolu_edge", "claude-opus-5-5");
        let window = usize::try_from(CLAUDE_TRANSCRIPT_TAIL_WINDOW_BYTES).unwrap();
        let tail = filler(window - record.len() - 1);
        let transcript = format!("{}{record}\n{tail}", filler(MIB));

        assert_eq!(transcript.len() - transcript.find(&record).unwrap(), window);
        assert_eq!(
            lookup(transcript.into_bytes(), "toolu_edge").as_deref(),
            Some("claude-opus-5-5")
        );
    }

    #[test]
    fn claude_transcript_bounds_record_starting_one_byte_before_window_is_skipped() {
        let record = tool_use_record("toolu_edge", "claude-opus-5-5");
        let window = usize::try_from(CLAUDE_TRANSCRIPT_TAIL_WINDOW_BYTES).unwrap();
        let tail = filler(window - record.len());
        let transcript = format!("{}{record}\n{tail}", filler(MIB));

        assert_eq!(
            transcript.len() - transcript.find(&record).unwrap(),
            window + 1
        );
        assert_eq!(lookup(transcript.into_bytes(), "toolu_edge"), None);
    }

    #[test]
    fn claude_transcript_bounds_record_at_cap_is_found_and_one_byte_over_is_skipped() {
        let at_cap = tool_use_record_of_length(
            "toolu_cap",
            "claude-opus-5-5",
            CLAUDE_TRANSCRIPT_MAX_RECORD_BYTES,
        );
        let over_cap = tool_use_record_of_length(
            "toolu_over",
            "claude-opus-5-5",
            CLAUDE_TRANSCRIPT_MAX_RECORD_BYTES + 1,
        );
        let transcript = format!("{at_cap}\n{over_cap}\n");

        assert_eq!(
            lookup(transcript.clone().into_bytes(), "toolu_cap").as_deref(),
            Some("claude-opus-5-5")
        );
        assert_eq!(lookup(transcript.into_bytes(), "toolu_over"), None);
    }

    #[test]
    fn claude_transcript_bounds_skips_oversized_and_malformed_records_and_continues() {
        let oversized = tool_use_record_of_length(
            "toolu_target",
            "claude-haiku-5-5",
            CLAUDE_TRANSCRIPT_MAX_RECORD_BYTES + 10,
        );
        let mut transcript = Vec::new();
        transcript.extend_from_slice(oversized.as_bytes());
        transcript.push(b'\n');
        transcript
            .extend_from_slice(b"{\"type\":\"assistant\",\"message\":{\"model\":\"\xff\xfe\"}}\n");
        transcript.extend_from_slice(b"{not json\n");
        transcript.extend_from_slice(tool_use_record("toolu_target", "claude-opus-5-5").as_bytes());
        transcript.push(b'\n');

        assert_eq!(
            lookup(transcript, "toolu_target").as_deref(),
            Some("claude-opus-5-5")
        );
    }

    #[test]
    fn claude_transcript_bounds_missing_id_in_32_mib_transcript_reads_at_most_5_mib() {
        let transcript = filler(32 * MIB).into_bytes();
        let mut reader = CountingReader {
            inner: Cursor::new(transcript),
            bytes_read: 0,
        };

        let started = Instant::now();
        let model = find_tool_use_model_in(&mut reader, "toolu_missing").expect("read succeeds");
        let elapsed = started.elapsed();

        assert_eq!(model, None);
        assert!(
            reader.bytes_read <= 5 * MIB as u64,
            "read {} bytes",
            reader.bytes_read
        );
        assert!(elapsed < Duration::from_millis(500), "took {elapsed:?}");
    }

    #[test]
    fn claude_transcript_bounds_open_failure_is_a_typed_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("missing.jsonl");

        let error = find_claude_transcript_tool_use_model(&missing, "toolu_a")
            .expect_err("missing transcript must fail to open");

        assert!(
            matches!(&error, ClaudeTranscriptReadError::Open { path, .. } if path == &missing),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn claude_transcript_bounds_read_failure_is_a_typed_error() {
        let error = find_tool_use_model_in(FailingReader, "toolu_a")
            .expect_err("failing reader must surface an error");

        assert!(
            matches!(error, ClaudeTranscriptReadError::Read(_)),
            "unexpected error: {error:?}"
        );
    }

    fn meta_of_length(agent_type: &str, tool_use_id: &str, length: usize) -> Vec<u8> {
        let empty =
            serde_json::json!({"agentType": agent_type, "toolUseId": tool_use_id, "pad": ""})
                .to_string();
        let pad = "x".repeat(length - empty.len());
        serde_json::json!({"agentType": agent_type, "toolUseId": tool_use_id, "pad": pad})
            .to_string()
            .into_bytes()
    }

    #[test]
    fn claude_subagent_meta_reads_agent_type_and_parent_tool_use_id() {
        let meta = read_subagent_meta_from(Cursor::new(
            br#"{"agentType":"general-purpose","toolUseId":"toolu_parent","description":"x"}"#,
        ))
        .unwrap();

        assert_eq!(
            meta,
            ClaudeSubagentMeta {
                agent_type: Some(String::from("general-purpose")),
                parent_tool_use_id: Some(String::from("toolu_parent")),
            }
        );
    }

    #[test]
    fn claude_subagent_meta_missing_or_non_string_fields_are_none() {
        let meta =
            read_subagent_meta_from(Cursor::new(br#"{"agentType":7,"toolUseId":"  "}"#)).unwrap();

        assert_eq!(meta, ClaudeSubagentMeta::default());
    }

    #[test]
    fn claude_subagent_meta_at_cap_is_read_and_one_byte_over_is_rejected() {
        let cap = usize::try_from(CLAUDE_SUBAGENT_META_MAX_BYTES).unwrap();

        let at_cap =
            read_subagent_meta_from(Cursor::new(meta_of_length("Explore", "toolu_p", cap)))
                .expect("meta at the cap is read");
        assert_eq!(at_cap.parent_tool_use_id.as_deref(), Some("toolu_p"));

        let over_cap =
            read_subagent_meta_from(Cursor::new(meta_of_length("Explore", "toolu_p", cap + 1)))
                .expect_err("meta over the cap is rejected");
        assert!(matches!(over_cap, ClaudeSubagentMetaReadError::Oversized));
    }

    #[test]
    fn claude_subagent_meta_malformed_and_missing_files_are_typed_errors() {
        let malformed = read_subagent_meta_from(Cursor::new(b"[\"not an object\"]"))
            .expect_err("non-object meta is rejected");
        assert!(matches!(
            malformed,
            ClaudeSubagentMetaReadError::Malformed(_)
        ));

        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("agent-a.meta.json");
        let error = read_claude_subagent_meta(&missing).expect_err("missing meta fails to open");
        assert!(
            matches!(&error, ClaudeSubagentMetaReadError::Open { path, .. } if path == &missing),
            "unexpected error: {error:?}"
        );
    }
}
