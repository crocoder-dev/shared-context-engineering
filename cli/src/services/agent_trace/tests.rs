use std::fmt::Write as _;
use std::path::Path;

use super::{
    build_agent_trace_from_evidence, validate_agent_trace_value, AgentTraceEvidence,
    AgentTraceMetadataInput, AgentTraceVcsType, DirectEvidenceSource, RepositoryRoot,
    AGENT_TRACE_VERSION,
};
use crate::services::{
    agent_trace::agent_trace_conversation_url,
    agent_trace_db::DiffTracePathOrigin,
    patch::{parse_patch, ParsedPatch},
};
use serde_json::Value;

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
    let direct_sources = direct_sources(sources);
    let trace = build_agent_trace_from_evidence(
        AgentTraceEvidence {
            repository_root: &test_root(),
            direct_sources: &direct_sources,
            mutation_ai_patch,
            mutation_non_ai_patch: &ParsedPatch { files: Vec::new() },
        },
        &committed_patch(),
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
