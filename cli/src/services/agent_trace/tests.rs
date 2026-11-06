use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use rand::{rngs::StdRng, Rng, SeedableRng};
use serde_json::Value;

use super::{
    build_agent_trace_from_evidence, line_change_attribution, resolve_line, resolve_patch_lines,
    validate_agent_trace_value, AcceptedDirectLine, AgentTraceEvidence, AgentTraceMetadataInput,
    AgentTraceVcsType, DirectEvidenceSource, DirectLineEvidence, HunkContributor, LineAttribution,
    LineChangeAttribution, LineChangeCounts, MutationEvidenceIndex, MutationLineEvidence,
    RepositoryRoot, ResolvedLine, AGENT_TRACE_VERSION,
};
use crate::services::{
    agent_trace::agent_trace_conversation_url,
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

fn contributor_kinds(trace: &super::AgentTrace) -> Vec<super::HunkContributor> {
    trace
        .files
        .iter()
        .flat_map(|file| file.conversations.iter())
        .map(|conversation| conversation.contributor.kind)
        .collect()
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
        contributor_kinds(&trace),
        vec![
            super::HunkContributor::Unknown,
            super::HunkContributor::Unknown
        ]
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
    assert_eq!(conversation.contributor.kind, super::HunkContributor::Ai);
    assert_eq!(conversation.contributor.model_id.as_deref(), Some(MODEL_A));
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
        contributor_kinds(&trace),
        vec![super::HunkContributor::Unknown, super::HunkContributor::Ai]
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
        contributor_kinds(&trace),
        vec![
            super::HunkContributor::Unknown,
            super::HunkContributor::Unknown
        ]
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
        contributor_kinds(&trace),
        vec![super::HunkContributor::Ai, super::HunkContributor::Unknown]
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
        contributor_kinds(&trace),
        vec![super::HunkContributor::Ai, super::HunkContributor::Ai]
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
        contributor_kinds(&trace),
        vec![super::HunkContributor::Ai, super::HunkContributor::Ai]
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
        contributor_kinds(&trace),
        vec![super::HunkContributor::Ai, super::HunkContributor::Ai]
    );
    assert_eq!(
        trace.files[0].conversations[0]
            .contributor
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
        contributor_kinds(&trace),
        vec![super::HunkContributor::Ai, super::HunkContributor::Ai]
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
        contributor_kinds(&trace),
        vec![super::HunkContributor::Unknown, super::HunkContributor::Ai]
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
            contributor_kinds(&trace),
            vec![super::HunkContributor::Unknown, super::HunkContributor::Ai],
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
        contributor_kinds(&trace),
        vec![super::HunkContributor::Ai, super::HunkContributor::Unknown]
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
        contributor_kinds(&trace),
        vec![super::HunkContributor::Ai, super::HunkContributor::Unknown]
    );
    assert_eq!(
        trace.files[0].conversations[0]
            .contributor
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
    let trace = build_trace(
        &committed,
        &[line_source(MIXED_PATH, MIXED_DIRECT, SESSION_A, MODEL_A)],
        &mutation_ai_lines(MIXED_PATH, MIXED_LINEAGE, SESSION_A, Some(MODEL_A)),
        &mutation_non_ai_lines(MIXED_PATH, MIXED_CONTENDED),
    );

    assert_eq!(buckets(&trace), ((2, 1), (1, 0), (1, 1)));
    assert_counters_cover(&trace, &committed);
    assert_eq!(contributor_kinds(&trace), vec![HunkContributor::Mixed]);
    assert_eq!(
        trace.files[0].conversations[0]
            .contributor
            .model_id
            .as_deref(),
        Some(MODEL_A)
    );
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
    assert_eq!(contributor_kinds(&trace), vec![HunkContributor::Ai]);
}

#[test]
fn entirely_unknown_hunk_counts_every_line_as_unknown() {
    let committed = mixed_committed();
    let trace = build_trace(&committed, &[], &no_mutation(), &no_mutation());

    assert_eq!(buckets(&trace), ((0, 0), (0, 0), (4, 2)));
    assert_counters_cover(&trace, &committed);
    assert_eq!(contributor_kinds(&trace), vec![HunkContributor::Unknown]);
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
        contributor_kinds(&trace),
        vec![HunkContributor::Unknown, HunkContributor::Unknown]
    );
    let conversation = &trace.files[0].conversations[0];
    assert_eq!(conversation.contributor.model_id, None);
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
    assert_eq!(conversation.contributor.kind, HunkContributor::Ai);
    assert_eq!(conversation.contributor.model_id, None);
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
    assert_eq!(contributor_kinds(&trace), vec![HunkContributor::Unknown]);
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
        contributor_kinds(&trace),
        vec![HunkContributor::Mixed, HunkContributor::Ai]
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
        contributor_kinds(&trace),
        vec![
            HunkContributor::Mixed,
            HunkContributor::Ai,
            HunkContributor::Mixed
        ]
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

    assert_eq!(trace.files.len(), 1);
    assert_eq!(trace.files[0].path, SOURCE_PATH);
    assert_eq!(contributor_kinds(&trace), vec![HunkContributor::Ai]);
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
