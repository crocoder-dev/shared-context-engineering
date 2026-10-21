use anyhow::{bail, Result};
use inquire::list_option::ListOption;
use inquire::validator::Validation;
use inquire::{Confirm, InquireError, MultiSelect, Select};

use crate::services::config::AuthMode;
use crate::services::style::{prompt_label, prompt_value_with_color_policy};

use super::{ConcreteSetupTarget, OptionalWorkflow, SetupPromptTarget};

const SETUP_PROMPT_TARGETS: [ConcreteSetupTarget; 4] = [
    ConcreteSetupTarget::OpenCode,
    ConcreteSetupTarget::Claude,
    ConcreteSetupTarget::Pi,
    ConcreteSetupTarget::Codex,
];

const SETUP_TARGET_REQUIRED_MESSAGE: &str = "Select at least one setup target.";

pub(super) fn prompt_targets(
    defaults: &[ConcreteSetupTarget],
) -> Result<Option<Vec<ConcreteSetupTarget>>> {
    let options: Vec<SetupPromptTarget> = SETUP_PROMPT_TARGETS
        .iter()
        .map(|target| SetupPromptTarget(*target))
        .collect();
    let default_indices: Vec<usize> = SETUP_PROMPT_TARGETS
        .iter()
        .enumerate()
        .filter(|(_, target)| defaults.contains(target))
        .map(|(index, _)| index)
        .collect();

    let selection = MultiSelect::new(&setup_prompt_title(), options)
        .with_default(&default_indices)
        .with_validator(|selected: &[ListOption<&SetupPromptTarget>]| {
            if selected.is_empty() {
                Ok(Validation::Invalid(SETUP_TARGET_REQUIRED_MESSAGE.into()))
            } else {
                Ok(Validation::Valid)
            }
        })
        .prompt();

    match selection {
        Ok(selected) => Ok(Some(selected.into_iter().map(|row| row.0).collect())),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Ok(None),
        Err(InquireError::NotTTY) => bail!(
            "Interactive setup requires a TTY. Re-run with '--non-interactive' and one of '--opencode', '--claude', '--pi', '--codex', or '--all'."
        ),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn prompt_optional_workflows(defaults: &[String]) -> Result<Option<Vec<String>>> {
    let Some((rows, default_indices)) =
        optional_workflow_prompt_inputs(super::OPTIONAL_WORKFLOWS, defaults)
    else {
        return Ok(Some(Vec::new()));
    };

    let selection = MultiSelect::new(&optional_workflow_prompt_title(), rows)
        .with_default(&default_indices)
        .prompt();

    match selection {
        Ok(selected) => Ok(Some(
            selected
                .into_iter()
                .map(|row| row.workflow.id.to_string())
                .collect(),
        )),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Ok(None),
        Err(InquireError::NotTTY) => bail!(
            "Interactive setup requires a TTY. Re-run with '--non-interactive' and one of '--opencode', '--claude', '--pi', '--codex', or '--all', adding '--workflow <slug>' for each optional workflow to install."
        ),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn prompt_agent_trace_auto_sync(default: bool) -> Result<Option<bool>> {
    prompt_confirmation(
        "Automatically sync Agent Traces?\n(Requires an SCE account. Sends Agent Traces from supported AI coding tools\nto SCE servers so they can be stored and viewed in your account.)",
        default,
    )
}

pub(super) fn prompt_attribution_hooks_enabled(default: bool) -> Result<Option<bool>> {
    prompt_confirmation(
        "Record SCE involvement in Git commits?\n(Adds SCE metadata to commits created or assisted by SCE.)",
        default,
    )
}

const AUTH_MODE_PROMPT_TITLE: &str = "SCE account mode for this machine";

const AUTH_MODE_PROMPT_ROWS: [AuthMode; 3] =
    [AuthMode::Optional, AuthMode::Mandatory, AuthMode::Disabled];

const AUTH_MODE_OPTIONAL_LABEL: &str =
    "Optional: use SCE locally and log in when you want SCE cloud features";
const AUTH_MODE_MANDATORY_LABEL: &str = "Mandatory: require an SCE login when setting up";
const AUTH_MODE_DISABLED_LABEL: &str =
    "Disabled: local only; no SCE login, cloud sync, or telemetry";

const TELEMETRY_CONSENT_PROMPT_LABEL: &str = "Share SCE telemetry?\n(Sends SCE usage telemetry to SCE servers using your SCE login. If Claude is\nselected, also configures Claude Code's native telemetry export. Separate from\nAgent Trace sync. Applies to all repositories on this machine.)";

const LOGIN_PROMPT_LABEL: &str = "Log in to SCE now?\n(Opens a device login in your browser. An existing login on this machine is\nreused. You can log in later with 'sce auth login'.)";

const LOGIN_PROMPT_DEFAULT: bool = true;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AuthModeRow(pub(super) AuthMode);

impl std::fmt::Display for AuthModeRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", auth_mode_row_label(self.0))
    }
}

pub(super) fn auth_mode_row_label(mode: AuthMode) -> &'static str {
    match mode {
        AuthMode::Optional => AUTH_MODE_OPTIONAL_LABEL,
        AuthMode::Mandatory => AUTH_MODE_MANDATORY_LABEL,
        AuthMode::Disabled => AUTH_MODE_DISABLED_LABEL,
    }
}

pub(super) fn auth_mode_prompt_rows() -> Vec<AuthModeRow> {
    AUTH_MODE_PROMPT_ROWS
        .iter()
        .copied()
        .map(AuthModeRow)
        .collect()
}

pub(super) fn auth_mode_prompt_default_index(default: AuthMode) -> usize {
    AUTH_MODE_PROMPT_ROWS
        .iter()
        .position(|mode| *mode == default)
        .unwrap_or(0)
}

pub(super) fn auth_mode_prompt_title() -> String {
    prompt_label(AUTH_MODE_PROMPT_TITLE)
}

pub(super) fn prompt_auth_mode(default: AuthMode) -> Result<Option<AuthMode>> {
    let selection = Select::new(&auth_mode_prompt_title(), auth_mode_prompt_rows())
        .with_starting_cursor(auth_mode_prompt_default_index(default))
        .prompt();

    interactive_prompt_result(selection.map(|row| row.0))
}

pub(super) fn prompt_telemetry_consent(default: bool) -> Result<Option<bool>> {
    prompt_confirmation(TELEMETRY_CONSENT_PROMPT_LABEL, default)
}

pub(super) fn prompt_login() -> Result<Option<bool>> {
    prompt_confirmation(LOGIN_PROMPT_LABEL, LOGIN_PROMPT_DEFAULT)
}

fn prompt_confirmation(label: &str, default: bool) -> Result<Option<bool>> {
    interactive_prompt_result(Confirm::new(label).with_default(default).prompt())
}

fn interactive_prompt_result<T>(result: Result<T, InquireError>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Ok(None),
        Err(InquireError::NotTTY) => bail!(
            "Interactive setup requires a TTY. Re-run with '--non-interactive' and one of '--opencode', '--claude', '--pi', '--codex', or '--all'."
        ),
        Err(error) => Err(error.into()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OptionalWorkflowRow {
    pub(super) workflow: &'static OptionalWorkflow,
}

impl std::fmt::Display for OptionalWorkflowRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", optional_workflow_row_label(self.workflow))
    }
}

pub(super) fn optional_workflow_prompt_inputs(
    catalog: &'static [OptionalWorkflow],
    defaults: &[String],
) -> Option<(Vec<OptionalWorkflowRow>, Vec<usize>)> {
    if catalog.is_empty() {
        return None;
    }

    Some((
        optional_workflow_rows(catalog),
        optional_workflow_default_indices(catalog, defaults),
    ))
}

pub(super) fn optional_workflow_rows(
    catalog: &'static [OptionalWorkflow],
) -> Vec<OptionalWorkflowRow> {
    catalog
        .iter()
        .map(|workflow| OptionalWorkflowRow { workflow })
        .collect()
}

pub(super) fn optional_workflow_default_indices(
    catalog: &'static [OptionalWorkflow],
    defaults: &[String],
) -> Vec<usize> {
    catalog
        .iter()
        .enumerate()
        .filter(|(_, workflow)| defaults.iter().any(|id| id == workflow.id))
        .map(|(index, _)| index)
        .collect()
}

pub(super) fn optional_workflow_prompt_title() -> String {
    prompt_label("Select optional workflows")
}

pub(super) fn optional_workflow_row_label(workflow: &OptionalWorkflow) -> String {
    optional_workflow_row_label_with_color_policy(
        workflow,
        crate::services::style::supports_color(),
    )
}

pub(super) fn optional_workflow_row_label_with_color_policy(
    workflow: &OptionalWorkflow,
    color_enabled: bool,
) -> String {
    format!(
        "{} — {}",
        prompt_value_with_color_policy(workflow.title, color_enabled),
        workflow.description
    )
}

pub(super) fn setup_prompt_title() -> String {
    prompt_label("Select setup targets")
}

pub(super) fn setup_prompt_target_label(target: SetupPromptTarget) -> String {
    setup_prompt_target_label_with_color_policy(target, crate::services::style::supports_color())
}

pub(super) fn setup_prompt_target_label_with_color_policy(
    target: SetupPromptTarget,
    color_enabled: bool,
) -> String {
    let label = match target.0 {
        ConcreteSetupTarget::OpenCode => "OpenCode",
        ConcreteSetupTarget::Claude => "Claude",
        ConcreteSetupTarget::Pi => "Pi",
        ConcreteSetupTarget::Codex => "Codex",
    };

    prompt_value_with_color_policy(label, color_enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_mode_rows_map_to_modes_in_fixed_order_with_fixed_labels() {
        let rows = auth_mode_prompt_rows();
        let mapped: Vec<(AuthMode, String)> =
            rows.iter().map(|row| (row.0, row.to_string())).collect();

        assert_eq!(
            mapped,
            vec![
                (
                    AuthMode::Optional,
                    "Optional: use SCE locally and log in when you want SCE cloud features"
                        .to_string()
                ),
                (
                    AuthMode::Mandatory,
                    "Mandatory: require an SCE login when setting up".to_string()
                ),
                (
                    AuthMode::Disabled,
                    "Disabled: local only; no SCE login, cloud sync, or telemetry".to_string()
                ),
            ]
        );
    }

    #[test]
    fn auth_mode_default_cursor_points_at_the_default_row() {
        let rows = auth_mode_prompt_rows();

        for mode in [AuthMode::Optional, AuthMode::Mandatory, AuthMode::Disabled] {
            let index = auth_mode_prompt_default_index(mode);
            assert_eq!(rows[index].0, mode);
        }
        assert_eq!(auth_mode_prompt_default_index(AuthMode::Optional), 0);
    }

    #[test]
    fn new_prompt_labels_and_defaults_are_fixed() {
        assert_eq!(
            TELEMETRY_CONSENT_PROMPT_LABEL,
            "Share SCE telemetry?\n(Sends SCE usage telemetry to SCE servers using your SCE login. If Claude is\nselected, also configures Claude Code's native telemetry export. Separate from\nAgent Trace sync. Applies to all repositories on this machine.)"
        );
        assert_eq!(
            LOGIN_PROMPT_LABEL,
            "Log in to SCE now?\n(Opens a device login in your browser. An existing login on this machine is\nreused. You can log in later with 'sce auth login'.)"
        );
        assert_eq!(AUTH_MODE_PROMPT_TITLE, "SCE account mode for this machine");
    }

    #[test]
    fn interactive_prompt_result_maps_cancel_and_not_tty_like_existing_prompts() {
        assert_eq!(
            interactive_prompt_result(Ok(AuthMode::Mandatory)).unwrap(),
            Some(AuthMode::Mandatory)
        );
        assert_eq!(
            interactive_prompt_result::<bool>(Err(InquireError::OperationCanceled)).unwrap(),
            None
        );
        assert_eq!(
            interactive_prompt_result::<bool>(Err(InquireError::OperationInterrupted)).unwrap(),
            None
        );
        let error = interactive_prompt_result::<bool>(Err(InquireError::NotTTY)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Interactive setup requires a TTY. Re-run with '--non-interactive' and one of '--opencode', '--claude', '--pi', '--codex', or '--all'."
        );
    }
}
