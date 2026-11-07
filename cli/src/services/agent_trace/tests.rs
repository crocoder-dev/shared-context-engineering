use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use rand::{rngs::StdRng, Rng, SeedableRng};
use serde_json::Value;

use super::committed_content::CommittedFileRead;
use super::{
    build_agent_trace_from_evidence, line_change_attribution, resolve_line, resolve_patch_lines,
    validate_agent_trace_value, AcceptedDirectLine, AgentTraceEvidence, AgentTraceMetadataInput,
    AgentTraceVcsType, CommittedFile, CommittedFileReader, ContributorType, DirectEvidenceSource,
    DirectLineEvidence, LineAttribution, LineChangeAttribution, LineChangeCounts,
    MutationEvidenceIndex, MutationLineEvidence, RepositoryRoot, ResolvedLine,
    UnverifiedCommittedFiles, AGENT_TRACE_VERSION,
};
use crate::services::{
    agent_trace::{agent_trace_conversation_url, agent_trace_session_url},
    agent_trace_db::DiffTracePathOrigin,
    mutation_trace::attribution::{PatchLineLocation, PatchLineProvenance},
    patch::{parse_patch, ParsedPatch, TouchedLineKind},
};

const TEST_COMMIT_TIMESTAMP: &str = "2026-04-23T10:20:30Z";
const TEST_COMMIT_REVISION: &str = "a0b1c2d3e4f5a6b7c8d9e0f11223344556677889";
const TEST_REPOSITORY_ROOT: &str = "/workspace/project";

#[derive(Clone, Copy)]
struct AgentTraceScenario {
    incremental: &'static [&'static str],
    post_commit: &'static str,
    golden: &'static str,
}

#[derive(Clone, Debug, Default)]
struct PatchNewSideFiles(BTreeMap<String, Vec<u8>>);

impl PatchNewSideFiles {
    fn from_patch(text: &str) -> Self {
        let mut files: BTreeMap<String, BTreeMap<u64, Vec<u8>>> = BTreeMap::new();
        let mut path: Option<String> = None;
        let (mut new_line, mut old_left, mut new_left) = (0_u64, 0_u64, 0_u64);
        let mut last_new_line: Option<u64> = None;
        for raw in text.split_inclusive('\n') {
            let terminated;
            let raw = if raw.ends_with('\n') {
                raw
            } else {
                terminated = format!("{raw}\n");
                terminated.as_str()
            };
            if old_left > 0 || new_left > 0 {
                let (marker, body) = if raw == "\n" {
                    (' ', "\n")
                } else {
                    (raw.chars().next().expect("hunk body line"), &raw[1..])
                };
                let file = files.entry(path.clone().unwrap_or_default()).or_default();
                match marker {
                    '+' => {
                        file.insert(new_line, body.as_bytes().to_vec());
                        last_new_line = Some(new_line);
                        new_line += 1;
                        new_left -= 1;
                    }
                    '-' => {
                        last_new_line = None;
                        old_left -= 1;
                    }
                    ' ' => {
                        file.insert(new_line, body.as_bytes().to_vec());
                        last_new_line = Some(new_line);
                        new_line += 1;
                        old_left -= 1;
                        new_left -= 1;
                    }
                    '\\' => {
                        Self::strip_final_newline(&mut files, path.as_deref(), last_new_line);
                    }
                    other => panic!("unexpected hunk body marker {other:?}"),
                }
                continue;
            }
            if raw.starts_with("\\ ") {
                Self::strip_final_newline(&mut files, path.as_deref(), last_new_line);
                continue;
            }
            last_new_line = None;
            let line = raw.trim_end_matches('\n');
            if let Some(rest) = line.strip_prefix("diff --git ") {
                path = rest.split_once(" b/").map(|(_, new)| new.to_owned());
            } else if let Some(rest) = line.strip_prefix("+++ ") {
                path = rest.strip_prefix("b/").map(ToOwned::to_owned);
            } else if let Some(rest) = line.strip_prefix("@@ -") {
                let (old, rest) = rest.split_once(" +").expect("hunk new range");
                let new = rest.split_once(" @@").expect("hunk header end").0;
                let count = |range: &str| {
                    range
                        .split_once(',')
                        .map_or(Ok(1), |(_, count)| count.parse())
                        .expect("hunk count")
                };
                old_left = count(old);
                new_left = count(new);
                new_line = new
                    .split(',')
                    .next()
                    .and_then(|start| start.parse().ok())
                    .expect("hunk new start");
            }
        }

        Self(
            files
                .into_iter()
                .filter(|(path, _)| !path.is_empty())
                .map(|(path, lines)| {
                    let last = lines.keys().copied().max().unwrap_or(0);
                    let bytes = (1..=last)
                        .flat_map(|line_number| {
                            lines.get(&line_number).cloned().unwrap_or_else(|| {
                                format!("<line {line_number} outside the patch>\n").into_bytes()
                            })
                        })
                        .collect();
                    (path, bytes)
                })
                .collect(),
        )
    }

    fn strip_final_newline(
        files: &mut BTreeMap<String, BTreeMap<u64, Vec<u8>>>,
        path: Option<&str>,
        line_number: Option<u64>,
    ) {
        if let (Some(path), Some(line_number)) = (path, line_number) {
            if let Some(line) = files
                .get_mut(path)
                .and_then(|lines| lines.get_mut(&line_number))
            {
                assert_eq!(
                    line.pop(),
                    Some(b'\n'),
                    "missing newline marker after {line_number}"
                );
            }
        }
    }

    fn bytes(&self, path: &str) -> &[u8] {
        self.0.get(path).expect("committed file")
    }
}

impl CommittedFileReader for PatchNewSideFiles {
    fn read_committed_file(&self, path: &str) -> CommittedFileRead {
        Ok(self.0.get(path).map_or(CommittedFile::Missing, |bytes| {
            CommittedFile::Blob(bytes.clone())
        }))
    }
}

fn parse_fixtures(fixtures: &[&str]) -> Vec<ParsedPatch> {
    fixtures
        .iter()
        .map(|fixture| parse_patch(fixture, None).expect("fixture patch should parse"))
        .collect()
}

const TEXT_FILE_LIFECYCLE_RECONSTRUCTION_INCREMENTALS: &[&str] = &[
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_01.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_02.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_03.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_04.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_05.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_06.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_07.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_08.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_09.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_10.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_11.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_12.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_13.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_14.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_15.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_16.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_17.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_18.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_19.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_20.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_21.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_22.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_23.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_24.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_25.patch"),
    include_str!("fixtures/text_file_lifecycle_reconstruction/incremental_26.patch"),
];

fn assert_builds_expected_agent_trace(scenario: AgentTraceScenario) {
    let direct_patches = parse_fixtures(scenario.incremental);
    let post_commit_patch =
        parse_patch(scenario.post_commit, None).expect("fixture patch should parse");
    let golden: Value = serde_json::from_str(scenario.golden).expect("golden json should load");
    validate_agent_trace_value(&golden).expect("golden json should validate against schema");
    let empty_mutation_patch = ParsedPatch { files: Vec::new() };
    let direct_sources: Vec<_> = direct_patches
        .iter()
        .map(DirectEvidenceSource::repository_relative)
        .collect();
    let actual = build_agent_trace_from_evidence(
        AgentTraceEvidence {
            repository_root: &RepositoryRoot::new(Path::new(TEST_REPOSITORY_ROOT)),
            direct_sources: &direct_sources,
            mutation_ai_patch: &empty_mutation_patch,
            mutation_non_ai_patch: &empty_mutation_patch,
        },
        &post_commit_patch,
        AgentTraceMetadataInput {
            commit_timestamp: TEST_COMMIT_TIMESTAMP,
            commit_revision: TEST_COMMIT_REVISION,
            vcs_type: Some(AgentTraceVcsType::Git),
        },
        &PatchNewSideFiles::from_patch(scenario.post_commit),
    )
    .expect("agent trace should build");
    assert_eq!(actual.version, AGENT_TRACE_VERSION);
    assert_eq!(actual.timestamp, TEST_COMMIT_TIMESTAMP);
    assert_eq!(
        actual.vcs,
        Some(super::AgentTraceVcs {
            r#type: AgentTraceVcsType::Git,
            revision: TEST_COMMIT_REVISION.to_string(),
        })
    );
    let actual_json = serde_json::to_value(&actual).expect("agent trace should serialize");
    validate_agent_trace_value(&actual_json).expect("actual json should validate against schema");
    let expected_conversation_url = agent_trace_conversation_url(&actual.id);
    let mut expected_files = golden["files"].clone();
    for conversation in expected_files
        .as_array_mut()
        .expect("golden files should be an array")
        .iter_mut()
        .flat_map(|file| {
            file["conversations"]
                .as_array_mut()
                .expect("golden conversations should be an array")
                .iter_mut()
        })
    {
        conversation["url"] = Value::String(expected_conversation_url.clone());
    }
    let metadata_version = actual_json["metadata"]["sce"]["version"]
        .as_str()
        .expect("metadata.sce.version should serialize as a string");
    assert!(
        !metadata_version.is_empty(),
        "metadata.sce.version should not be empty"
    );
    assert_eq!(
        actual_json["metadata"]["sce"]["line_changes"], golden["metadata"]["sce"]["line_changes"],
        "line_changes should match golden fixture exactly"
    );
    assert_eq!(actual_json["vcs"], golden["vcs"]);
    assert_eq!(actual_json["files"], expected_files);
}

#[test]
fn average_age_reconstruction_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace(AgentTraceScenario {
        incremental: &[
            include_str!("fixtures/average_age_reconstruction/incremental_01.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_02.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_03.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_04.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_05.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_06.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_07.patch"),
        ],
        post_commit: include_str!("fixtures/average_age_reconstruction/post_commit.patch"),
        golden: include_str!("fixtures/average_age_reconstruction/golden.json"),
    });
}

#[test]
fn hello_world_reconstruction_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace(AgentTraceScenario {
        incremental: &[include_str!(
            "fixtures/hello_world_reconstruction/incremental_01.patch"
        )],
        post_commit: include_str!("fixtures/hello_world_reconstruction/post_commit.patch"),
        golden: include_str!("fixtures/hello_world_reconstruction/golden.json"),
    });
}

#[test]
fn mixed_change_reconstruction_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace(AgentTraceScenario {
        incremental: &[
            include_str!("fixtures/mixed_change_reconstruction/incremental_01.patch"),
            include_str!("fixtures/mixed_change_reconstruction/incremental_02.patch"),
            include_str!("fixtures/mixed_change_reconstruction/incremental_03.patch"),
            include_str!("fixtures/mixed_change_reconstruction/incremental_04.patch"),
        ],
        post_commit: include_str!("fixtures/mixed_change_reconstruction/post_commit.patch"),
        golden: include_str!("fixtures/mixed_change_reconstruction/golden.json"),
    });
}

#[test]
fn poem_edit_reconstruction_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace(AgentTraceScenario {
        incremental: &[
            include_str!("fixtures/poem_edit_reconstruction/incremental_01.patch"),
            include_str!("fixtures/poem_edit_reconstruction/incremental_02.patch"),
        ],
        post_commit: include_str!("fixtures/poem_edit_reconstruction/post_commit.patch"),
        golden: include_str!("fixtures/poem_edit_reconstruction/golden.json"),
    });
}

