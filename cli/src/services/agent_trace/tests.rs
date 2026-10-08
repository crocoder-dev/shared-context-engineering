use super::{
    build_agent_trace_from_evidence, validate_agent_trace_value, AgentTraceEvidence,
    AgentTraceMetadataInput, AgentTraceVcsType, AGENT_TRACE_VERSION,
};
use crate::services::{agent_trace::agent_trace_conversation_url, patch::parse_patch};
use serde_json::Value;

const TEST_COMMIT_TIMESTAMP: &str = "2026-04-23T10:20:30Z";
const TEST_COMMIT_REVISION: &str = "a0b1c2d3e4f5a6b7c8d9e0f11223344556677889";

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

    let actual = build_agent_trace_from_evidence(
        AgentTraceEvidence {
            direct_patch: &direct_patch,
            mutation_ai_patch: &mutation_ai_patch,
        },
        &post_commit_patch,
        AgentTraceMetadataInput {
            commit_timestamp: TEST_COMMIT_TIMESTAMP,
            commit_revision: TEST_COMMIT_REVISION,
            vcs_type: Some(AgentTraceVcsType::Git),
            tool_name: Some(EVIDENCE_TOOL_NAME),
            tool_version: Some(EVIDENCE_TOOL_VERSION),
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
