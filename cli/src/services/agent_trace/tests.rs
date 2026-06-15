use super::{
    build_agent_trace, patches_have_overlap, validate_agent_trace_value, AgentTraceMetadataInput,
    AgentTraceVcsType, LineRange, AGENT_TRACE_VERSION,
};
use crate::services::{
    agent_trace::agent_trace_conversation_url,
    patch::{combine_patches, parse_patch, ParsedPatch},
    structured_patch::{derive_claude_structured_patch, ClaudeStructuredPatchDerivationResult},
};
use serde_json::{json, Value};

#[derive(Clone, Copy)]
struct AgentTraceScenario {
    incremental: &'static [&'static str],
    post_commit: &'static str,
    golden: &'static str,
}

const TEST_COMMIT_TIMESTAMP: &str = "2026-04-23T10:20:30Z";
const TEST_COMMIT_REVISION: &str = "a0b1c2d3e4f5a6b7c8d9e0f11223344556677889";

fn parse_fixtures(fixtures: &[&str]) -> Vec<ParsedPatch> {
    fixtures
        .iter()
        .map(|fixture| parse_patch(fixture, None).expect("fixture patch should parse"))
        .collect()
}

fn parse_fixture(fixture: &str) -> ParsedPatch {
    parse_patch(fixture, None).expect("fixture patch should parse")
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
    let constructed_patch = combine_patches(&parse_fixtures(scenario.incremental));
    let post_commit_patch =
        parse_patch(scenario.post_commit, None).expect("fixture patch should parse");
    let golden: Value = serde_json::from_str(scenario.golden).expect("golden json should load");
    validate_agent_trace_value(&golden).expect("golden json should validate against schema");
    let actual = build_agent_trace(
        &constructed_patch,
        &post_commit_patch,
        AgentTraceMetadataInput {
            commit_timestamp: TEST_COMMIT_TIMESTAMP,
            commit_revision: TEST_COMMIT_REVISION,
            vcs_type: Some(AgentTraceVcsType::Git),
            tool_name: None,
            tool_version: None,
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
    assert_eq!(actual_json["vcs"], golden["vcs"]);
    assert_eq!(actual_json["files"], expected_files);
}

#[test]
fn patch_overlap_predicate_detects_matching_touched_lines() {
    let candidate_patch = parse_fixture(include_str!(
        "fixtures/hello_world_reconstruction/incremental_01.patch"
    ));
    let target_patch = parse_fixture(include_str!(
        "fixtures/hello_world_reconstruction/post_commit.patch"
    ));

    assert!(patches_have_overlap(&candidate_patch, &target_patch));
}

#[test]
fn patch_overlap_predicate_rejects_unrelated_touched_lines() {
    let candidate_patch = parse_fixture(include_str!(
        "fixtures/hello_world_reconstruction/incremental_01.patch"
    ));
    let target_patch = parse_fixture(include_str!(
        "fixtures/poem_write_reconstruction/post_commit.patch"
    ));

    assert!(!patches_have_overlap(&candidate_patch, &target_patch));
}

#[test]
fn patch_overlap_predicate_rejects_empty_or_untouched_patches() {
    let candidate_patch = parse_fixture(include_str!(
        "fixtures/hello_world_reconstruction/incremental_01.patch"
    ));
    let untouched_patch = parse_fixture(include_str!(
        "../structured_patch/fixtures/write_create_empty/expected.patch"
    ));
    let empty_patch = parse_fixture("");

    assert!(!patches_have_overlap(&candidate_patch, &untouched_patch));
    assert!(!patches_have_overlap(&untouched_patch, &candidate_patch));
    assert!(!patches_have_overlap(&empty_patch, &candidate_patch));
    assert!(!patches_have_overlap(&candidate_patch, &empty_patch));
}

#[test]
fn patch_overlap_predicate_accepts_claude_structured_patch_derivation() {
    let payload: Value = serde_json::from_str(include_str!(
        "../structured_patch/fixtures/edit_single_hunk/claude-post-tool-use.json"
    ))
    .expect("Claude structured fixture should parse");
    let expected_patch = parse_fixture(include_str!(
        "../structured_patch/fixtures/edit_single_hunk/expected.patch"
    ));
    let derived_patch = match derive_claude_structured_patch("PostToolUse", &payload, 1, None) {
        ClaudeStructuredPatchDerivationResult::Derived(derived) => derived.patch,
        ClaudeStructuredPatchDerivationResult::Skipped(reason) => {
            panic!("Claude structured fixture should derive a patch, got {reason}")
        }
    };

    assert_eq!(derived_patch, expected_patch);
    assert!(patches_have_overlap(&derived_patch, &expected_patch));
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
fn poem_edit_reconstruction_maps_each_hunk_to_one_range() {
    let mut constructed_patch = combine_patches(&parse_fixtures(&[
        include_str!("fixtures/poem_edit_reconstruction/incremental_01.patch"),
        include_str!("fixtures/poem_edit_reconstruction/incremental_02.patch"),
    ]));
    let post_commit_patch = parse_patch(
        include_str!("fixtures/poem_edit_reconstruction/post_commit.patch"),
        None,
    )
    .expect("fixture patch should parse");

    let first_hunk_lines = &mut constructed_patch.files[0].hunks[0].lines;
    first_hunk_lines[0].session_id = Some(String::from("session-z"));
    first_hunk_lines[1].session_id = Some(String::from("session-a"));

    let agent_trace = build_agent_trace(
        &constructed_patch,
        &post_commit_patch,
        AgentTraceMetadataInput {
            commit_timestamp: TEST_COMMIT_TIMESTAMP,
            commit_revision: TEST_COMMIT_REVISION,
            vcs_type: Some(AgentTraceVcsType::Git),
            tool_name: None,
            tool_version: None,
        },
    )
    .expect("agent trace should build");

    let actual_json = serde_json::to_value(&agent_trace).expect("agent trace should serialize");
    validate_agent_trace_value(&actual_json).expect("actual json should validate against schema");

    assert_eq!(agent_trace.files.len(), 1);
    assert_eq!(agent_trace.files[0].path, "poem.md");
    assert_eq!(agent_trace.files[0].conversations.len(), 3);
    assert_eq!(
        agent_trace.files[0].conversations[0].related,
        Some(vec![
            super::ConversationRelated {
                kind: String::from("session"),
                url: String::from("https://sce.crocoder.dev/sessions/session-a"),
            },
            super::ConversationRelated {
                kind: String::from("session"),
                url: String::from("https://sce.crocoder.dev/sessions/session-z"),
            },
        ])
    );
    assert_eq!(agent_trace.files[0].conversations[1].related, None);
    assert_eq!(agent_trace.files[0].conversations[2].related, None);
    assert_eq!(
        actual_json["files"][0]["conversations"][0]["related"],
        json!([
            {
                "type": "session",
                "url": "https://sce.crocoder.dev/sessions/session-a"
            },
            {
                "type": "session",
                "url": "https://sce.crocoder.dev/sessions/session-z"
            }
        ])
    );
    assert!(
        actual_json["files"][0]["conversations"][1]["related"].is_null(),
        "conversations without session-backed lines should omit related"
    );
    assert!(
        actual_json["files"][0]["conversations"][2]["related"].is_null(),
        "conversations without session-backed lines should omit related"
    );
    assert_eq!(
        agent_trace.files[0]
            .conversations
            .iter()
            .map(|conversation| conversation.ranges.as_slice())
            .collect::<Vec<_>>(),
        vec![
            &[LineRange {
                start_line: 1,
                end_line: 8,
                content_hash: "murmur3:25e05a40".to_string(),
            }][..],
            &[LineRange {
                start_line: 10,
                end_line: 16,
                content_hash: "murmur3:bc5d346b".to_string(),
            }][..],
            &[LineRange {
                start_line: 21,
                end_line: 24,
                content_hash: "murmur3:c8621bcb".to_string(),
            }][..],
        ]
    );
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
fn schema_validation_allows_agent_trace_without_vcs() {
    let value = json!({
        "version": AGENT_TRACE_VERSION,
        "id": "0196f25d-cf7f-7ca8-a652-8562c8a9f1d5",
        "timestamp": TEST_COMMIT_TIMESTAMP,
        "files": []
    });

    validate_agent_trace_value(&value)
        .expect("agent trace without vcs should validate against schema");
}

#[test]
fn schema_validation_rejects_vcs_missing_revision() {
    let value = json!({
        "version": AGENT_TRACE_VERSION,
        "id": "0196f25d-cf7f-7ca8-a652-8562c8a9f1d5",
        "timestamp": TEST_COMMIT_TIMESTAMP,
        "vcs": {
            "type": "git"
        },
        "files": []
    });

    let error = validate_agent_trace_value(&value)
        .expect_err("agent trace with vcs missing revision should fail validation");
    let rendered = error.to_string();
    assert!(
        rendered.contains("\"revision\" is a required property"),
        "expected vcs/revision validation failure, got: {rendered}"
    );
}