#[test]
fn poem_write_reconstruction_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace(AgentTraceScenario {
        incremental: &[include_str!(
            "fixtures/poem_write_reconstruction/incremental_01.patch"
        )],
        post_commit: include_str!("fixtures/poem_write_reconstruction/post_commit.patch"),
        golden: include_str!("fixtures/poem_write_reconstruction/golden.json"),
    });
}

#[test]
fn text_file_lifecycle_reconstruction_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace(AgentTraceScenario {
        incremental: TEXT_FILE_LIFECYCLE_RECONSTRUCTION_INCREMENTALS,
        post_commit: include_str!("fixtures/text_file_lifecycle_reconstruction/post_commit.patch"),
        golden: include_str!("fixtures/text_file_lifecycle_reconstruction/golden.json"),
    });
}

#[test]
fn file_rename_reconstruction_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace(AgentTraceScenario {
        incremental: &[include_str!(
            "fixtures/file_rename_reconstruction/incremental_01.patch"
        )],
        post_commit: include_str!("fixtures/file_rename_reconstruction/post_commit.patch"),
        golden: include_str!("fixtures/file_rename_reconstruction/golden.json"),
    });
}

#[test]
fn line_and_file_deletion_reconstruction_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace(AgentTraceScenario {
        incremental: &[
            include_str!("fixtures/line_and_file_deletion_reconstruction/incremental_01.patch"),
            include_str!("fixtures/line_and_file_deletion_reconstruction/incremental_02.patch"),
        ],
        post_commit: include_str!(
            "fixtures/line_and_file_deletion_reconstruction/post_commit.patch"
        ),
        golden: include_str!("fixtures/line_and_file_deletion_reconstruction/golden.json"),
    });
}

#[derive(Clone, Copy)]
struct EvidenceScenario {
    direct: &'static str,
    mutation_ai: &'static str,
    post_commit: &'static str,
    golden: &'static str,
}

const EVIDENCE_DIRECT_SESSION_ID: &str = "sess-direct";
const EVIDENCE_DIRECT_MODEL_ID: &str = "claude-sonnet-5";
const EVIDENCE_TOOL_NAME: &str = "claude-code";
const EVIDENCE_TOOL_VERSION: &str = "9.9.9";

fn assert_builds_expected_agent_trace_from_evidence(scenario: EvidenceScenario) {
    let mut direct_patch = parse_patch(scenario.direct, Some(EVIDENCE_DIRECT_SESSION_ID))
        .expect("direct fixture patch should parse");
    for file in &mut direct_patch.files {
        for hunk in &mut file.hunks {
            hunk.model_id = Some(String::from(EVIDENCE_DIRECT_MODEL_ID));
        }
    }
    let mutation_ai_patch =
        parse_patch(scenario.mutation_ai, None).expect("mutation-ai fixture patch should parse");
    let post_commit_patch =
        parse_patch(scenario.post_commit, None).expect("post-commit fixture patch should parse");

    let golden: Value = serde_json::from_str(scenario.golden).expect("golden json should load");
    validate_agent_trace_value(&golden).expect("golden json should validate against schema");

    let direct_source = DirectEvidenceSource {
        tool_name: Some(EVIDENCE_TOOL_NAME),
        tool_version: Some(EVIDENCE_TOOL_VERSION),
        ..DirectEvidenceSource::repository_relative(&direct_patch)
    };
    let actual = build_agent_trace_from_evidence(
        AgentTraceEvidence {
            repository_root: &RepositoryRoot::new(Path::new(TEST_REPOSITORY_ROOT)),
            direct_sources: std::slice::from_ref(&direct_source),
            mutation_ai_patch: &mutation_ai_patch,
            mutation_non_ai_patch: &ParsedPatch { files: Vec::new() },
        },
        &post_commit_patch,
        AgentTraceMetadataInput {
            commit_timestamp: TEST_COMMIT_TIMESTAMP,
            commit_revision: TEST_COMMIT_REVISION,
            vcs_type: Some(AgentTraceVcsType::Git),
        },
        &PatchNewSideFiles::from_patch(scenario.post_commit),
    )
    .expect("agent trace should build");

    assert_eq!(actual.version, AGENT_TRACE_VERSION);
    assert_eq!(actual.timestamp, TEST_COMMIT_TIMESTAMP);

    let actual_json = serde_json::to_value(&actual).expect("agent trace should serialize");
    validate_agent_trace_value(&actual_json).expect("actual json should validate against schema");

    let expected_conversation_url = agent_trace_conversation_url(&actual.id);
    let mut expected_files = golden["files"].clone();
    for conversation in expected_files
        .as_array_mut()
        .expect("golden files should be an array")
        .iter_mut()
        .flat_map(|file| {
            file["conversations"]
                .as_array_mut()
                .expect("golden conversations should be an array")
                .iter_mut()
        })
    {
        conversation["url"] = Value::String(expected_conversation_url.clone());
    }

    assert_eq!(actual_json["vcs"], golden["vcs"]);
    assert_eq!(actual_json["tool"], golden["tool"]);
    assert_eq!(
        actual_json["metadata"]["sce"]["line_changes"],
        golden["metadata"]["sce"]["line_changes"]
    );
    assert_eq!(actual_json["files"], expected_files);
}

#[test]
fn direct_only_evidence_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace_from_evidence(EvidenceScenario {
        direct: include_str!("fixtures/direct_only/direct.patch"),
        mutation_ai: include_str!("fixtures/direct_only/mutation_ai.patch"),
        post_commit: include_str!("fixtures/direct_only/post_commit.patch"),
        golden: include_str!("fixtures/direct_only/golden.json"),
    });
}

#[test]
fn exclusive_without_direct_evidence_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace_from_evidence(EvidenceScenario {
        direct: include_str!("fixtures/exclusive_without_direct/direct.patch"),
        mutation_ai: include_str!("fixtures/exclusive_without_direct/mutation_ai.patch"),
        post_commit: include_str!("fixtures/exclusive_without_direct/post_commit.patch"),
        golden: include_str!("fixtures/exclusive_without_direct/golden.json"),
    });
}

#[test]
fn direct_plus_mutation_evidence_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace_from_evidence(EvidenceScenario {
        direct: include_str!("fixtures/direct_plus_mutation/direct.patch"),
        mutation_ai: include_str!("fixtures/direct_plus_mutation/mutation_ai.patch"),
        post_commit: include_str!("fixtures/direct_plus_mutation/post_commit.patch"),
        golden: include_str!("fixtures/direct_plus_mutation/golden.json"),
    });
}

#[test]
fn partial_combined_evidence_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace_from_evidence(EvidenceScenario {
        direct: include_str!("fixtures/partial_combined/direct.patch"),
        mutation_ai: include_str!("fixtures/partial_combined/mutation_ai.patch"),
        post_commit: include_str!("fixtures/partial_combined/post_commit.patch"),
        golden: include_str!("fixtures/partial_combined/golden.json"),
    });
}

#[test]
fn newer_nonexclusive_blocks_evidence_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace_from_evidence(EvidenceScenario {
        direct: include_str!("fixtures/newer_nonexclusive_blocks/direct.patch"),
        mutation_ai: include_str!("fixtures/newer_nonexclusive_blocks/mutation_ai.patch"),
        post_commit: include_str!("fixtures/newer_nonexclusive_blocks/post_commit.patch"),
        golden: include_str!("fixtures/newer_nonexclusive_blocks/golden.json"),
    });
}

#[test]
fn mutation_only_no_provenance_evidence_matches_golden_agent_trace() {
    assert_builds_expected_agent_trace_from_evidence(EvidenceScenario {
        direct: include_str!("fixtures/mutation_only_no_provenance/direct.patch"),
        mutation_ai: include_str!("fixtures/mutation_only_no_provenance/mutation_ai.patch"),
        post_commit: include_str!("fixtures/mutation_only_no_provenance/post_commit.patch"),
        golden: include_str!("fixtures/mutation_only_no_provenance/golden.json"),
    });
}

const SOURCE_PATH: &str = "src/lib.rs";
const LINE_X: (u64, &str) = (2, "let alpha = 1;");
const LINE_Y: (u64, &str) = (20, "let omega = 2;");
const SESSION_A: &str = "cc_session-a";
const SESSION_B: &str = "oc_session-b";
const SESSION_C: &str = "cc_session-c";
const MODEL_A: &str = "model-a";
const MODEL_B: &str = "model-b";
const MODEL_C: &str = "model-c";

fn added_lines_patch(path: &str, lines: &[(u64, &str)]) -> String {
    let mut text = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
    for (line_number, content) in lines {
        write!(
            text,
            "@@ -{},0 +{line_number},1 @@\n+{content}\n",
            line_number - 1
        )
        .expect("write hunk");
    }
    text
}

fn attributed_patch(path: &str, lines: &[(u64, &str)], session: &str, model: &str) -> ParsedPatch {
    let mut attributed = parse_patch(&added_lines_patch(path, lines), Some(session))
        .expect("attributed patch should parse");
    for hunk in attributed
        .files
        .iter_mut()
        .flat_map(|file| file.hunks.iter_mut())
    {
        hunk.model_id = Some(model.to_owned());
    }
    attributed
}

fn committed_patch() -> ParsedPatch {
    parse_patch(&added_lines_patch(SOURCE_PATH, &[LINE_X, LINE_Y]), None)
        .expect("committed patch should parse")
}

struct ToolSource {
    patch: ParsedPatch,
    tool_name: Option<&'static str>,
    tool_version: Option<&'static str>,
    path_origin: DiffTracePathOrigin,
}

fn tool_source(
    path: &str,
    lines: &[(u64, &str)],
    session: &str,
    model: &str,
    tool: Option<(&'static str, Option<&'static str>)>,
) -> ToolSource {
    ToolSource {
        patch: attributed_patch(path, lines, session, model),
        tool_name: tool.map(|(name, _)| name),
        tool_version: tool.and_then(|(_, version)| version),
        path_origin: DiffTracePathOrigin::RepositoryRelative,
    }
}

fn claude_source(
    path: &str,
    lines: &[(u64, &str)],
    path_origin: DiffTracePathOrigin,
) -> ToolSource {
    ToolSource {
        path_origin,
        ..tool_source(path, lines, SESSION_A, MODEL_A, Some(("claude", None)))
    }
}

fn working_directory(cwd: &str) -> DiffTracePathOrigin {
    DiffTracePathOrigin::WorkingDirectory(cwd.to_owned())
}

fn direct_sources(sources: &[ToolSource]) -> Vec<DirectEvidenceSource<'_>> {
    sources
        .iter()
        .map(|source| DirectEvidenceSource {
            tool_name: source.tool_name,
            tool_version: source.tool_version,
            path_origin: &source.path_origin,
            ..DirectEvidenceSource::repository_relative(&source.patch)
        })
        .collect()
}

fn test_root() -> RepositoryRoot {
    RepositoryRoot::new(Path::new(TEST_REPOSITORY_ROOT))
}

fn build_with_sources(
    sources: &[ToolSource],
    mutation_ai_patch: &ParsedPatch,
) -> super::AgentTrace {
    build_trace(
        &committed_patch(),
        sources,
        mutation_ai_patch,
        &ParsedPatch { files: Vec::new() },
    )
}

fn build_trace(
    committed: &ParsedPatch,
    sources: &[ToolSource],
    mutation_ai_patch: &ParsedPatch,
    mutation_non_ai_patch: &ParsedPatch,
) -> super::AgentTrace {
    build_trace_with_content(
        committed,
        sources,
        mutation_ai_patch,
        mutation_non_ai_patch,
        &UnverifiedCommittedFiles,
    )
}

