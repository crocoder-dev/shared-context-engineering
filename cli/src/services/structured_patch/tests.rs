use std::fs;

use serde_json::Value;

use super::{derive_claude_structured_patch, ClaudeStructuredPatchDerivationResult};
use crate::services::patch::parse_patch;

const FIXED_TIME: u64 = 1_700_000_000_000;
const FIXED_TOOL_VERSION: &str = "test-claude-version";
const EXPECTED_SCENARIOS: &[&str] = &[
    "write_create_simple",
    "write_create_empty",
    "write_create_no_newline",
    "write_create_multiline",
    "edit_single_hunk",
    "edit_multi_hunk",
    "edit_only_additions",
    "edit_only_deletions",
];

fn fixture_root() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("src/services/structured_patch/fixtures")
}

fn discover_fixture_scenarios() -> Vec<String> {
    let root = fixture_root();
    let mut scenarios = Vec::new();
    for entry in fs::read_dir(&root).expect("fixture root should exist") {
        let entry = entry.expect("fixture entry should be readable");
        if entry
            .file_type()
            .expect("file type should be readable")
            .is_dir()
        {
            scenarios.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    scenarios.sort();
    scenarios
}

fn ordered_fixture_scenarios() -> Vec<String> {
    let discovered = discover_fixture_scenarios();

    let missing: Vec<&str> = EXPECTED_SCENARIOS
        .iter()
        .copied()
        .filter(|name| !discovered.contains(&name.to_string()))
        .collect();
    let extra: Vec<&str> = discovered
        .iter()
        .map(String::as_str)
        .filter(|name| !EXPECTED_SCENARIOS.contains(name))
        .collect();

    assert!(
        missing.is_empty() && extra.is_empty(),
        "Unexpected Claude diff-creation fixtures. Missing: {}. Extra: {}.",
        missing.join(", "),
        extra.join(", ")
    );

    EXPECTED_SCENARIOS
        .iter()
        .copied()
        .filter(|name| discovered.contains(&name.to_string()))
        .map(ToString::to_string)
        .collect()
}

fn load_fixture(name: &str) -> (Value, String) {
    let dir = fixture_root().join(name);
    let input_json = fs::read_to_string(dir.join("claude-post-tool-use.json"))
        .expect("fixture input should exist");
    let expected_patch = fs::read_to_string(dir.join("expected.patch"))
        .expect("fixture expected patch should exist");

    let input: Value =
        serde_json::from_str(&input_json).expect("fixture input should be valid JSON");

    assert!(
        input.get("session_id").and_then(|v| v.as_str()).is_some(),
        "{name} fixture is missing a string session_id"
    );

    (input, expected_patch)
}

#[test]
fn claude_derivation_golden_tests() {
    for name in ordered_fixture_scenarios() {
        let (input, expected_patch_text) = load_fixture(&name);

        let result = derive_claude_structured_patch(
            "PostToolUse",
            &input,
            FIXED_TIME,
            Some(FIXED_TOOL_VERSION),
        );

        match result {
            ClaudeStructuredPatchDerivationResult::Derived(patch) => {
                let session_id = input
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .expect("session_id should be validated by load_fixture");
                assert_eq!(
                    patch.session_id, session_id,
                    "session_id mismatch for scenario {name}"
                );
                assert_eq!(patch.time, FIXED_TIME, "time mismatch for scenario {name}");
                assert_eq!(
                    patch.tool_name, "claude",
                    "tool_name mismatch for scenario {name}"
                );
                assert_eq!(
                    patch.tool_version,
                    Some(FIXED_TOOL_VERSION.to_string()),
                    "tool_version mismatch for scenario {name}"
                );

                let expected_patch = parse_patch(&expected_patch_text, None).unwrap_or_else(|e| {
                    panic!("expected patch should parse for scenario {name}: {e}")
                });

                assert_eq!(
                    patch.patch, expected_patch,
                    "patch mismatch for scenario {name}"
                );

                // model_id is omitted at the structured-patch layer; hunks should not carry it.
                let all_hunks_empty_model_id = patch
                    .patch
                    .files
                    .iter()
                    .all(|file| file.hunks.iter().all(|hunk| hunk.model_id.is_none()));
                assert!(
                    all_hunks_empty_model_id,
                    "model_id should be omitted for scenario {name}"
                );
            }
            ClaudeStructuredPatchDerivationResult::Skipped(reason) => {
                panic!(
                    "Expected {name} fixture to derive a diff trace, but got skipped: {reason:?}"
                );
            }
        }
    }
}
