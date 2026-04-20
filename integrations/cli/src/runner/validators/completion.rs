use super::super::catalog::{
    COMPLETION_BASH_REQUIRED_MARKERS, COMPLETION_FISH_REQUIRED_MARKERS,
    COMPLETION_ZSH_REQUIRED_MARKERS,
};
use super::shared::{assert_non_empty_payload, assert_required_substrings};

pub(super) fn validate_completion_bash_output(stream: &str) -> Result<(), String> {
    assert_non_empty_payload(stream, "completion bash")?;
    assert_required_substrings(stream, COMPLETION_BASH_REQUIRED_MARKERS, "completion bash")
}

pub(super) fn validate_completion_zsh_output(stream: &str) -> Result<(), String> {
    assert_non_empty_payload(stream, "completion zsh")?;
    assert_required_substrings(stream, COMPLETION_ZSH_REQUIRED_MARKERS, "completion zsh")
}

pub(super) fn validate_completion_fish_output(stream: &str) -> Result<(), String> {
    assert_non_empty_payload(stream, "completion fish")?;
    assert_required_substrings(stream, COMPLETION_FISH_REQUIRED_MARKERS, "completion fish")
}