fn build_trace_with_content(
    committed: &ParsedPatch,
    sources: &[ToolSource],
    mutation_ai_patch: &ParsedPatch,
    mutation_non_ai_patch: &ParsedPatch,
    committed_files: &dyn CommittedFileReader,
) -> super::AgentTrace {
    let direct_sources = direct_sources(sources);
    let trace = build_agent_trace_from_evidence(
        AgentTraceEvidence {
            repository_root: &test_root(),
            direct_sources: &direct_sources,
            mutation_ai_patch,
            mutation_non_ai_patch,
        },
        committed,
        AgentTraceMetadataInput {
            commit_timestamp: TEST_COMMIT_TIMESTAMP,
            commit_revision: TEST_COMMIT_REVISION,
            vcs_type: Some(AgentTraceVcsType::Git),
        },
        committed_files,
    )
    .expect("agent trace should build");
    let json = serde_json::to_value(&trace).expect("agent trace should serialize");
    validate_agent_trace_value(&json).expect("agent trace should validate against schema");
    trace
}

fn no_mutation() -> ParsedPatch {
    ParsedPatch { files: Vec::new() }
}

fn tool_of(trace: &super::AgentTrace) -> Option<(Option<&str>, Option<&str>)> {
    trace
        .tool
        .as_ref()
        .map(|tool| (tool.name.as_deref(), tool.version.as_deref()))
}

fn range_kinds(trace: &super::AgentTrace) -> Vec<ContributorType> {
    trace
        .files
        .iter()
        .flat_map(|file| {
            let mut ranges: Vec<_> = file
                .conversations
                .iter()
                .flat_map(|conversation| {
                    conversation.ranges.iter().map(move |range| {
                        let contributor = range
                            .contributor
                            .as_ref()
                            .or(conversation.contributor.as_ref())
                            .expect("every range has an effective contributor");
                        (range.start_line, contributor.kind)
                    })
                })
                .collect();
            ranges.sort_by_key(|(start_line, _)| *start_line);
            ranges.into_iter().map(|(_, kind)| kind)
        })
        .collect()
}

fn range_positions(trace: &super::AgentTrace) -> Vec<(u64, u64, ContributorType)> {
    let mut positions = Vec::new();
    for file in &trace.files {
        let mut ranges: Vec<_> = file
            .conversations
            .iter()
            .flat_map(|conversation| {
                conversation.ranges.iter().map(move |range| {
                    let contributor = range
                        .contributor
                        .as_ref()
                        .or(conversation.contributor.as_ref())
                        .expect("every range has an effective contributor");
                    (range.start_line, range.end_line, contributor.kind)
                })
            })
            .collect();
        ranges.sort_by_key(|(start_line, _, _)| *start_line);
        positions.extend(ranges);
    }
    positions
}

fn related_urls(conversation: &super::Conversation) -> Vec<String> {
    conversation
        .related
        .iter()
        .flatten()
        .map(|related| related.url.clone())
        .collect()
}

fn expected_hash(bytes: &[u8]) -> String {
    let hash = murmur3::murmur3_32(&mut std::io::Cursor::new(bytes), 0).expect("murmur3");
    format!("murmur3:{hash:08x}")
}

fn accepted_line_count(sources: &[ToolSource], target: &ParsedPatch) -> usize {
    super::direct_evidence_patch(&direct_sources(sources), target, &test_root())
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .map(|hunk| hunk.lines.len())
        .sum()
}

#[test]
fn two_sessions_claiming_one_line_yield_no_accepted_direct_evidence() {
    let sources = [
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", None)),
        ),
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_B,
            MODEL_B,
            Some(("opencode", None)),
        ),
    ];

    assert!(!super::has_direct_evidence(
        &direct_sources(&sources),
        &committed_patch(),
        &test_root()
    ));
    assert_eq!(accepted_line_count(&sources, &committed_patch()), 0);

    let trace = build_with_sources(&sources, &no_mutation());
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Unknown, ContributorType::Unknown]
    );
    assert_eq!(tool_of(&trace), None);
}

#[test]
fn several_rows_from_one_session_are_accepted_as_that_session() {
    let sources = [
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", None)),
        ),
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", None)),
        ),
    ];

    assert!(super::has_direct_evidence(
        &direct_sources(&sources),
        &committed_patch(),
        &test_root()
    ));
    let trace = build_with_sources(&sources, &no_mutation());
    let conversation = &trace.files[0].conversations[0];
    assert_eq!(
        conversation
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .kind,
        ContributorType::Ai
    );
    assert_eq!(
        conversation
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .model_id
            .as_deref(),
        Some(MODEL_A)
    );
    assert_eq!(tool_of(&trace), Some((Some("claude"), None)));
}

#[test]
fn a_conflicted_line_does_not_hide_or_join_a_valid_line() {
    let sources = [
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", None)),
        ),
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_B,
            MODEL_B,
            Some(("opencode", None)),
        ),
        tool_source(
            SOURCE_PATH,
            &[LINE_Y],
            SESSION_A,
            MODEL_A,
            Some(("claude", None)),
        ),
    ];

    assert_eq!(accepted_line_count(&sources, &committed_patch()), 1);
    let trace = build_with_sources(&sources, &no_mutation());
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Unknown, ContributorType::Ai]
    );
    assert_eq!(tool_of(&trace), Some((Some("claude"), None)));
}

#[test]
fn identical_lines_in_a_different_file_with_the_same_suffix_do_not_attribute() {
    let sources = [tool_source(
        "packages/foo/src/lib.rs",
        &[LINE_X, LINE_Y],
        SESSION_B,
        MODEL_B,
        Some(("opencode", None)),
    )];

    assert!(!super::has_direct_evidence(
        &direct_sources(&sources),
        &committed_patch(),
        &test_root()
    ));
    let trace = build_with_sources(&sources, &no_mutation());
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Unknown, ContributorType::Unknown]
    );
    assert_eq!(tool_of(&trace), None);
}

#[test]
fn absolute_evidence_paths_attribute_only_inside_the_repository_root() {
    let inside = format!("{TEST_REPOSITORY_ROOT}/{SOURCE_PATH}");
    let outside = format!("/workspace/another-project/{SOURCE_PATH}");
    let sources = [
        tool_source(
            &inside,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", None)),
        ),
        tool_source(
            &outside,
            &[LINE_Y],
            SESSION_B,
            MODEL_B,
            Some(("opencode", None)),
        ),
    ];

    let trace = build_with_sources(&sources, &no_mutation());
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Ai, ContributorType::Unknown]
    );
    assert_eq!(tool_of(&trace), Some((Some("claude"), None)));
}

#[test]
fn deleted_files_match_direct_evidence_by_their_old_path() {
    let deletion = |path: &str| {
        format!(
            "diff --git a/{path} b/{path}\ndeleted file mode 100644\n--- a/{path}\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-gone\n"
        )
    };
    let committed = parse_patch(&deletion("src/old.rs"), None).expect("deletion parses");
    let matching = parse_patch(&deletion("src/old.rs"), Some(SESSION_A)).expect("deletion parses");
    let other =
        parse_patch(&deletion("vendor/src/old.rs"), Some(SESSION_A)).expect("deletion parses");

    for (candidate, expected) in [(&matching, true), (&other, false)] {
        assert_eq!(
            super::has_direct_evidence(
                &[DirectEvidenceSource::repository_relative(candidate)],
                &committed,
                &test_root()
            ),
            expected
        );
    }
}

#[test]
fn tool_comes_from_an_older_accepted_source_when_the_newest_is_rejected_by_lineage() {
    let sources = [
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", Some("1.0"))),
        ),
        tool_source(
            SOURCE_PATH,
            &[LINE_Y],
            SESSION_B,
            MODEL_B,
            Some(("opencode", Some("0.9"))),
        ),
    ];
    let mut mutation_ai = attributed_patch(SOURCE_PATH, &[LINE_X], SESSION_A, MODEL_A);
    mutation_ai
        .files
        .extend(attributed_patch(SOURCE_PATH, &[LINE_Y], SESSION_C, MODEL_C).files);

    let trace = build_with_sources(&sources, &mutation_ai);
    assert_eq!(tool_of(&trace), Some((Some("claude"), Some("1.0"))));
    let json = serde_json::to_string(&trace).expect("trace serializes");
    assert!(!json.contains("opencode") && !json.contains(SESSION_B) && !json.contains(MODEL_B));
}

#[test]
fn accepted_sources_from_one_tool_keep_that_tool() {
    let sources = [
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", Some("1.0"))),
        ),
        tool_source(
            SOURCE_PATH,
            &[LINE_Y],
            SESSION_C,
            MODEL_C,
            Some(("claude", Some("1.0"))),
        ),
    ];

    let trace = build_with_sources(&sources, &no_mutation());
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Ai, ContributorType::Ai]
    );
    assert_eq!(tool_of(&trace), Some((Some("claude"), Some("1.0"))));
}

#[test]
fn accepted_sources_from_different_tools_omit_the_tool() {
    let sources = [
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", None)),
        ),
        tool_source(
            SOURCE_PATH,
            &[LINE_Y],
            SESSION_B,
            MODEL_B,
            Some(("opencode", None)),
        ),
    ];

    let trace = build_with_sources(&sources, &no_mutation());
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Ai, ContributorType::Ai]
    );
    assert_eq!(tool_of(&trace), None);
}

#[test]
fn conflicting_tool_versions_keep_the_name_and_omit_the_version() {
    let sources = [
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", Some("1.0"))),
        ),
        tool_source(
            SOURCE_PATH,
            &[LINE_Y],
            SESSION_C,
            MODEL_C,
            Some(("claude", Some("2.0"))),
        ),
    ];

    let trace = build_with_sources(&sources, &no_mutation());
    assert_eq!(tool_of(&trace), Some((Some("claude"), None)));
}

#[test]
fn an_accepted_source_without_tool_metadata_is_not_filled_from_other_rows() {
    let sources = [
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", None)),
        ),
        tool_source(SOURCE_PATH, &[LINE_Y], SESSION_C, MODEL_C, None),
    ];

    let trace = build_with_sources(&sources, &no_mutation());
    assert_eq!(tool_of(&trace), None);
}

#[test]
fn direct_matches_fully_overridden_by_lineage_emit_no_tool() {
    let sources = [tool_source(
        SOURCE_PATH,
        &[LINE_X, LINE_Y],
        SESSION_B,
        MODEL_B,
        Some(("opencode", None)),
    )];
    let mutation_ai = attributed_patch(SOURCE_PATH, &[LINE_X, LINE_Y], SESSION_C, MODEL_C);

    let trace = build_with_sources(&sources, &mutation_ai);
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Ai, ContributorType::Ai]
    );
    assert_eq!(
        trace.files[0].conversations[0]
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .model_id
            .as_deref(),
        Some(MODEL_C)
    );
    assert_eq!(tool_of(&trace), None);
}

#[test]
fn mutation_only_attribution_emits_no_tool() {
    let mutation_ai = attributed_patch(SOURCE_PATH, &[LINE_X, LINE_Y], SESSION_C, MODEL_C);

    let trace = build_with_sources(&[], &mutation_ai);
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Ai, ContributorType::Ai]
    );
    assert_eq!(tool_of(&trace), None);
}

