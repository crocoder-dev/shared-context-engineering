use anyhow::{bail, Result};
use inquire::{Confirm, InquireError, MultiSelect, Select};

use crate::services::style::{
    prompt_label, prompt_label_with_color_policy, prompt_value_with_color_policy,
};

use super::{OptionalWorkflow, SetupDispatch, SetupMode, SetupPromptTarget, SetupTarget};

fn proceed(target: SetupTarget) -> SetupDispatch {
    SetupDispatch::Proceed {
        mode: SetupMode::NonInteractive(target),
        optional_workflows: None,
        agent_trace_auto_sync: None,
        attribution_hooks_enabled: None,
    }
}

pub(super) fn prompt_target() -> Result<SetupDispatch> {
    let options = vec![
        SetupPromptTarget::OpenCode,
        SetupPromptTarget::Claude,
        SetupPromptTarget::Pi,
        SetupPromptTarget::Codex,
        SetupPromptTarget::All,
    ];

    let selection = Select::new(&setup_prompt_title(), options).prompt();

    match selection {
        Ok(SetupPromptTarget::OpenCode) => Ok(proceed(SetupTarget::OpenCode)),
        Ok(SetupPromptTarget::Claude) => Ok(proceed(SetupTarget::Claude)),
        Ok(SetupPromptTarget::Pi) => Ok(proceed(SetupTarget::Pi)),
        Ok(SetupPromptTarget::Codex) => Ok(proceed(SetupTarget::Codex)),
        Ok(SetupPromptTarget::All) => Ok(proceed(SetupTarget::All)),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            Ok(SetupDispatch::Cancelled)
        }
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

pub(super) fn prompt_agent_trace_auto_sync() -> Result<Option<bool>> {
    prompt_confirmation(
        "Automatically sync Agent Traces?\n(Requires an SCE account. Sends Agent Traces from supported AI coding tools\nto SCE servers so they can be stored and viewed in your account.)",
    )
}

pub(super) fn prompt_attribution_hooks_enabled() -> Result<Option<bool>> {
    prompt_confirmation(
        "Record SCE involvement in Git commits?\n(Adds SCE metadata to commits created or assisted by SCE.)",
    )
}

fn prompt_confirmation(label: &str) -> Result<Option<bool>> {
    match Confirm::new(label).with_default(true).prompt() {
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
    prompt_label("Select setup target")
}

pub(super) fn setup_prompt_target_label(target: SetupPromptTarget) -> String {
    setup_prompt_target_label_with_color_policy(target, crate::services::style::supports_color())
}

pub(super) fn setup_prompt_target_label_with_color_policy(
    target: SetupPromptTarget,
    color_enabled: bool,
) -> String {
    let label = match target {
        SetupPromptTarget::OpenCode => "OpenCode",
        SetupPromptTarget::Claude => "Claude",
        SetupPromptTarget::Pi => "Pi",
        SetupPromptTarget::Codex => "Codex",
        SetupPromptTarget::All => "All (OpenCode + Claude + Pi + Codex)",
    };

    prompt_value_with_color_policy(label, color_enabled)
}

#[allow(dead_code)]
pub(super) fn setup_prompt_title_with_color_policy(color_enabled: bool) -> String {
    prompt_label_with_color_policy("Select setup target", color_enabled)
}
