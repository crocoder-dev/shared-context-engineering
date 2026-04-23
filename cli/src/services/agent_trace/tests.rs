use super::{
    build_agent_trace, AgentTrace, AgentTraceMetadataInput, Contributor, Conversation,
    HunkContributor, LineRange, AGENT_TRACE_VERSION,
};
use crate::services::patch::{combine_patches, parse_patch, ParsedPatch};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Copy)]
struct AgentTraceScenario {
    incremental: &'static [&'static str],
    post_commit: &'static str,
    golden: &'static str,
}

const GOLDEN_TEST_ID: &str = "01962f15-2d3d-7c85-9f6b-0a8b4f6b2fd1";
const GOLDEN_TEST_TIMESTAMP: &str = "2026-04-23T10:20:30Z";
const TEST_COMMIT_TIMESTAMP: &str = "2026-04-23T10:20:30Z";

fn test_metadata_input() -> AgentTraceMetadataInput<'static> {
    AgentTraceMetadataInput {
        commit_timestamp: TEST_COMMIT_TIMESTAMP,
    }
}

fn parse_fixtures(fixtures: &[&str]) -> Vec<ParsedPatch> {
    fixtures
        .iter()
        .map(|fixture| parse_patch(fixture).expect("fixture patch should parse"))
        .collect()
}

fn normalize_dynamic_metadata(mut payload: Value) -> Value {
    payload["id"] = serde_json::json!(GOLDEN_TEST_ID);
    payload["timestamp"] = serde_json::json!(GOLDEN_TEST_TIMESTAMP);
    payload
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
    let post_commit_patch = parse_patch(scenario.post_commit).expect("fixture patch should parse");
    let golden: Value = serde_json::from_str(scenario.golden).expect("golden json should load");

    let actual = build_agent_trace(
        &constructed_patch,
        &post_commit_patch,
        test_metadata_input(),
    )
    .expect("agent trace should build");

    assert_eq!(actual.version, AGENT_TRACE_VERSION);
    assert_eq!(actual.timestamp, TEST_COMMIT_TIMESTAMP);
    let uuid = Uuid::parse_str(&actual.id).expect("agent trace id should be a UUID");
    assert_eq!(uuid.get_version_num(), 7, "agent trace id should be UUIDv7");

    let actual_json = serde_json::to_value(&actual).expect("agent trace should serialize");
    let normalized_actual = normalize_dynamic_metadata(actual_json);

    assert_eq!(normalized_actual, golden);
}

#[test]
fn conversation_serializes_nested_contributor_and_ranges_shape() {
    let conversation = Conversation {
        contributor: Contributor {
            kind: HunkContributor::Ai,
        },
        ranges: vec![LineRange {
            start_line: 3,
            end_line: 7,
        }],
    };

    let serialized = serde_json::to_value(&conversation).expect("conversation should serialize");

    assert_eq!(
        serialized,
        serde_json::json!({
            "contributor": { "type": "ai" },
            "ranges": [
                {
                    "start_line": 3,
                    "end_line": 7
                }
            ]
        })
    );
}

#[test]
fn agent_trace_serializes_top_level_version_id_timestamp_and_files() {
    let trace = AgentTrace {
        version: AGENT_TRACE_VERSION.to_owned(),
        id: "2c3de67f-c4f3-4f7b-a74c-42f0f9db21f1".to_owned(),
        timestamp: "2026-04-23T10:20:30Z".to_owned(),
        files: vec![],
    };

    let serialized = serde_json::to_value(&trace).expect("agent trace should serialize");

    assert_eq!(serialized["version"], serde_json::json!("v0.1.0"));
    assert_eq!(
        serialized["id"],
        serde_json::json!("2c3de67f-c4f3-4f7b-a74c-42f0f9db21f1")
    );
    assert_eq!(
        serialized["timestamp"],
        serde_json::json!("2026-04-23T10:20:30Z")
    );
    assert_eq!(serialized["files"], serde_json::json!([]));
}

#[test]
fn build_agent_trace_generates_uuidv7_id_and_rfc3339_timestamp() {
    let constructed_patch = combine_patches(&parse_fixtures(&[include_str!(
        "fixtures/hello_world_reconstruction/incremental_01.patch"
    )]));
    let post_commit_patch = parse_patch(include_str!(
        "fixtures/hello_world_reconstruction/post_commit.patch"
    ))
    .expect("fixture patch should parse");

    let agent_trace = build_agent_trace(
        &constructed_patch,
        &post_commit_patch,
        test_metadata_input(),
    )
    .expect("agent trace should build");

    let uuid = Uuid::parse_str(&agent_trace.id).expect("id should be UUID formatted");
    assert_eq!(uuid.get_version_num(), 7, "id should be UUIDv7");
    assert_eq!(agent_trace.timestamp, TEST_COMMIT_TIMESTAMP);
}

#[test]
fn build_agent_trace_uses_provided_commit_timestamp() {
    let constructed_patch = combine_patches(&parse_fixtures(&[include_str!(
        "fixtures/hello_world_reconstruction/incremental_01.patch"
    )]));
    let post_commit_patch = parse_patch(include_str!(
        "fixtures/hello_world_reconstruction/post_commit.patch"
    ))
    .expect("fixture patch should parse");

    let commit_timestamp = "2024-12-31T23:59:59+00:00";
    let agent_trace = build_agent_trace(
        &constructed_patch,
        &post_commit_patch,
        AgentTraceMetadataInput { commit_timestamp },
    )
    .expect("agent trace should build");

    assert_eq!(agent_trace.timestamp, commit_timestamp);
}

#[test]
fn build_agent_trace_rejects_non_rfc3339_commit_timestamp() {
    let constructed_patch = combine_patches(&parse_fixtures(&[include_str!(
        "fixtures/hello_world_reconstruction/incremental_01.patch"
    )]));
    let post_commit_patch = parse_patch(include_str!(
        "fixtures/hello_world_reconstruction/post_commit.patch"
    ))
    .expect("fixture patch should parse");

    let error = build_agent_trace(
        &constructed_patch,
        &post_commit_patch,
        AgentTraceMetadataInput {
            commit_timestamp: "not-a-timestamp",
        },
    )
    .expect_err("invalid commit timestamp should fail");

    assert!(
        error.to_string().contains("expected RFC 3339 date-time"),
        "error should mention RFC 3339 requirement"
    );
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
    let constructed_patch = combine_patches(&parse_fixtures(&[
        include_str!("fixtures/poem_edit_reconstruction/incremental_01.patch"),
        include_str!("fixtures/poem_edit_reconstruction/incremental_02.patch"),
    ]));
    let post_commit_patch = parse_patch(include_str!(
        "fixtures/poem_edit_reconstruction/post_commit.patch"
    ))
    .expect("fixture patch should parse");

    let agent_trace = build_agent_trace(
        &constructed_patch,
        &post_commit_patch,
        test_metadata_input(),
    )
    .expect("agent trace should build");

    assert_eq!(agent_trace.files.len(), 1);
    assert_eq!(agent_trace.files[0].path, "poem.md");
    assert_eq!(agent_trace.files[0].conversations.len(), 3);
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
            }][..],
            &[LineRange {
                start_line: 10,
                end_line: 16,
            }][..],
            &[LineRange {
                start_line: 21,
                end_line: 24,
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