#[test]
fn a_session_conflicted_source_cannot_supply_the_tool() {
    let sources = [
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_A,
            MODEL_A,
            Some(("claude", None)),
        ),
        tool_source(
            SOURCE_PATH,
            &[LINE_X],
            SESSION_B,
            MODEL_B,
            Some(("opencode", None)),
        ),
    ];
    let mutation_ai = attributed_patch(SOURCE_PATH, &[LINE_Y], SESSION_C, MODEL_C);

    let trace = build_with_sources(&sources, &mutation_ai);
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Unknown, ContributorType::Ai]
    );
    assert_eq!(tool_of(&trace), None);
}

#[test]
fn claude_relative_paths_without_an_absolute_cwd_cannot_attribute_or_supply_the_tool() {
    for path_origin in [
        DiffTracePathOrigin::AbsoluteOnly,
        working_directory("relative/base"),
        working_directory(""),
    ] {
        let sources = [
            claude_source(SOURCE_PATH, &[LINE_X], path_origin.clone()),
            tool_source(
                SOURCE_PATH,
                &[LINE_Y],
                SESSION_B,
                MODEL_B,
                Some(("opencode", Some("0.9"))),
            ),
        ];

        assert_eq!(
            accepted_line_count(&sources, &committed_patch()),
            1,
            "{path_origin:?}"
        );
        assert!(!super::has_direct_evidence(
            &direct_sources(&sources[..1]),
            &committed_patch(),
            &test_root()
        ));
        let trace = build_with_sources(&sources, &no_mutation());
        assert_eq!(
            range_kinds(&trace),
            vec![ContributorType::Unknown, ContributorType::Ai],
            "{path_origin:?}"
        );
        assert_eq!(tool_of(&trace), Some((Some("opencode"), Some("0.9"))));
    }
}

#[test]
fn claude_absolute_paths_attribute_without_a_cwd_only_inside_the_repository() {
    let sources = [
        claude_source(
            &format!("{TEST_REPOSITORY_ROOT}/{SOURCE_PATH}"),
            &[LINE_X],
            DiffTracePathOrigin::AbsoluteOnly,
        ),
        claude_source(
            &format!("/workspace/another-project/{SOURCE_PATH}"),
            &[LINE_Y],
            DiffTracePathOrigin::AbsoluteOnly,
        ),
    ];

    let trace = build_with_sources(&sources, &no_mutation());
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Ai, ContributorType::Unknown]
    );
    assert_eq!(tool_of(&trace), Some((Some("claude"), None)));
}

#[test]
fn claude_relative_paths_resolve_against_their_absolute_cwd() {
    let subdirectory = format!("{TEST_REPOSITORY_ROOT}/cli");
    for (path, cwd, expected) in [
        (SOURCE_PATH, TEST_REPOSITORY_ROOT, true),
        ("../src/lib.rs", subdirectory.as_str(), true),
        (SOURCE_PATH, subdirectory.as_str(), false),
        ("../../project/src/lib.rs", subdirectory.as_str(), true),
        (
            "../../another-project/src/lib.rs",
            subdirectory.as_str(),
            false,
        ),
        ("../../../../src/lib.rs", subdirectory.as_str(), false),
    ] {
        let sources = [claude_source(path, &[LINE_X], working_directory(cwd))];
        assert_eq!(
            super::has_direct_evidence(&direct_sources(&sources), &committed_patch(), &test_root()),
            expected,
            "{cwd} {path}"
        );
    }

    let nested = parse_patch(&added_lines_patch("cli/src/lib.rs", &[LINE_X]), None)
        .expect("nested committed patch parses");
    let sources = [claude_source(
        SOURCE_PATH,
        &[LINE_X],
        working_directory(&subdirectory),
    )];
    assert!(super::has_direct_evidence(
        &direct_sources(&sources),
        &nested,
        &test_root()
    ));
}

#[test]
fn mutation_lineage_still_attributes_when_claude_direct_evidence_lacks_a_cwd() {
    let sources = [claude_source(
        SOURCE_PATH,
        &[LINE_X, LINE_Y],
        DiffTracePathOrigin::AbsoluteOnly,
    )];
    let mutation_ai = attributed_patch(SOURCE_PATH, &[LINE_X], SESSION_C, MODEL_C);

    let trace = build_with_sources(&sources, &mutation_ai);
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Ai, ContributorType::Unknown]
    );
    assert_eq!(
        trace.files[0].conversations[0]
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .model_id
            .as_deref(),
        Some(MODEL_C)
    );
    assert_eq!(tool_of(&trace), None);
    let json = serde_json::to_string(&trace).expect("trace serializes");
    assert!(!json.contains(SESSION_A) && !json.contains(MODEL_A));
}

const ADDED: char = '+';
const REMOVED: char = '-';

type Buckets = ((u64, u64), (u64, u64), (u64, u64));

fn buckets(trace: &super::AgentTrace) -> Buckets {
    let pair = |counts: LineChangeCounts| (counts.added, counts.removed);
    let LineChangeAttribution {
        ai,
        non_exclusive,
        unknown,
    } = trace.metadata.sce.line_changes;
    (pair(ai), pair(non_exclusive), pair(unknown))
}

fn line_evidence_patch(path: &str, lines: &[(char, u64, &str)]) -> String {
    let mut text = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
    for (kind, line_number, content) in lines {
        let header = if *kind == ADDED {
            format!("@@ -{},0 +{line_number},1 @@", line_number - 1)
        } else {
            format!("@@ -{line_number},1 +{},0 @@", line_number - 1)
        };
        writeln!(text, "{header}\n{kind}{content}").expect("write hunk");
    }
    text
}

fn line_source(path: &str, lines: &[(char, u64, &str)], session: &str, model: &str) -> ToolSource {
    ToolSource {
        patch: mutation_ai_lines(path, lines, session, Some(model)),
        tool_name: Some("claude"),
        tool_version: Some("1.0"),
        path_origin: DiffTracePathOrigin::RepositoryRelative,
    }
}

fn mutation_ai_lines(
    path: &str,
    lines: &[(char, u64, &str)],
    session: &str,
    model: Option<&str>,
) -> ParsedPatch {
    let mut attributed =
        parse_patch(&line_evidence_patch(path, lines), Some(session)).expect("evidence parses");
    for hunk in attributed
        .files
        .iter_mut()
        .flat_map(|file| file.hunks.iter_mut())
    {
        hunk.model_id = model.map(ToOwned::to_owned);
    }
    attributed
}

fn mutation_non_ai_lines(path: &str, lines: &[(char, u64, &str)]) -> ParsedPatch {
    parse_patch(&line_evidence_patch(path, lines), None).expect("mutation parses")
}

fn touched_totals(patch: &ParsedPatch) -> (u64, u64) {
    let mut totals = (0, 0);
    for line in patch
        .files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .flat_map(|hunk| hunk.lines.iter())
    {
        match line.kind {
            TouchedLineKind::Added => totals.0 += 1,
            TouchedLineKind::Removed => totals.1 += 1,
        }
    }
    totals
}

fn assert_counters_cover(trace: &super::AgentTrace, committed: &ParsedPatch) {
    let (ai, non_exclusive, unknown) = buckets(trace);
    assert_eq!(
        (
            ai.0 + non_exclusive.0 + unknown.0,
            ai.1 + non_exclusive.1 + unknown.1
        ),
        touched_totals(committed),
        "line_changes must cover every canonical touched line exactly once"
    );
}

const MIXED_PATH: &str = "src/mixed.rs";
const MIXED_HUNK: &str = "diff --git a/src/mixed.rs b/src/mixed.rs\n--- a/src/mixed.rs\n+++ b/src/mixed.rs\n@@ -1,4 +1,6 @@\n keep\n-old direct\n-old unknown\n+new direct\n+new lineage\n+new contended\n+new manual\n keep too\n";
const MIXED_DIRECT: &[(char, u64, &str)] = &[(REMOVED, 2, "old direct"), (ADDED, 2, "new direct")];
const MIXED_LINEAGE: &[(char, u64, &str)] = &[(ADDED, 3, "new lineage")];
const MIXED_CONTENDED: &[(char, u64, &str)] = &[(ADDED, 4, "new contended")];
const MIXED_ALL: &[(char, u64, &str)] = &[
    (REMOVED, 2, "old direct"),
    (REMOVED, 3, "old unknown"),
    (ADDED, 2, "new direct"),
    (ADDED, 3, "new lineage"),
    (ADDED, 4, "new contended"),
    (ADDED, 5, "new manual"),
];

fn mixed_committed() -> ParsedPatch {
    parse_patch(MIXED_HUNK, None).expect("mixed hunk parses")
}

#[test]
fn mixed_hunk_counts_each_touched_line_by_its_own_decision() {
    let committed = mixed_committed();
    let trace = build_trace_with_content(
        &committed,
        &[line_source(MIXED_PATH, MIXED_DIRECT, SESSION_A, MODEL_A)],
        &mutation_ai_lines(MIXED_PATH, MIXED_LINEAGE, SESSION_A, Some(MODEL_A)),
        &mutation_non_ai_lines(MIXED_PATH, MIXED_CONTENDED),
        &PatchNewSideFiles::from_patch(MIXED_HUNK),
    );

    assert_eq!(buckets(&trace), ((2, 1), (1, 0), (1, 1)));
    assert_counters_cover(&trace, &committed);
    let url = agent_trace_conversation_url(&trace.id);
    assert_eq!(
        serde_json::to_value(&trace.files).expect("files serialize"),
        serde_json::json!([{
            "path": MIXED_PATH,
            "conversations": [
                {
                    "url": url,
                    "contributor": { "type": "ai", "model_id": MODEL_A },
                    "ranges": [{
                        "start_line": 2,
                        "end_line": 3,
                        "content_hash": expected_hash(b"new direct\nnew lineage\n"),
                    }],
                    "related": [{ "type": "session", "url": agent_trace_session_url(SESSION_A) }],
                },
                {
                    "url": url,
                    "contributor": { "type": "unknown" },
                    "ranges": [{
                        "start_line": 4,
                        "end_line": 4,
                        "content_hash": expected_hash(b"new contended\n"),
                    }],
                },
                {
                    "url": url,
                    "contributor": { "type": "unknown" },
                    "ranges": [{
                        "start_line": 5,
                        "end_line": 5,
                        "content_hash": expected_hash(b"new manual\n"),
                    }],
                },
            ],
        }])
    );
    assert!(!range_kinds(&trace).contains(&ContributorType::Mixed));
    assert_eq!(tool_of(&trace), Some((Some("claude"), Some("1.0"))));
}

#[test]
fn every_canonical_touched_line_receives_exactly_one_decision() {
    let committed = mixed_committed();
    let sources = [line_source(MIXED_PATH, MIXED_DIRECT, SESSION_A, MODEL_A)];
    let mutation_ai = mutation_ai_lines(MIXED_PATH, MIXED_LINEAGE, SESSION_A, Some(MODEL_A));
    let mutation_non_ai = mutation_non_ai_lines(MIXED_PATH, MIXED_CONTENDED);
    let decisions = resolve_patch_lines(
        &committed,
        &direct_sources(&sources),
        &test_root(),
        &MutationEvidenceIndex::new(&mutation_ai, &mutation_non_ai),
    );

    let expected: BTreeSet<PatchLineLocation> = (0..MIXED_ALL.len())
        .map(|line_index| PatchLineLocation {
            file_index: 0,
            hunk_index: 0,
            line_index,
        })
        .collect();
    assert_eq!(decisions.keys().copied().collect::<BTreeSet<_>>(), expected);
    let kinds: Vec<&str> = decisions
        .values()
        .map(|decision| match decision {
            LineAttribution::Ai(_) => "ai",
            LineAttribution::NonExclusive => "non_exclusive",
            LineAttribution::Unknown => "unknown",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["ai", "unknown", "ai", "ai", "non_exclusive", "unknown"]
    );
}

#[test]
fn entirely_ai_hunk_counts_every_line_as_ai() {
    let committed = mixed_committed();
    let trace = build_trace(
        &committed,
        &[line_source(MIXED_PATH, MIXED_ALL, SESSION_A, MODEL_A)],
        &no_mutation(),
        &no_mutation(),
    );

    assert_eq!(buckets(&trace), ((4, 2), (0, 0), (0, 0)));
    assert_counters_cover(&trace, &committed);
    assert_eq!(range_kinds(&trace), vec![ContributorType::Ai]);
}

#[test]
fn entirely_unknown_hunk_counts_every_line_as_unknown() {
    let committed = mixed_committed();
    let trace = build_trace(&committed, &[], &no_mutation(), &no_mutation());

    assert_eq!(buckets(&trace), ((0, 0), (0, 0), (4, 2)));
    assert_counters_cover(&trace, &committed);
    assert_eq!(range_kinds(&trace), vec![ContributorType::Unknown]);
}

#[test]
fn non_exclusive_mutation_overrides_direct_ai_evidence_in_counters_and_provenance() {
    let committed = committed_patch();
    let lines: &[(char, u64, &str)] = &[(ADDED, LINE_X.0, LINE_X.1)];
    let trace = build_trace(
        &committed,
        &[line_source(SOURCE_PATH, lines, SESSION_B, MODEL_B)],
        &no_mutation(),
        &mutation_non_ai_lines(SOURCE_PATH, lines),
    );

    assert_eq!(buckets(&trace), ((0, 0), (1, 0), (1, 0)));
    assert_counters_cover(&trace, &committed);
    assert_eq!(
        range_kinds(&trace),
        vec![ContributorType::Unknown, ContributorType::Unknown]
    );
    let conversation = &trace.files[0].conversations[0];
    assert_eq!(
        conversation
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .model_id,
        None
    );
    assert_eq!(conversation.related, None);
    assert_eq!(tool_of(&trace), None);
    let json = serde_json::to_string(&trace).expect("trace serializes");
    assert!(!json.contains(SESSION_B) && !json.contains(MODEL_B) && !json.contains("claude"));
}

#[test]
fn conflicting_direct_sessions_count_as_unknown() {
    let committed = committed_patch();
    let lines: &[(char, u64, &str)] = &[(ADDED, LINE_X.0, LINE_X.1)];
    let trace = build_trace(
        &committed,
        &[
            line_source(SOURCE_PATH, lines, SESSION_A, MODEL_A),
            line_source(SOURCE_PATH, lines, SESSION_B, MODEL_B),
        ],
        &no_mutation(),
        &no_mutation(),
    );

    assert_eq!(buckets(&trace), ((0, 0), (0, 0), (2, 0)));
    assert_eq!(tool_of(&trace), None);
}

#[test]
fn mutation_only_attribution_without_a_model_counts_as_ai() {
    let committed = committed_patch();
    let trace = build_trace(
        &committed,
        &[],
        &mutation_ai_lines(SOURCE_PATH, &[(ADDED, LINE_X.0, LINE_X.1)], SESSION_C, None),
        &no_mutation(),
    );

    assert_eq!(buckets(&trace), ((1, 0), (0, 0), (1, 0)));
    let conversation = &trace.files[0].conversations[0];
    assert_eq!(
        conversation
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .kind,
        ContributorType::Ai
    );
    assert_eq!(
        conversation
            .contributor
            .as_ref()
            .expect("conversation contributor")
            .model_id,
        None
    );
}

#[test]
fn a_removed_line_without_direct_evidence_counts_as_unknown_regardless_of_its_author() {
    let committed = parse_patch(
        &line_evidence_patch(SOURCE_PATH, &[(REMOVED, LINE_X.0, LINE_X.1)]),
        None,
    )
    .expect("deletion parses");
    let introduced: &[(char, u64, &str)] = &[(ADDED, LINE_X.0, LINE_X.1)];
    let trace = build_trace(
        &committed,
        &[line_source(SOURCE_PATH, introduced, SESSION_A, MODEL_A)],
        &mutation_ai_lines(SOURCE_PATH, introduced, SESSION_A, Some(MODEL_A)),
        &no_mutation(),
    );

    assert_eq!(buckets(&trace), ((0, 0), (0, 0), (0, 1)));
    assert!(trace.files.is_empty());
    assert_eq!(tool_of(&trace), None);
}

#[test]
fn duplicate_identical_lines_are_each_counted_once_and_only_supported_ones_are_ai() {
    let committed = parse_patch(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +2,3 @@\n+dup\n+dup\n+dup\n@@ -9,0 +12,1 @@\n+dup\n",
        None,
    )
    .expect("committed parses");
    let trace = build_trace(
        &committed,
        &[line_source(
            SOURCE_PATH,
            &[(ADDED, 3, "dup")],
            SESSION_A,
            MODEL_A,
        )],
        &mutation_ai_lines(SOURCE_PATH, &[(ADDED, 12, "dup")], SESSION_C, Some(MODEL_C)),
        &no_mutation(),
    );

    assert_eq!(buckets(&trace), ((2, 0), (0, 0), (2, 0)));
    assert_counters_cover(&trace, &committed);
    assert_eq!(
        range_positions(&trace),
        vec![
            (2, 2, ContributorType::Unknown),
            (3, 3, ContributorType::Ai),
            (4, 4, ContributorType::Unknown),
            (12, 12, ContributorType::Ai)
        ]
    );
    let sessions: Vec<_> = trace.files[0]
        .conversations
        .iter()
        .map(related_urls)
        .collect();
    assert_eq!(
        sessions,
        vec![
            vec![],
            vec![agent_trace_session_url(SESSION_A)],
            vec![agent_trace_session_url(SESSION_C)]
        ]
    );
}

#[test]
fn counters_sum_every_touched_line_across_multiple_hunks_and_files() {
    let committed = parse_patch(
        "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 +1,2 @@\n-a old\n+a one\n+a two\n@@ -10,0 +12,1 @@\n+a three\ndiff --git a/src/b.rs b/src/b.rs\n--- a/src/b.rs\n+++ b/src/b.rs\n@@ -4,2 +4,1 @@\n-b old one\n-b old two\n+b new\n",
        None,
    )
    .expect("committed parses");
    let trace = build_trace(
        &committed,
        &[
            line_source("src/a.rs", &[(ADDED, 1, "a one")], SESSION_A, MODEL_A),
            line_source("src/b.rs", &[(REMOVED, 5, "b old two")], SESSION_A, MODEL_A),
        ],
        &mutation_ai_lines(
            "src/a.rs",
            &[(ADDED, 12, "a three")],
            SESSION_A,
            Some(MODEL_A),
        ),
        &mutation_non_ai_lines("src/b.rs", &[(ADDED, 4, "b new")]),
    );

    assert_eq!(buckets(&trace), ((2, 1), (1, 0), (1, 2)));
    assert_counters_cover(&trace, &committed);
    assert_eq!(
        range_kinds(&trace),
        vec![
            ContributorType::Ai,
            ContributorType::Unknown,
            ContributorType::Ai,
            ContributorType::Unknown
        ]
    );
    let session_a = &trace.files[0].conversations[0];
    assert_eq!(
        session_a
            .ranges
            .iter()
            .map(|range| (range.start_line, range.end_line))
            .collect::<Vec<_>>(),
        vec![(1, 1), (12, 12)]
    );
    assert_eq!(
        related_urls(session_a),
        vec![agent_trace_session_url(SESSION_A)]
    );
}

#[test]
fn a_deleted_patch_file_counts_only_its_literal_deleted_lines() {
    let embedded = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +2,1 @@\n+let alpha = 1;\n";
    let embedded_lines = embedded.lines().count();
    let mut committed_text = format!(
        "diff --git a/changes.patch b/changes.patch\ndeleted file mode 100644\n--- a/changes.patch\n+++ /dev/null\n@@ -1,{embedded_lines} +0,0 @@\n"
    );
    for line in embedded.lines() {
        writeln!(committed_text, "-{line}").expect("write deleted line");
    }
    let committed = parse_patch(&committed_text, None).expect("committed parses");
    let trace = build_trace(
        &committed,
        &[line_source(
            SOURCE_PATH,
            &[(ADDED, LINE_X.0, LINE_X.1)],
            SESSION_A,
            MODEL_A,
        )],
        &no_mutation(),
        &no_mutation(),
    );

    assert!(trace.files.is_empty());
    assert_eq!(tool_of(&trace), None);
    assert_eq!(
        buckets(&trace),
        ((0, 0), (0, 0), (0, embedded_lines as u64))
    );
    assert_counters_cover(&trace, &committed);
}

fn accepted(session: &str, model: Option<&str>, source: usize) -> DirectLineEvidence {
    DirectLineEvidence::Accepted(AcceptedDirectLine {
        provenance: provenance(Some(session), model),
        sources: BTreeSet::from([source]),
    })
}

fn provenance(session: Option<&str>, model: Option<&str>) -> PatchLineProvenance {
    PatchLineProvenance {
        session_id: session.map(ToOwned::to_owned),
        model_id: model.map(ToOwned::to_owned),
    }
}

fn ai_decision(session: &str, model: Option<&str>, direct_sources: &[usize]) -> LineAttribution {
    LineAttribution::Ai(ResolvedLine {
        provenance: provenance(Some(session), model),
        direct_sources: direct_sources.iter().copied().collect(),
    })
}

#[test]
fn resolve_line_follows_mutation_precedence_over_direct_evidence() {
    let lineage = || MutationLineEvidence::Ai(provenance(Some(SESSION_C), None));
    let cases = [
        (
            lineage(),
            Some(accepted(SESSION_C, Some(MODEL_C), 0)),
            ai_decision(SESSION_C, Some(MODEL_C), &[0]),
        ),
        (
            lineage(),
            Some(accepted(SESSION_A, Some(MODEL_A), 0)),
            ai_decision(SESSION_C, None, &[]),
        ),
        (
            lineage(),
            Some(DirectLineEvidence::Conflicting),
            ai_decision(SESSION_C, None, &[]),
        ),
        (lineage(), None, ai_decision(SESSION_C, None, &[])),
        (
            MutationLineEvidence::NonAi,
            Some(accepted(SESSION_A, Some(MODEL_A), 0)),
            LineAttribution::NonExclusive,
        ),
        (
            MutationLineEvidence::NonAi,
            Some(DirectLineEvidence::Conflicting),
            LineAttribution::NonExclusive,
        ),
        (
            MutationLineEvidence::NonAi,
            None,
            LineAttribution::NonExclusive,
        ),
        (
            MutationLineEvidence::Unresolved,
            Some(accepted(SESSION_A, Some(MODEL_A), 1)),
            ai_decision(SESSION_A, Some(MODEL_A), &[1]),
        ),
        (
            MutationLineEvidence::Unresolved,
            Some(DirectLineEvidence::Conflicting),
            LineAttribution::Unknown,
        ),
        (
            MutationLineEvidence::Unresolved,
            None,
            LineAttribution::Unknown,
        ),
    ];

    for (mutation, direct, expected) in cases {
        let label = format!("{mutation:?} + {direct:?}");
        assert_eq!(resolve_line(mutation, direct.as_ref()), expected, "{label}");
    }
}

const RANDOMIZED_SEEDS: u64 = 256;

type FlatLines = Vec<(usize, TouchedLineKind, LineAttribution)>;

fn random_decision(rng: &mut StdRng) -> LineAttribution {
    match rng.gen_range(0..3) {
        0 => ai_decision(SESSION_A, Some(MODEL_A), &[]),
        1 => LineAttribution::NonExclusive,
        _ => LineAttribution::Unknown,
    }
}

fn random_lines(rng: &mut StdRng) -> FlatLines {
    let files = rng.gen_range(1..=4);
    let mut lines = Vec::new();
    for file in 0..files {
        for _ in 0..rng.gen_range(1..=12) {
            let kind = if rng.gen_bool(0.5) {
                TouchedLineKind::Added
            } else {
                TouchedLineKind::Removed
            };
            lines.push((file, kind, random_decision(rng)));
        }
    }
    lines
}

fn layout(
    lines: &FlatLines,
    file_order: &[usize],
    max_hunk_len: usize,
) -> (ParsedPatch, BTreeMap<PatchLineLocation, LineAttribution>) {
    let mut text = String::new();
    let mut decisions = BTreeMap::new();
    for (file_index, file) in file_order.iter().enumerate() {
        let path = format!("src/file_{file}.rs");
        write!(
            text,
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n"
        )
        .expect("write file header");
        let file_lines: Vec<_> = lines.iter().filter(|line| line.0 == *file).collect();
        let (mut old_start, mut new_start) = (1_u64, 1_u64);
        for (hunk_index, hunk) in file_lines.chunks(max_hunk_len).enumerate() {
            let removed = hunk
                .iter()
                .filter(|line| line.1 == TouchedLineKind::Removed)
                .count() as u64;
            let added = hunk.len() as u64 - removed;
            writeln!(text, "@@ -{old_start},{removed} +{new_start},{added} @@")
                .expect("write hunk header");
            for (line_index, (_, kind, decision)) in hunk.iter().enumerate() {
                let marker = match kind {
                    TouchedLineKind::Added => ADDED,
                    TouchedLineKind::Removed => REMOVED,
                };
                writeln!(text, "{marker}dup").expect("write line");
                decisions.insert(
                    PatchLineLocation {
                        file_index,
                        hunk_index,
                        line_index,
                    },
                    decision.clone(),
                );
            }
            old_start += removed + 1;
            new_start += added + 1;
        }
    }
    (
        parse_patch(&text, None).expect("randomized patch parses"),
        decisions,
    )
}

fn expected_counts(lines: &FlatLines) -> LineChangeAttribution {
    let mut counts = LineChangeAttribution::default();
    for (_, kind, decision) in lines {
        let bucket = match decision {
            LineAttribution::Ai(_) => &mut counts.ai,
            LineAttribution::NonExclusive => &mut counts.non_exclusive,
            LineAttribution::Unknown => &mut counts.unknown,
        };
        match kind {
            TouchedLineKind::Added => bucket.added += 1,
            TouchedLineKind::Removed => bucket.removed += 1,
        }
    }
    counts
}

#[test]
fn randomized_counters_are_exact_and_independent_of_grouping_and_order() {
    for seed in 0..RANDOMIZED_SEEDS {
        let mut rng = StdRng::seed_from_u64(seed);
        let lines = random_lines(&mut rng);
        let files: Vec<usize> = (0..=lines.iter().map(|line| line.0).max().unwrap()).collect();
        let expected = expected_counts(&lines);

        let (patch, decisions) = layout(&lines, &files, rng.gen_range(1..=6));
        let counts = line_change_attribution(&patch, &decisions);
        assert_eq!(counts, expected, "seed {seed}");
        assert_eq!(decisions.len(), lines.len(), "seed {seed}");
        assert_eq!(
            (
                counts.ai.added + counts.non_exclusive.added + counts.unknown.added,
                counts.ai.removed + counts.non_exclusive.removed + counts.unknown.removed,
            ),
            touched_totals(&patch),
            "seed {seed}"
        );

        for max_hunk_len in [1, 3, usize::MAX] {
            let (patch, decisions) = layout(&lines, &files, max_hunk_len);
            assert_eq!(
                line_change_attribution(&patch, &decisions),
                expected,
                "seed {seed} hunk partition {max_hunk_len}"
            );
        }

        let reversed: Vec<usize> = files.iter().rev().copied().collect();
        let (patch, decisions) = layout(&lines, &reversed, 2);
        assert_eq!(
            line_change_attribution(&patch, &decisions),
            expected,
            "seed {seed} reordered files"
        );

        if let Some(index) = lines
            .iter()
            .position(|line| matches!(line.2, LineAttribution::Ai(_)))
        {
            let mut demoted = lines.clone();
            demoted[index].2 = LineAttribution::Unknown;
            let (patch, decisions) = layout(&demoted, &files, 4);
            let counts = line_change_attribution(&patch, &decisions);
            let mut adjusted = expected;
            match lines[index].1 {
                TouchedLineKind::Added => {
                    adjusted.ai.added -= 1;
                    adjusted.unknown.added += 1;
                }
                TouchedLineKind::Removed => {
                    adjusted.ai.removed -= 1;
                    adjusted.unknown.removed += 1;
                }
            }
            assert_eq!(counts, adjusted, "seed {seed} demoted line {index}");
        }
    }
}

const EXAMPLE_PATH: &str = "src/example.ts";
const EXAMPLE_PATCH: &str = "diff --git a/src/example.ts b/src/example.ts\n--- a/src/example.ts\n+++ b/src/example.ts\n@@ -10,2 +10,4 @@\n-old\n+let a = 1;\n+let b = 2;\n+let c = 3;\n unchanged\n";

#[test]
fn a_hunk_with_ai_and_unresolved_additions_emits_homogeneous_conversations_without_context() {
    let committed = parse_patch(EXAMPLE_PATCH, None).expect("example parses");
    let trace = build_trace_with_content(
        &committed,
        &[line_source(
            EXAMPLE_PATH,
            &[(ADDED, 10, "let a = 1;"), (ADDED, 11, "let b = 2;")],
            SESSION_A,
            MODEL_A,
        )],
        &no_mutation(),
        &no_mutation(),
        &PatchNewSideFiles::from_patch(EXAMPLE_PATCH),
    );

    let url = agent_trace_conversation_url(&trace.id);
    assert_eq!(
        serde_json::to_value(&trace.files).expect("files serialize"),
        serde_json::json!([{
            "path": EXAMPLE_PATH,
            "conversations": [
                {
                    "url": url,
                    "contributor": { "type": "ai", "model_id": MODEL_A },
                    "ranges": [{
                        "start_line": 10,
                        "end_line": 11,
                        "content_hash": expected_hash(b"let a = 1;\nlet b = 2;\n"),
                    }],
                    "related": [{ "type": "session", "url": agent_trace_session_url(SESSION_A) }],
                },
                {
                    "url": url,
                    "contributor": { "type": "unknown" },
                    "ranges": [{
                        "start_line": 12,
                        "end_line": 12,
                        "content_hash": expected_hash(b"let c = 3;\n"),
                    }],
                },
            ],
        }])
    );
    assert_eq!(buckets(&trace), ((2, 0), (0, 0), (1, 1)));
}

fn unresolved_trace(patch: &str) -> super::AgentTrace {
    build_trace_with_content(
        &parse_patch(patch, None).expect("patch parses"),
        &[],
        &no_mutation(),
        &no_mutation(),
        &PatchNewSideFiles::from_patch(patch),
    )
}

#[test]
fn additions_separated_by_context_produce_separate_ranges_in_one_conversation() {
    let trace = unresolved_trace(
        "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -4,2 +4,5 @@\n+one\n keep\n+two\n+three\n keep too\n",
    );
    assert_eq!(trace.files[0].conversations.len(), 1);
    assert_eq!(
        range_positions(&trace),
        vec![
            (4, 4, ContributorType::Unknown),
            (6, 7, ContributorType::Unknown)
        ]
    );
}

#[test]
fn new_file_ranges_start_at_line_one() {
    let trace = unresolved_trace(
        "diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+first\n+second\n",
    );
    assert_eq!(trace.files[0].path, "new.txt");
    assert_eq!(
        range_positions(&trace),
        vec![(1, 2, ContributorType::Unknown)]
    );
}

#[test]
fn deleted_files_and_pure_renames_emit_no_ranges_but_keep_counters() {
    let patch = "diff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\n--- a/gone.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\ndiff --git a/old.rs b/new.rs\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\n";
    let trace = unresolved_trace(patch);
    assert!(trace.files.is_empty());
    assert_eq!(buckets(&trace), ((0, 0), (0, 0), (0, 2)));
}

#[test]
fn renamed_files_with_additions_use_the_new_path_and_positions() {
    let trace = unresolved_trace(
        "diff --git a/old.rs b/new.rs\nsimilarity index 80%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -2,1 +2,2 @@\n keep\n+added\n",
    );
    assert_eq!(trace.files.len(), 1);
    assert_eq!(trace.files[0].path, "new.rs");
    assert_eq!(
        range_positions(&trace),
        vec![(3, 3, ContributorType::Unknown)]
    );
}

#[test]
fn adjacent_lines_from_different_sessions_become_separate_conversations() {
    let patch =
        "diff --git a/s.rs b/s.rs\n--- a/s.rs\n+++ b/s.rs\n@@ -0,0 +1,2 @@\n+from a\n+from b\n";
    let trace = build_trace_with_content(
        &parse_patch(patch, None).expect("patch parses"),
        &[
            line_source("s.rs", &[(ADDED, 1, "from a")], SESSION_A, MODEL_A),
            line_source("s.rs", &[(ADDED, 2, "from b")], SESSION_B, MODEL_A),
        ],
        &no_mutation(),
        &no_mutation(),
        &PatchNewSideFiles::from_patch(patch),
    );
    let conversations = &trace.files[0].conversations;
    assert_eq!(conversations.len(), 2);
    assert_eq!(
        related_urls(&conversations[0]),
        vec![agent_trace_session_url(SESSION_A)]
    );
    assert_eq!(
        related_urls(&conversations[1]),
        vec![agent_trace_session_url(SESSION_B)]
    );
    assert_eq!(
        (
            conversations[0].ranges[0].start_line,
            conversations[1].ranges[0].start_line
        ),
        (1, 2)
    );
}

#[test]
fn sessionless_lines_with_different_models_never_share_a_model() {
    let patch =
        "diff --git a/m.rs b/m.rs\n--- a/m.rs\n+++ b/m.rs\n@@ -0,0 +1,2 @@\n+model a\n+model b\n";
    let mut mutation_ai = mutation_ai_lines("m.rs", &[(ADDED, 1, "model a")], "", Some(MODEL_A));
    mutation_ai
        .files
        .extend(mutation_ai_lines("m.rs", &[(ADDED, 2, "model b")], "", Some(MODEL_B)).files);
    let trace = build_trace_with_content(
        &parse_patch(patch, None).expect("patch parses"),
        &[],
        &mutation_ai,
        &no_mutation(),
        &PatchNewSideFiles::from_patch(patch),
    );
    let models: Vec<_> = trace.files[0]
        .conversations
        .iter()
        .map(|conversation| {
            assert_eq!(conversation.related, None);
            conversation
                .contributor
                .as_ref()
                .and_then(|contributor| contributor.model_id.clone())
        })
        .collect();
    assert_eq!(
        models,
        vec![Some(MODEL_A.to_owned()), Some(MODEL_B.to_owned())]
    );
}

#[test]
fn one_session_with_several_models_keeps_one_conversation_with_range_overrides() {
    let patch = "diff --git a/h.rs b/h.rs\n--- a/h.rs\n+++ b/h.rs\n@@ -0,0 +1,3 @@\n+first\n+second\n+third\n";
    let mut mutation_ai = mutation_ai_lines(
        "h.rs",
        &[(ADDED, 1, "first"), (ADDED, 3, "third")],
        SESSION_A,
        Some(MODEL_A),
    );
    mutation_ai
        .files
        .extend(mutation_ai_lines("h.rs", &[(ADDED, 2, "second")], SESSION_A, Some(MODEL_B)).files);
    let trace = build_trace_with_content(
        &parse_patch(patch, None).expect("patch parses"),
        &[],
        &mutation_ai,
        &no_mutation(),
        &PatchNewSideFiles::from_patch(patch),
    );

    let url = agent_trace_conversation_url(&trace.id);
    let range = |line: u64, bytes: &[u8], model: &str| {
        serde_json::json!({
            "start_line": line,
            "end_line": line,
            "content_hash": expected_hash(bytes),
            "contributor": { "type": "ai", "model_id": model },
        })
    };
    assert_eq!(
        serde_json::to_value(&trace.files[0].conversations).expect("serialize"),
        serde_json::json!([{
            "url": url,
            "contributor": { "type": "ai" },
            "ranges": [
                range(1, b"first\n", MODEL_A),
                range(2, b"second\n", MODEL_B),
                range(3, b"third\n", MODEL_A),
            ],
            "related": [{ "type": "session", "url": agent_trace_session_url(SESSION_A) }],
        }])
    );
}

#[test]
fn unknown_and_non_exclusive_lines_never_share_a_range() {
    use super::ranges::{coalesce_ranges, AttributedLine, RangeAttribution};

    let line = |line_number, attribution| AttributedLine {
        line_number,
        attribution,
    };
    let ranges = coalesce_ranges(&[
        line(1, RangeAttribution::Unknown),
        line(2, RangeAttribution::Unknown),
        line(3, RangeAttribution::NonExclusive),
        line(4, RangeAttribution::NonExclusive),
        line(6, RangeAttribution::NonExclusive),
    ]);
    let spans: Vec<_> = ranges
        .iter()
        .map(|range| (range.start_line, range.end_line, range.attribution.clone()))
        .collect();
    assert_eq!(
        spans,
        vec![
            (1, 2, RangeAttribution::Unknown),
            (3, 4, RangeAttribution::NonExclusive),
            (6, 6, RangeAttribution::NonExclusive),
        ]
    );
}

#[test]
fn legacy_records_with_mixed_contributors_and_hunk_hashes_still_deserialize() {
    let legacy = serde_json::json!({
        "version": "0.1.0",
        "id": "01962f15-2d3d-7c85-9f6b-0a8b4f6b2fd1",
        "timestamp": "2026-04-23T10:20:30Z",
        "files": [{
            "path": "src/lib.rs",
            "conversations": [{
                "url": "https://sce.crocoder.dev/conversations/01962f15-2d3d-7c85-9f6b-0a8b4f6b2fd1",
                "contributor": { "type": "mixed", "model_id": MODEL_A },
                "ranges": [{ "start_line": 1, "end_line": 4, "content_hash": "murmur3:0badf00d" }],
            }, {
                "url": "https://sce.crocoder.dev/conversations/01962f15-2d3d-7c85-9f6b-0a8b4f6b2fd1",
                "ranges": [{ "start_line": 7, "end_line": 7, "contributor": { "type": "human" } }],
            }],
        }],
    });
    validate_agent_trace_value(&legacy).expect("legacy record validates");
    let trace: super::AgentTrace = serde_json::from_value(legacy).expect("legacy deserializes");
    let conversations = &trace.files[0].conversations;
    assert_eq!(
        conversations[0].contributor.as_ref().map(|c| c.kind),
        Some(ContributorType::Mixed)
    );
    assert_eq!(
        conversations[0].ranges[0].content_hash.as_deref(),
        Some("murmur3:0badf00d")
    );
    assert_eq!(conversations[1].contributor, None);
    assert_eq!(
        conversations[1].ranges[0]
            .contributor
            .as_ref()
            .map(|c| c.kind),
        Some(ContributorType::Human)
    );
    assert_eq!(conversations[1].ranges[0].content_hash, None);
}

#[test]
fn content_hash_matches_known_murmur3_vectors() {
    use super::committed_content::content_hash;

    assert_eq!(content_hash(b""), "murmur3:00000000");
    assert_eq!(content_hash(b"hello"), "murmur3:248bfa47");
    assert_eq!(
        content_hash(b"The quick brown fox jumps over the lazy dog"),
        "murmur3:2e4ff723"
    );
}

#[test]
fn committed_lines_keep_exact_terminators_and_unicode() {
    use super::committed_content::CommittedLines;

    let bytes = "lf\r\ncrlf\r\n\n   \nünïcødé ✓\nno final newline".as_bytes();
    let lines = CommittedLines::new(bytes);
    assert_eq!(lines.line_count(), 6);
    assert_eq!(lines.span(1, 1), Some(&b"lf\r\n"[..]));
    assert_eq!(lines.span(2, 3), Some(&b"crlf\r\n\n"[..]));
    assert_eq!(lines.span(4, 4), Some(&b"   \n"[..]));
    assert_eq!(lines.span(5, 5), Some("ünïcødé ✓\n".as_bytes()));
    assert_eq!(lines.span(6, 6), Some(&b"no final newline"[..]));
    assert_eq!(lines.span(1, 6), Some(bytes));
    assert_eq!(lines.span(0, 1), None);
    assert_eq!(lines.span(3, 2), None);
    assert_eq!(lines.span(6, 7), None);
    assert!(lines.line_matches(2, "crlf"));
    assert!(lines.line_matches(3, ""));
    assert!(lines.line_matches(6, "no final newline"));
    assert!(!lines.line_matches(4, ""));
    assert_eq!(CommittedLines::new(b"").line_count(), 0);
    assert_eq!(CommittedLines::new(b"\n").line_count(), 1);
}

#[test]
fn identical_committed_bytes_hash_identically_across_paths_positions_and_hunks() {
    let patch = "diff --git a/one.rs b/one.rs\n--- a/one.rs\n+++ b/one.rs\n@@ -0,0 +1,2 @@\n+same\n+bytes\ndiff --git a/two.rs b/two.rs\n--- a/two.rs\n+++ b/two.rs\n@@ -10,0 +11,1 @@\n+same\n@@ -20,0 +22,1 @@\n+bytes\n@@ -30,0 +33,2 @@\n+same\n+bytes\n";
    let trace = unresolved_trace(patch);
    let hashes: Vec<Vec<(u64, Option<String>)>> = trace
        .files
        .iter()
        .map(|file| {
            file.conversations[0]
                .ranges
                .iter()
                .map(|range| (range.start_line, range.content_hash.clone()))
                .collect()
        })
        .collect();
    let pair = Some(expected_hash(b"same\nbytes\n"));
    assert_eq!(
        hashes,
        vec![
            vec![(1, pair.clone())],
            vec![
                (11, Some(expected_hash(b"same\n"))),
                (22, Some(expected_hash(b"bytes\n"))),
                (33, pair),
            ],
        ]
    );
}

#[test]
fn crlf_and_missing_final_newline_are_hashed_byte_exactly() {
    let patch = "diff --git a/w.txt b/w.txt\nnew file mode 100644\n--- /dev/null\n+++ b/w.txt\n@@ -0,0 +1,3 @@\n+crlf\r\n+  \r\n+tail ✓\n\\ No newline at end of file\n";
    let trace = unresolved_trace(patch);
    assert_eq!(
        trace.files[0].conversations[0].ranges[0].content_hash,
        Some(expected_hash("crlf\r\n  \r\ntail ✓".as_bytes()))
    );
    assert_ne!(
        trace.files[0].conversations[0].ranges[0].content_hash,
        Some(expected_hash("crlf\n  \ntail ✓\n".as_bytes()))
    );
}

#[test]
fn an_explicitly_unverified_reader_keeps_positions_without_a_hash() {
    let committed = parse_patch(EXAMPLE_PATCH, None).expect("example parses");
    let trace = build_trace(&committed, &[], &no_mutation(), &no_mutation());
    let range = &trace.files[0].conversations[0].ranges[0];
    assert_eq!((range.start_line, range.end_line), (10, 12));
    assert_eq!(range.content_hash, None);
}

struct FixedCommittedFile(CommittedFileRead);

impl CommittedFileReader for FixedCommittedFile {
    fn read_committed_file(&self, _path: &str) -> CommittedFileRead {
        self.0.clone()
    }
}

#[test]
fn non_line_addressable_paths_emit_no_ranges() {
    let committed = parse_patch(EXAMPLE_PATCH, None).expect("example parses");
    let trace = build_trace_with_content(
        &committed,
        &[],
        &no_mutation(),
        &no_mutation(),
        &FixedCommittedFile(Ok(CommittedFile::NotLineAddressable)),
    );
    assert!(trace.files.is_empty());
    assert_eq!(buckets(&trace), ((0, 0), (0, 0), (3, 1)));
}

#[test]
fn positions_that_disagree_with_committed_content_are_rejected() {
    let committed = parse_patch(EXAMPLE_PATCH, None).expect("example parses");
    let mut short = PatchNewSideFiles::from_patch(EXAMPLE_PATCH);
    short
        .0
        .insert(EXAMPLE_PATH.to_owned(), b"only one line\n".to_vec());
    let mut different = PatchNewSideFiles::from_patch(EXAMPLE_PATCH);
    let mut bytes = different.bytes(EXAMPLE_PATH).to_vec();
    let edited = bytes.len() - "unchanged\n".len() - 2;
    bytes[edited] = b'9';
    different.0.insert(EXAMPLE_PATH.to_owned(), bytes);

    for reader in [short, different] {
        let error = build_agent_trace_from_evidence(
            AgentTraceEvidence {
                repository_root: &test_root(),
                direct_sources: &[],
                mutation_ai_patch: &no_mutation(),
                mutation_non_ai_patch: &no_mutation(),
            },
            &committed,
            AgentTraceMetadataInput {
                commit_timestamp: TEST_COMMIT_TIMESTAMP,
                commit_revision: TEST_COMMIT_REVISION,
                vcs_type: Some(AgentTraceVcsType::Git),
            },
            &reader,
        )
        .expect_err("inconsistent positions must not be accepted");
        assert!(
            error
                .to_string()
                .starts_with("Invalid Agent Trace range position for 'src/example.ts'"),
            "{error}"
        );
    }
}

const RANDOM_LINE_CONTENTS: &[&str] = &["", "   ", "dup", "x = 1;", "ünïcødé ✓", "\tfn f() {}"];

#[derive(Clone, Debug)]
struct RandomFile {
    path: String,
    new_lines: Vec<(Vec<u8>, Option<LineAttribution>)>,
    patch: crate::services::patch::PatchFileChange,
    decisions: Vec<LineAttribution>,
}

fn random_attribution(rng: &mut StdRng) -> LineAttribution {
    let ai = |session: Option<&str>, model: Option<&str>| {
        LineAttribution::Ai(ResolvedLine {
            provenance: provenance(session, model),
            direct_sources: BTreeSet::new(),
        })
    };
    match rng.gen_range(0..7) {
        0 => ai(Some(SESSION_A), Some(MODEL_A)),
        1 => ai(Some(SESSION_A), Some(MODEL_B)),
        2 => ai(Some(SESSION_B), Some(MODEL_A)),
        3 => ai(None, Some(MODEL_A)),
        4 => ai(None, None),
        5 => LineAttribution::NonExclusive,
        _ => LineAttribution::Unknown,
    }
}

fn random_line(rng: &mut StdRng) -> Vec<u8> {
    let content = RANDOM_LINE_CONTENTS[rng.gen_range(0..RANDOM_LINE_CONTENTS.len())];
    let terminator = if rng.gen_bool(0.25) { "\r\n" } else { "\n" };
    format!("{content}{terminator}").into_bytes()
}

fn touched_line(
    kind: TouchedLineKind,
    line_number: u64,
    bytes: &[u8],
) -> crate::services::patch::TouchedLine {
    let text = std::str::from_utf8(bytes).expect("utf-8 line");
    let text = text.strip_suffix('\n').unwrap_or(text);
    crate::services::patch::TouchedLine {
        kind,
        line_number,
        content: text.strip_suffix('\r').unwrap_or(text).to_owned(),
        session_id: None,
    }
}

fn random_file(rng: &mut StdRng, index: usize) -> RandomFile {
    use crate::services::patch::{FileChangeKind, PatchFileChange};

    let mut new_lines = Vec::new();
    let mut touched = Vec::new();
    let mut old_line = 1_u64;
    for _ in 0..rng.gen_range(1..=8) {
        if rng.gen_bool(0.4) {
            for _ in 0..rng.gen_range(1..=3) {
                new_lines.push((random_line(rng), None));
                old_line += 1;
            }
        } else {
            for _ in 0..rng.gen_range(0..=2) {
                let decision = random_attribution(rng);
                touched.push((
                    touched_line(TouchedLineKind::Removed, old_line, &random_line(rng)),
                    decision.clone(),
                ));
                old_line += 1;
            }
            for _ in 0..rng.gen_range(0..=4) {
                let bytes = random_line(rng);
                let decision = random_attribution(rng);
                let line_number = new_lines.len() as u64 + 1;
                touched.push((
                    touched_line(TouchedLineKind::Added, line_number, &bytes),
                    decision.clone(),
                ));
                new_lines.push((bytes, Some(decision)));
            }
        }
    }
    let last_has_content = new_lines
        .last()
        .is_some_and(|(bytes, _)| !matches!(bytes.as_slice(), b"\n" | b"\r\n"));
    if last_has_content && rng.gen_bool(0.3) {
        if let Some((last, _)) = new_lines.last_mut() {
            while matches!(last.last(), Some(b'\n' | b'\r')) {
                last.pop();
            }
            if let Some((line, _)) = touched.iter_mut().rev().find(|(line, _)| {
                line.kind == TouchedLineKind::Added && line.line_number == new_lines.len() as u64
            }) {
                *line = touched_line(
                    TouchedLineKind::Added,
                    line.line_number,
                    &new_lines.last().expect("last line").0,
                );
            }
        }
    }

    let hunk = crate::services::patch::PatchHunk {
        old_start: 1,
        old_count: old_line - 1,
        new_start: 1,
        new_count: new_lines.len() as u64,
        model_id: None,
        lines: Vec::new(),
    };
    let file_path = format!("src/random_{index}.rs");
    let mut patch = PatchFileChange {
        old_path: file_path.clone(),
        new_path: file_path.clone(),
        kind: FileChangeKind::Modified,
        hunks: vec![hunk],
    };
    patch.hunks[0].lines = touched.iter().map(|(line, _)| line.clone()).collect();
    RandomFile {
        path: file_path,
        new_lines,
        patch,
        decisions: touched.into_iter().map(|(_, decision)| decision).collect(),
    }
}

impl RandomFile {
    fn bytes(&self) -> Vec<u8> {
        self.new_lines
            .iter()
            .flat_map(|(bytes, _)| bytes.clone())
            .collect()
    }
}

fn partition(
    files: &[RandomFile],
    max_hunk_len: usize,
) -> (ParsedPatch, BTreeMap<PatchLineLocation, LineAttribution>) {
    let mut decisions = BTreeMap::new();
    let patch_files = files
        .iter()
        .enumerate()
        .map(|(file_index, file)| {
            let whole = &file.patch.hunks[0];
            let hunks = whole
                .lines
                .chunks(max_hunk_len.max(1))
                .zip(file.decisions.chunks(max_hunk_len.max(1)))
                .enumerate()
                .map(|(hunk_index, (lines, hunk_decisions))| {
                    for (line_index, decision) in hunk_decisions.iter().enumerate() {
                        decisions.insert(
                            PatchLineLocation {
                                file_index,
                                hunk_index,
                                line_index,
                            },
                            decision.clone(),
                        );
                    }
                    crate::services::patch::PatchHunk {
                        lines: lines.to_vec(),
                        ..whole.clone()
                    }
                })
                .collect();
            crate::services::patch::PatchFileChange {
                hunks,
                ..file.patch.clone()
            }
        })
        .collect();
    (ParsedPatch { files: patch_files }, decisions)
}

type ExpectedKey = (Option<String>, Option<String>, &'static str);

fn expected_key(decision: &LineAttribution) -> ExpectedKey {
    match decision {
        LineAttribution::Ai(line) => (
            line.provenance.session_id.clone(),
            line.provenance.model_id.clone(),
            "ai",
        ),
        LineAttribution::NonExclusive => (None, None, "non_exclusive"),
        LineAttribution::Unknown => (None, None, "unknown"),
    }
}

fn expected_ranges(file: &RandomFile) -> Vec<(u64, u64, ExpectedKey)> {
    let mut ranges: Vec<(u64, u64, ExpectedKey)> = Vec::new();
    for (index, (_, decision)) in file.new_lines.iter().enumerate() {
        let line_number = index as u64 + 1;
        let Some(decision) = decision else { continue };
        let key = expected_key(decision);
        match ranges.last_mut() {
            Some((_, end, last)) if *end + 1 == line_number && *last == key => *end = line_number,
            _ => ranges.push((line_number, line_number, key)),
        }
    }
    ranges
}

struct RandomFiles(BTreeMap<String, Vec<u8>>);

impl CommittedFileReader for RandomFiles {
    fn read_committed_file(&self, path: &str) -> CommittedFileRead {
        Ok(self.0.get(path).map_or(CommittedFile::Missing, |bytes| {
            CommittedFile::Blob(bytes.clone())
        }))
    }
}

fn assert_random_trace_file(seed: u64, trace_file: &super::TraceFile, file: &RandomFile) {
    assert_eq!(trace_file.path, file.path);
    let mut actual = Vec::new();
    for conversation in &trace_file.conversations {
        let mut conversation_keys = BTreeSet::new();
        for range in &conversation.ranges {
            assert!(1 <= range.start_line && range.start_line <= range.end_line);
            let start = usize::try_from(range.start_line).expect("start fits") - 1;
            let end = usize::try_from(range.end_line).expect("end fits");
            assert!(end <= file.new_lines.len());
            let lines = &file.new_lines[start..end];
            let first = lines[0].1.as_ref().map(expected_key);
            assert!(
                lines.iter().all(|(_, decision)| {
                    decision.is_some() && decision.as_ref().map(expected_key) == first
                }),
                "seed {seed}: range crosses context or an attribution boundary"
            );
            let key = first.expect("added line");
            let bytes: Vec<u8> = lines.iter().flat_map(|(bytes, _)| bytes.clone()).collect();
            assert_eq!(
                range.content_hash,
                Some(expected_hash(&bytes)),
                "seed {seed}"
            );

            let contributor = range
                .contributor
                .as_ref()
                .or(conversation.contributor.as_ref())
                .expect("effective contributor");
            let expected_kind = if key.2 == "ai" {
                ContributorType::Ai
            } else {
                ContributorType::Unknown
            };
            assert_eq!(contributor.kind, expected_kind, "seed {seed}");
            let expected_model = if key.2 == "ai" { key.1.clone() } else { None };
            assert_eq!(contributor.model_id, expected_model, "seed {seed}");
            conversation_keys.insert(match key.2 {
                "ai" if key.0.is_some() => (key.0.clone(), None, "session"),
                "ai" => (None, key.1.clone(), "sessionless"),
                other => (None, None, other),
            });
            actual.push((range.start_line, range.end_line, key));
        }
        assert_eq!(
            conversation_keys.len(),
            1,
            "seed {seed}: conversation provenance"
        );
        let (session, _, _) = conversation_keys.into_iter().next().expect("key");
        assert_eq!(
            related_urls(conversation),
            session
                .map(|session| vec![agent_trace_session_url(&session)])
                .unwrap_or_default(),
            "seed {seed}: related sessions"
        );
    }
    actual.sort_by_key(|(start, _, _)| *start);
    assert_eq!(actual, expected_ranges(file), "seed {seed}");
}

#[test]
fn randomized_ranges_reference_exact_committed_content_with_defensible_provenance() {
    for seed in 0..RANDOMIZED_SEEDS {
        let mut rng = StdRng::seed_from_u64(seed);
        let files: Vec<RandomFile> = (0..rng.gen_range(1..=3))
            .map(|index| random_file(&mut rng, index))
            .collect();
        let blobs = RandomFiles(
            files
                .iter()
                .map(|file| (file.path.clone(), file.bytes()))
                .collect(),
        );
        let url = agent_trace_conversation_url("00000000-0000-7000-8000-000000000000");

        let (patch, decisions) = partition(&files, usize::MAX);
        let trace_files =
            super::build_trace_files(&patch, &decisions, &url, &blobs, TEST_COMMIT_REVISION)
                .expect("random trace files build");

        for max_hunk_len in [1, 2, 5] {
            let (repartitioned, repartitioned_decisions) = partition(&files, max_hunk_len);
            assert_eq!(
                line_change_attribution(&repartitioned, &repartitioned_decisions),
                line_change_attribution(&patch, &decisions),
                "seed {seed} counters hunk partition {max_hunk_len}"
            );
            assert_eq!(
                super::build_trace_files(
                    &repartitioned,
                    &repartitioned_decisions,
                    &url,
                    &blobs,
                    TEST_COMMIT_REVISION
                )
                .expect("repartitioned trace files build"),
                trace_files,
                "seed {seed} hunk partition {max_hunk_len}"
            );
        }

        let record = serde_json::json!({
            "version": AGENT_TRACE_VERSION,
            "id": "00000000-0000-7000-8000-000000000000",
            "timestamp": TEST_COMMIT_TIMESTAMP,
            "files": trace_files,
        });
        validate_agent_trace_value(&record).expect("random trace validates");
        let json = record.to_string();
        assert!(!json.contains("\"human\"") && !json.contains("\"mixed\""));

        let expected_files: Vec<&RandomFile> = files
            .iter()
            .filter(|file| {
                file.new_lines
                    .iter()
                    .any(|(_, decision)| decision.is_some())
            })
            .collect();
        assert_eq!(trace_files.len(), expected_files.len(), "seed {seed}");
        for (trace_file, file) in trace_files.iter().zip(expected_files) {
            assert_random_trace_file(seed, trace_file, file);
        }
    }
}
