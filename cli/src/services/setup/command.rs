use std::future::Future;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::app::ContextWithRepoRoot;
use crate::services::auth_command::{self, AuthFormat, LoginReport};
use crate::services::config::{AuthMode, ResolvedAccessPolicy, ValueSource};
use crate::services::error::{AuthPolicyConfigFile, CliError, UserError};
use crate::services::lifecycle::{
    lifecycle_providers, RequiredHookInstallStatus, RequiredHooksInstallOutcome,
};
use crate::services::setup::access_preferences::{
    persist_global_access_preferences, read_global_access_preferences, GlobalAccessPreferences,
    GlobalAccessPreferencesError, GlobalWriteOutcome,
};
use crate::services::setup::claude_telemetry::CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE;
use crate::services::{config, managed_telemetry, setup};

const SCE_ACCESS_HEADING: &str = "SCE access:";
const SAVED_ACCESS_SCOPE: &str = "applies to all repositories on this machine";
const AUTH_LOGOUT_HINT_LINE: &str =
    "- SCE login: stored SCE credentials remain on this machine; run 'sce auth logout' to remove them.";
const GLOBAL_ACCESS_WRITE_REMEDIATION: &str = "No repository files were changed. Run 'sce config validate' to check the file, then rerun 'sce setup'.";
const GLOBAL_ACCESS_SYMLINK_REMEDIATION: &str =
    "No repository files were changed. Rerun 'sce setup' after updating the config source.";
const SETUP_LOGIN_DISABLED_LINE: &str =
    "SCE login: skipped; SCE authentication is disabled by config.";
const SETUP_LOGIN_NOT_REQUESTED_LINE: &str =
    "SCE login: not requested; run 'sce auth login' to log in or refresh credentials.";
const SETUP_LOGIN_DECLINED_LINE: &str = "SCE login: skipped; run 'sce auth login' to log in.";
const SETUP_LOGIN_FAILED_PREFIX: &str = "SCE login: failed: ";
const SETUP_LOGIN_RETRY_LINE: &str = "Run 'sce auth login' to retry.";
const RECONFIGURE_HINT_LINE: &str =
    "- Run 'sce setup --reconfigure' to change saved setup answers.";

pub struct SetupCommand {
    pub request: setup::SetupRequest,
}

impl SetupCommand {
    pub async fn execute<C: ContextWithRepoRoot>(&self, context: &C) -> Result<String, CliError> {
        let setup_start_path = match &self.request.hooks_repo_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .context("Failed to determine current directory")
                .map_err(unexpected_failure)?,
        };

        let repository_root = resolve_setup_repository(&setup_start_path)?;

        let (setup_dispatch, interactive_access) = match self.prepare_dispatch(&repository_root)? {
            PreparedSetup::Cancelled => return Ok(setup::setup_cancelled_text()),
            PreparedSetup::Ready {
                dispatch,
                interactive_access,
            } => (dispatch, interactive_access),
        };

        let claude_telemetry_selected = dispatch_targets_claude(setup_dispatch.as_ref());
        let authorization_check = RepositoryAuthorizationCheck::for_mode(self.request.config_mode);

        let mut sections = Vec::new();

        let context_message =
            setup::bootstrap_context_baseline(&repository_root).map_err(unexpected_failure)?;
        sections.push(context_message);

        if self.request.context_only {
            return Ok(sections.join("\n\n"));
        }

        let ctx = context.with_repo_root(repository_root.clone());

        let providers = lifecycle_providers(self.request.install_hooks);

        for provider in &providers {
            let outcome = provider.setup(&ctx).await.map_err(unexpected_failure)?;

            sections.extend(outcome.messages);

            if let Some(ref hooks_outcome) = outcome.required_hooks_install {
                sections.push(setup::format_required_hook_install_success_message(
                    &setup_required_hooks_outcome_from_lifecycle(hooks_outcome),
                ));
            }
        }

        if let Some(setup::SetupDispatch::Proceed {
            targets,
            target_persistence,
            optional_workflows: prompted_optional_workflows,
            agent_trace_auto_sync,
            attribution_hooks_enabled,
            login_intent,
            ..
        }) = setup_dispatch
        {
            let optional_workflows = prompted_optional_workflows
                .as_deref()
                .or(self.request.optional_workflows.as_deref());

            let setup_message = setup::run_setup_for_targets(
                &repository_root,
                &targets,
                target_persistence,
                optional_workflows,
                agent_trace_auto_sync,
                attribution_hooks_enabled,
            )
            .map_err(unexpected_failure)?;
            sections.push(setup_message);

            let render_claude_telemetry = async |action| {
                claude_telemetry_report(
                    &repository_root,
                    action,
                    claude_telemetry_selected,
                    authorization_check,
                )
                .await
            };

            let deferred_telemetry_action = if let Some((access_save, auth_db_present)) =
                &interactive_access
            {
                let (access_section, policy) = setup_access_report(
                    &repository_root,
                    access_save,
                    resolve_access_policy,
                    *auth_db_present,
                )?;
                sections.push(access_section);

                let (removal_action, deferred_action) =
                    split_pre_login_removal(claude_telemetry_action(
                        read_global_preferences(&access_save.global_config_path),
                        policy.telemetry_enabled(),
                        claude_telemetry_selected,
                        true,
                    ));
                sections.extend(render_claude_telemetry(removal_action).await);

                let login_status =
                    run_setup_login_step(setup_login_action(&policy, login_intent), || {
                        auth_command::run_login_in(&repository_root, AuthFormat::Text)
                    })
                    .await?;
                sections.push(render_setup_login_section(&login_status));
                deferred_action
            } else {
                non_interactive_claude_telemetry_action(&repository_root, claude_telemetry_selected)
            };

            sections.extend(render_claude_telemetry(deferred_telemetry_action).await);
        }

        Ok(sections.join("\n\n"))
    }

    fn prepare_dispatch(&self, repository_root: &Path) -> Result<PreparedSetup, CliError> {
        if self.request.context_only {
            return Ok(PreparedSetup::Ready {
                dispatch: None,
                interactive_access: None,
            });
        }
        let Some(mode) = self.request.config_mode else {
            return Ok(PreparedSetup::Ready {
                dispatch: None,
                interactive_access: None,
            });
        };

        let optional_workflow_defaults = match &self.request.optional_workflows {
            Some(selection) => selection.clone(),
            None => setup::persisted_optional_workflows(repository_root),
        };

        if mode != setup::SetupMode::Interactive {
            let dispatch = setup::resolve_setup_dispatch(
                mode,
                &setup::InquireSetupTargetPrompter,
                &optional_workflow_defaults,
                &setup::persisted_integration_targets(repository_root),
                setup::persisted_setup_confirmations(repository_root),
                setup::SetupAccessGate::default(),
                false,
            )
            .map_err(unexpected_failure)?;

            return Ok(match dispatch {
                setup::SetupDispatch::Cancelled => PreparedSetup::Cancelled,
                dispatch @ setup::SetupDispatch::Proceed { .. } => PreparedSetup::Ready {
                    dispatch: Some(dispatch),
                    interactive_access: None,
                },
            });
        }

        let global_config_path =
            config::resolver::resolve_default_global_config_path().map_err(unexpected_failure)?;
        let auth_db_present = managed_telemetry::auth_db_present();

        Ok(
            match prepare_interactive_setup(
                repository_root,
                &setup::InquireSetupTargetPrompter,
                &optional_workflow_defaults,
                &global_config_path,
                resolve_access_policy,
                auth_db_present,
                self.request.reconfigure,
            )? {
                InteractiveSetup::Cancelled => PreparedSetup::Cancelled,
                InteractiveSetup::Ready {
                    dispatch,
                    access_save,
                } => PreparedSetup::Ready {
                    dispatch: Some(dispatch),
                    interactive_access: Some((access_save, auth_db_present)),
                },
            },
        )
    }
}

enum PreparedSetup {
    Cancelled,
    Ready {
        dispatch: Option<setup::SetupDispatch>,
        interactive_access: Option<(SetupAccessSave, bool)>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SetupAccessSave {
    global_config_path: PathBuf,
    auth_mode: Option<AuthMode>,
    telemetry_enabled: Option<bool>,
    outcome: GlobalWriteOutcome,
    prompts_suppressed: bool,
}

#[derive(Debug)]
enum InteractiveSetup {
    Cancelled,
    Ready {
        dispatch: setup::SetupDispatch,
        access_save: SetupAccessSave,
    },
}

fn resolve_access_policy(repository_root: &Path) -> anyhow::Result<ResolvedAccessPolicy> {
    config::resolve_auth_runtime_config(repository_root).map(|config| config.access_policy)
}

fn prepare_interactive_setup<P, R>(
    repository_root: &Path,
    prompter: &P,
    optional_workflow_defaults: &[String],
    global_config_path: &Path,
    resolve_policy: R,
    auth_db_present: bool,
    reconfigure: bool,
) -> Result<InteractiveSetup, CliError>
where
    P: setup::SetupTargetPrompter,
    R: Fn(&Path) -> anyhow::Result<ResolvedAccessPolicy>,
{
    let policy = resolve_policy(repository_root).map_err(unexpected_failure)?;
    let global_preferences =
        read_global_access_preferences(global_config_path).map_err(unexpected_failure)?;
    let persisted_confirmations = setup::persisted_setup_confirmations(repository_root)
        .with_global_access_preferences(global_preferences);

    let dispatch = setup::resolve_setup_dispatch(
        setup::SetupMode::Interactive,
        prompter,
        optional_workflow_defaults,
        &setup::persisted_integration_targets(repository_root),
        persisted_confirmations,
        setup::SetupAccessGate::from_policy(&policy, auth_db_present),
        reconfigure,
    )
    .map_err(unexpected_failure)?;

    let (auth_mode, telemetry_enabled, prompts_suppressed) = match &dispatch {
        setup::SetupDispatch::Cancelled => return Ok(InteractiveSetup::Cancelled),
        setup::SetupDispatch::Proceed {
            auth_mode,
            telemetry_enabled,
            prompts_suppressed,
            ..
        } => (*auth_mode, *telemetry_enabled, *prompts_suppressed),
    };

    let access_save = SetupAccessSave {
        prompts_suppressed,
        ..persist_setup_access_preferences(global_config_path, auth_mode, telemetry_enabled)?
    };

    Ok(InteractiveSetup::Ready {
        dispatch,
        access_save,
    })
}

fn persist_setup_access_preferences(
    global_config_path: &Path,
    auth_mode: Option<AuthMode>,
    telemetry_enabled: Option<bool>,
) -> Result<SetupAccessSave, CliError> {
    let outcome =
        persist_global_access_preferences(global_config_path, auth_mode, telemetry_enabled)
            .map_err(|source| global_access_write_failure(global_config_path, &source))?;

    Ok(SetupAccessSave {
        global_config_path: global_config_path.to_path_buf(),
        auth_mode,
        telemetry_enabled,
        outcome,
        prompts_suppressed: false,
    })
}

fn global_access_write_failure(
    global_config_path: &Path,
    source: &GlobalAccessPreferencesError,
) -> CliError {
    let remediation = match source {
        GlobalAccessPreferencesError::SymlinkManaged { .. } => GLOBAL_ACCESS_SYMLINK_REMEDIATION,
        _ => GLOBAL_ACCESS_WRITE_REMEDIATION,
    };
    CliError::runtime(anyhow::anyhow!(
        "Failed to save SCE access preferences to the global SCE config '{}'.\n{source}\n{remediation}",
        global_config_path.display()
    ))
}

fn setup_access_report<R>(
    repository_root: &Path,
    access_save: &SetupAccessSave,
    resolve_policy: R,
    auth_db_present: bool,
) -> Result<(String, ResolvedAccessPolicy), CliError>
where
    R: Fn(&Path) -> anyhow::Result<ResolvedAccessPolicy>,
{
    let policy = resolve_policy(repository_root).map_err(unexpected_failure)?;
    let section = render_setup_access_section(access_save, &policy, auth_db_present);

    Ok((section, policy))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupLoginSkip {
    AuthDisabled,
    NotOffered,
    Declined,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupLoginAction {
    Skip(SetupLoginSkip),
    Run { required: bool },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SetupLoginStatus {
    Skipped(SetupLoginSkip),
    Authenticated(LoginReport),
    Failed(UserError),
}

fn setup_login_action(
    policy: &ResolvedAccessPolicy,
    intent: setup::SetupLoginIntent,
) -> SetupLoginAction {
    if !policy.auth_allowed() {
        return SetupLoginAction::Skip(SetupLoginSkip::AuthDisabled);
    }
    if policy.auth_mode.value == AuthMode::Mandatory {
        return SetupLoginAction::Run { required: true };
    }
    match intent {
        setup::SetupLoginIntent::Requested => SetupLoginAction::Run { required: false },
        setup::SetupLoginIntent::Declined => SetupLoginAction::Skip(SetupLoginSkip::Declined),
        setup::SetupLoginIntent::NotOffered => SetupLoginAction::Skip(SetupLoginSkip::NotOffered),
    }
}

async fn run_setup_login_step<L, LF>(
    action: SetupLoginAction,
    login: L,
) -> Result<SetupLoginStatus, CliError>
where
    L: FnOnce() -> LF,
    LF: Future<Output = Result<LoginReport, CliError>>,
{
    let required = match action {
        SetupLoginAction::Skip(skip) => return Ok(SetupLoginStatus::Skipped(skip)),
        SetupLoginAction::Run { required } => required,
    };

    match login().await {
        Ok(report) => Ok(SetupLoginStatus::Authenticated(report)),
        Err(error) if required => Err(CliError::user_with_source(
            UserError::SetupLoginRequired,
            error,
        )),
        Err(CliError::User { error, .. }) => Ok(SetupLoginStatus::Failed(error)),
        Err(CliError::Internal { .. }) => {
            Ok(SetupLoginStatus::Failed(UserError::UnexpectedFailure))
        }
    }
}

fn render_setup_login_section(status: &SetupLoginStatus) -> String {
    match status {
        SetupLoginStatus::Skipped(SetupLoginSkip::AuthDisabled) => {
            SETUP_LOGIN_DISABLED_LINE.to_string()
        }
        SetupLoginStatus::Skipped(SetupLoginSkip::NotOffered) => {
            SETUP_LOGIN_NOT_REQUESTED_LINE.to_string()
        }
        SetupLoginStatus::Skipped(SetupLoginSkip::Declined) => {
            SETUP_LOGIN_DECLINED_LINE.to_string()
        }
        SetupLoginStatus::Authenticated(report) => report.rendered.clone(),
        SetupLoginStatus::Failed(error) => format!(
            "{SETUP_LOGIN_FAILED_PREFIX}{}\n{SETUP_LOGIN_RETRY_LINE}",
            error.message()
        ),
    }
}

fn render_setup_access_section(
    access_save: &SetupAccessSave,
    policy: &ResolvedAccessPolicy,
    auth_db_present: bool,
) -> String {
    let global_config_path = access_save.global_config_path.display();
    let mut lines = vec![SCE_ACCESS_HEADING.to_string()];

    let saved_values = [
        access_save
            .auth_mode
            .map(|mode| ("auth.mode", mode.as_str().to_string())),
        access_save
            .telemetry_enabled
            .map(|enabled| ("telemetry.enabled", enabled.to_string())),
    ];
    if access_save.outcome == GlobalWriteOutcome::Written {
        lines.extend(saved_values.into_iter().flatten().map(|(key, saved)| {
            format!(
                "- {key}: {saved} (saved to global SCE config '{global_config_path}'; {SAVED_ACCESS_SCOPE})"
            )
        }));
    } else {
        lines.push(format!(
            "- saved access preferences: unchanged (global SCE config '{global_config_path}')"
        ));
    }

    lines.push(format!(
        "- effective auth.mode: {} ({})",
        policy.auth_mode.value.as_str(),
        format_policy_source(policy.auth_mode.source)
    ));
    lines.push(match policy.telemetry.disabled_reason() {
        Some(reason) => format!(
            "- effective telemetry: disabled (reason: {})",
            reason.as_str()
        ),
        None => "- effective telemetry: enabled".to_string(),
    });

    if let Some(file) = setup::SetupAccessGate::from_policy(policy, auth_db_present).auth_locked_by
    {
        lines.push(format!("- {}", auth_locked_by_line(file)));
    }
    if !policy.auth_allowed() && auth_db_present {
        lines.push(AUTH_LOGOUT_HINT_LINE.to_string());
    }
    if access_save.prompts_suppressed {
        lines.push(RECONFIGURE_HINT_LINE.to_string());
    }

    lines.join("\n")
}

fn format_policy_source(source: ValueSource) -> String {
    match source.config_source() {
        Some(config_source) => format!(
            "source: {}, config_source: {}",
            source.as_str(),
            config_source.as_str()
        ),
        None => format!("source: {}", source.as_str()),
    }
}

fn auth_locked_by_line(file: AuthPolicyConfigFile) -> &'static str {
    let message = UserError::AuthDisabledByConfig(file).message();
    message.lines().next().unwrap_or(message)
}

fn dispatch_targets_claude(dispatch: Option<&setup::SetupDispatch>) -> bool {
    matches!(
        dispatch,
        Some(setup::SetupDispatch::Proceed { targets, .. })
            if targets.contains(&setup::ConcreteSetupTarget::Claude)
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RepositoryAuthorizationCheck {
    Verify,
    Skip,
}

impl RepositoryAuthorizationCheck {
    fn for_mode(config_mode: Option<setup::SetupMode>) -> Self {
        match config_mode {
            Some(setup::SetupMode::Interactive) => Self::Verify,
            Some(setup::SetupMode::NonInteractive(_)) | None => Self::Skip,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaudeTelemetryAction {
    Install,
    Remove,
    SkipDisabled,
}

fn global_consent_withdrawn(global: GlobalAccessPreferences) -> bool {
    matches!(
        global,
        GlobalAccessPreferences::Valid {
            telemetry_enabled: Some(false),
            ..
        } | GlobalAccessPreferences::Valid {
            auth_mode: Some(AuthMode::Disabled),
            ..
        }
    )
}

fn claude_telemetry_action(
    global: GlobalAccessPreferences,
    effective_enabled: bool,
    claude_selected: bool,
    interactive: bool,
) -> Option<ClaudeTelemetryAction> {
    if global_consent_withdrawn(global) && (interactive || claude_selected) {
        return Some(ClaudeTelemetryAction::Remove);
    }
    if !claude_selected {
        return None;
    }
    if effective_enabled {
        Some(ClaudeTelemetryAction::Install)
    } else {
        Some(ClaudeTelemetryAction::SkipDisabled)
    }
}

fn split_pre_login_removal(
    action: Option<ClaudeTelemetryAction>,
) -> (Option<ClaudeTelemetryAction>, Option<ClaudeTelemetryAction>) {
    match action {
        Some(ClaudeTelemetryAction::Remove) => (action, None),
        _ => (None, action),
    }
}

fn non_interactive_claude_telemetry_action(
    repository_root: &Path,
    claude_selected: bool,
) -> Option<ClaudeTelemetryAction> {
    if !claude_selected {
        return None;
    }
    let global = config::resolver::resolve_default_global_config_path()
        .map_or(GlobalAccessPreferences::Invalid, |path| {
            read_global_preferences(&path)
        });
    claude_telemetry_action(
        global,
        effective_telemetry_enabled(repository_root),
        claude_selected,
        false,
    )
}

fn read_global_preferences(global_config_path: &Path) -> GlobalAccessPreferences {
    read_global_access_preferences(global_config_path).unwrap_or(GlobalAccessPreferences::Invalid)
}

fn effective_telemetry_enabled(repository_root: &Path) -> bool {
    config::resolve_auth_runtime_config(repository_root)
        .is_ok_and(|config| config.access_policy.telemetry_enabled())
}

async fn claude_telemetry_report(
    repository_root: &std::path::Path,
    action: Option<ClaudeTelemetryAction>,
    claude_selected: bool,
    authorization_check: RepositoryAuthorizationCheck,
) -> Option<String> {
    render_claude_telemetry_action(
        action?,
        claude_selected,
        setup::claude_telemetry::resolve_claude_settings_path().as_deref(),
        &setup::claude_telemetry::managed_claude_settings_path(),
        authorization_check,
        || auth_command::repository_telemetry_authorized(repository_root),
    )
    .await
}

async fn render_claude_telemetry_action<A, AF>(
    action: ClaudeTelemetryAction,
    claude_selected: bool,
    settings_path: Option<&std::path::Path>,
    managed_settings_path: &std::path::Path,
    authorization_check: RepositoryAuthorizationCheck,
    repository_authorized: A,
) -> Option<String>
where
    A: FnOnce() -> AF,
    AF: Future<Output = bool>,
{
    use setup::claude_telemetry::{
        format_claude_telemetry_removal_report, remove_claude_telemetry,
    };

    match action {
        ClaudeTelemetryAction::Remove => match settings_path {
            Some(settings_path) => format_claude_telemetry_removal_report(
                &remove_claude_telemetry(settings_path, managed_settings_path),
                claude_selected,
            ),
            None => claude_selected.then(|| CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE.to_string()),
        },
        ClaudeTelemetryAction::Install | ClaudeTelemetryAction::SkipDisabled => Some(
            render_claude_telemetry_section(
                action == ClaudeTelemetryAction::Install,
                settings_path,
                managed_settings_path,
                authorization_check,
                repository_authorized,
            )
            .await,
        ),
    }
}

async fn render_claude_telemetry_section<A, AF>(
    telemetry_enabled: bool,
    settings_path: Option<&std::path::Path>,
    managed_settings_path: &std::path::Path,
    authorization_check: RepositoryAuthorizationCheck,
    repository_authorized: A,
) -> String
where
    A: FnOnce() -> AF,
    AF: Future<Output = bool>,
{
    use setup::claude_telemetry::{
        format_claude_telemetry_report, install_claude_telemetry, ClaudeTelemetryInstallOutcome,
        RepositoryAuthorization,
    };

    if !telemetry_enabled {
        return CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE.to_string();
    }

    let installed = match settings_path {
        Some(settings_path) => install_claude_telemetry(settings_path, managed_settings_path)
            .unwrap_or(ClaudeTelemetryInstallOutcome::SkippedInvalid),
        None => ClaudeTelemetryInstallOutcome::SkippedInvalid,
    };
    let authorization = match (&installed, authorization_check) {
        (ClaudeTelemetryInstallOutcome::Configured, RepositoryAuthorizationCheck::Verify) => {
            if repository_authorized().await {
                RepositoryAuthorization::Authorized
            } else {
                RepositoryAuthorization::Unauthorized
            }
        }
        _ => RepositoryAuthorization::NotChecked,
    };

    format_claude_telemetry_report(&installed, authorization)
}

fn resolve_setup_repository(start_path: &std::path::Path) -> Result<std::path::PathBuf, CliError> {
    let repository_root = setup::ensure_git_repository(start_path).map_err(|source| {
        let user_error = match source {
            setup::GitRepositoryResolutionError::NotGitRepository(_) => UserError::NotGitRepository,
            setup::GitRepositoryResolutionError::Unexpected(_) => UserError::UnexpectedFailure,
        };

        CliError::user_with_source(user_error, source)
    })?;

    let storage_config = config::resolve_agent_trace_storage_runtime_config(&repository_root)
        .map_err(unexpected_failure)?;

    setup::ensure_git_remote(&repository_root, &storage_config.repository_remote).map_err(
        |source| {
            if setup::is_missing_git_remote_error(&source) {
                CliError::user_with_source(UserError::NotGitRemote, source)
            } else {
                unexpected_failure(source)
            }
        },
    )?;

    Ok(repository_root)
}

fn unexpected_failure(source: impl Into<anyhow::Error>) -> CliError {
    CliError::user_with_source(UserError::UnexpectedFailure, source)
}

fn setup_required_hooks_outcome_from_lifecycle(
    outcome: &RequiredHooksInstallOutcome,
) -> setup::RequiredHooksInstallOutcome {
    setup::RequiredHooksInstallOutcome {
        repository_root: outcome.repository_root.clone(),
        hooks_directory: outcome.hooks_directory.clone(),
        hook_results: outcome
            .hook_results
            .iter()
            .map(|result| setup::RequiredHookInstallResult {
                hook_name: result.hook_name.clone(),
                hook_path: result.hook_path.clone(),
                status: match result.status {
                    RequiredHookInstallStatus::Installed => {
                        setup::RequiredHookInstallStatus::Installed
                    }
                    RequiredHookInstallStatus::Updated => setup::RequiredHookInstallStatus::Updated,
                    RequiredHookInstallStatus::Skipped => setup::RequiredHookInstallStatus::Skipped,
                },
                unreachable_block_advisory: result.unreachable_block_advisory,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use anyhow::Result;
    use serde_json::Value;

    use super::*;
    use crate::services::auth_command::TelemetryProvisioningResult;
    use crate::services::config::AuthMode;
    use crate::services::setup::{
        resolve_setup_dispatch, resolve_setup_request, ConcreteSetupTarget,
        PersistedSetupConfirmations, SetupAccessGate, SetupCliOptions, SetupDispatch, SetupMode,
        SetupTarget, SetupTargetPrompter,
    };

    struct FakePrompter {
        targets: Option<Vec<ConcreteSetupTarget>>,
        auth_mode: Option<AuthMode>,
        telemetry_consent: bool,
    }

    impl FakePrompter {
        fn with_targets(targets: Option<Vec<ConcreteSetupTarget>>) -> Self {
            Self {
                targets,
                auth_mode: None,
                telemetry_consent: false,
            }
        }

        fn answering(auth_mode: AuthMode, telemetry_consent: bool) -> Self {
            Self {
                targets: Some(vec![ConcreteSetupTarget::OpenCode]),
                auth_mode: Some(auth_mode),
                telemetry_consent,
            }
        }
    }

    impl SetupTargetPrompter for FakePrompter {
        fn prompt_targets(
            &self,
            _configured: &[ConcreteSetupTarget],
        ) -> Result<Option<Vec<ConcreteSetupTarget>>> {
            Ok(self.targets.clone())
        }

        fn prompt_optional_workflows(&self, _defaults: &[String]) -> Result<Option<Vec<String>>> {
            Ok(Some(Vec::new()))
        }

        fn prompt_agent_trace_auto_sync(&self, _default: bool) -> Result<Option<bool>> {
            Ok(Some(false))
        }

        fn prompt_attribution_hooks_enabled(&self, _default: bool) -> Result<Option<bool>> {
            Ok(Some(false))
        }

        fn prompt_auth_mode(&self, default: AuthMode) -> Result<Option<AuthMode>> {
            Ok(Some(self.auth_mode.unwrap_or(default)))
        }

        fn prompt_telemetry_consent(&self, _default: bool) -> Result<Option<bool>> {
            Ok(Some(self.telemetry_consent))
        }

        fn prompt_login(&self) -> Result<Option<bool>> {
            Ok(Some(false))
        }
    }

    fn dispatch_for(mode: SetupMode, prompted: Option<Vec<ConcreteSetupTarget>>) -> SetupDispatch {
        resolve_setup_dispatch(
            mode,
            &FakePrompter::with_targets(prompted),
            &[],
            &[],
            PersistedSetupConfirmations::default(),
            SetupAccessGate::default(),
            false,
        )
        .expect("dispatch should resolve")
    }

    fn render_claude_telemetry(
        telemetry_enabled: bool,
        settings_path: &Path,
        managed_settings_path: &Path,
        authorization_check: RepositoryAuthorizationCheck,
        repository_authorized: Option<bool>,
    ) -> (String, bool) {
        let looked_up = std::cell::Cell::new(false);
        let report = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(render_claude_telemetry_section(
                telemetry_enabled,
                Some(settings_path),
                managed_settings_path,
                authorization_check,
                || {
                    looked_up.set(true);
                    async move {
                        repository_authorized.expect("repository authorization lookup must not run")
                    }
                },
            ));
        (report, looked_up.get())
    }

    fn run_claude_telemetry_stage(
        request_mode: Option<SetupMode>,
        dispatch: Option<&SetupDispatch>,
        telemetry_enabled: bool,
        root: &Path,
    ) -> Option<String> {
        let authorization_check = RepositoryAuthorizationCheck::for_mode(request_mode);
        let authorized =
            (authorization_check == RepositoryAuthorizationCheck::Verify).then_some(false);
        dispatch_targets_claude(dispatch).then(|| {
            render_claude_telemetry(
                telemetry_enabled,
                &root.join("settings.json"),
                &root.join("managed.json"),
                authorization_check,
                authorized,
            )
            .0
        })
    }

    const CLAUDE_CONFIGURED_LINE: &str =
        "Claude telemetry: configured in user-level Claude settings for native OpenTelemetry export.";
    const CLAUDE_NOT_READY_LINE: &str = "Claude telemetry: not ready for this repository; no valid SCE telemetry credential is authorized for it, so Claude will not export. Run 'sce auth login' in this repository with the SCE account it is enrolled under.";
    const CLAUDE_NOT_CHECKED_LINE: &str = "Claude telemetry: configured; repository authorization was not checked during non-interactive setup.";

    #[test]
    fn setup_claude_telemetry_readiness_follows_repository_authorization() {
        use RepositoryAuthorizationCheck::{Skip, Verify};

        let configured_not_ready = format!("{CLAUDE_CONFIGURED_LINE}\n{CLAUDE_NOT_READY_LINE}");
        let conflict = Some(r#"{"otelHeadersHelper":"my-helper"}"#);
        let conflict_line = "Claude telemetry: skipped; existing Claude settings define a different otelHeadersHelper.";
        let invalid_line = "Claude telemetry: skipped; existing Claude settings could not be parsed and were left unchanged.";
        let cases = [
            (
                "interactive authorized",
                Verify,
                true,
                None,
                Some(true),
                CLAUDE_CONFIGURED_LINE,
            ),
            (
                "interactive unauthorized",
                Verify,
                true,
                None,
                Some(false),
                configured_not_ready.as_str(),
            ),
            (
                "non-interactive not checked",
                Skip,
                true,
                None,
                None,
                CLAUDE_NOT_CHECKED_LINE,
            ),
            (
                "interactive telemetry disabled",
                Verify,
                false,
                None,
                None,
                CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE,
            ),
            (
                "non-interactive telemetry disabled",
                Skip,
                false,
                None,
                None,
                CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE,
            ),
            (
                "interactive settings conflict",
                Verify,
                true,
                conflict,
                None,
                conflict_line,
            ),
            (
                "non-interactive settings conflict",
                Skip,
                true,
                conflict,
                None,
                conflict_line,
            ),
            (
                "interactive invalid settings",
                Verify,
                true,
                Some("{not json"),
                None,
                invalid_line,
            ),
            (
                "non-interactive invalid settings",
                Skip,
                true,
                Some("{not json"),
                None,
                invalid_line,
            ),
        ];

        for (name, check, telemetry_enabled, existing, authorized, expected) in cases {
            let dir = tempfile::tempdir().unwrap();
            let settings = dir.path().join("settings.json");
            if let Some(existing) = existing {
                fs::write(&settings, existing).unwrap();
            }

            let (report, looked_up) = render_claude_telemetry(
                telemetry_enabled,
                &settings,
                &dir.path().join("managed.json"),
                check,
                authorized,
            );

            assert_eq!(report, expected, "{name}");
            assert_eq!(looked_up, authorized.is_some(), "{name}");
            assert!(!report.contains("Bearer"), "{name}");
        }
    }

    #[test]
    fn setup_repository_authorization_is_checked_only_for_interactive_setup() {
        let request_mode = |target, non_interactive| {
            resolve_setup_request(SetupCliOptions {
                target,
                non_interactive,
                ..SetupCliOptions::default()
            })
            .unwrap()
            .config_mode
        };

        assert_eq!(
            RepositoryAuthorizationCheck::for_mode(request_mode(None, false)),
            RepositoryAuthorizationCheck::Verify
        );
        for target in [SetupTarget::Claude, SetupTarget::All] {
            for non_interactive in [true, false] {
                assert_eq!(
                    RepositoryAuthorizationCheck::for_mode(request_mode(
                        Some(target),
                        non_interactive
                    )),
                    RepositoryAuthorizationCheck::Skip,
                    "{target:?} non_interactive={non_interactive}"
                );
            }
        }
        assert_eq!(
            RepositoryAuthorizationCheck::for_mode(None),
            RepositoryAuthorizationCheck::Skip
        );
    }

    fn assert_installed(root: &Path) {
        let parsed: Value =
            serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
        assert_eq!(parsed["otelHeadersHelper"], "sce auth telemetry-headers");
        assert!(parsed["env"].is_object());
    }

    fn assert_untouched(root: &Path) {
        assert!(!root.join("settings.json").exists());
    }

    #[test]
    fn setup_claude_telemetry_installs_when_claude_is_selected() {
        let interactive = dispatch_for(
            SetupMode::Interactive,
            Some(vec![ConcreteSetupTarget::Claude]),
        );
        let configured_not_ready = format!("{CLAUDE_CONFIGURED_LINE}\n{CLAUDE_NOT_READY_LINE}");
        for (mode, dispatch, expected) in [
            (
                SetupMode::NonInteractive(SetupTarget::Claude),
                dispatch_for(SetupMode::NonInteractive(SetupTarget::Claude), None),
                CLAUDE_NOT_CHECKED_LINE,
            ),
            (
                SetupMode::NonInteractive(SetupTarget::All),
                dispatch_for(SetupMode::NonInteractive(SetupTarget::All), None),
                CLAUDE_NOT_CHECKED_LINE,
            ),
            (
                SetupMode::Interactive,
                interactive,
                configured_not_ready.as_str(),
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();

            assert_eq!(
                run_claude_telemetry_stage(Some(mode), Some(&dispatch), true, dir.path())
                    .as_deref(),
                Some(expected),
                "{mode:?}"
            );
            assert_installed(dir.path());
        }
    }

    #[test]
    fn setup_claude_telemetry_skips_install_when_telemetry_is_disabled_by_config() {
        let mode = Some(SetupMode::NonInteractive(SetupTarget::Claude));
        let dispatch = dispatch_for(SetupMode::NonInteractive(SetupTarget::Claude), None);

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            run_claude_telemetry_stage(mode, Some(&dispatch), false, dir.path()).as_deref(),
            Some(CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE)
        );
        assert_untouched(dir.path());

        let dir = tempfile::tempdir().unwrap();
        let existing = br#"{"env":{"FOO":"bar"},"model":"opus"}"#;
        fs::write(dir.path().join("settings.json"), existing).unwrap();
        assert_eq!(
            run_claude_telemetry_stage(mode, Some(&dispatch), false, dir.path()).as_deref(),
            Some("Claude telemetry: skipped; SCE telemetry is disabled by config.")
        );
        assert_eq!(
            fs::read(dir.path().join("settings.json")).unwrap(),
            existing
        );

        let report = run_claude_telemetry_stage(mode, Some(&dispatch), true, dir.path()).unwrap();
        assert_eq!(report, CLAUDE_NOT_CHECKED_LINE);
        assert_installed(dir.path());
        let parsed: Value =
            serde_json::from_slice(&fs::read(dir.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(parsed["model"], "opus");
        assert_eq!(parsed["env"]["FOO"], "bar");
    }

    #[test]
    fn setup_claude_telemetry_policy_follows_layered_config() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let global = dir.path().join("global.json");
        let local_dir = dir.path().join(".sce");
        fs::create_dir_all(&local_dir).unwrap();
        let local = local_dir.join("config.json");
        let enabled = |global_json: &str, local_json: Option<&str>| {
            fs::write(&global, global_json).unwrap();
            match local_json {
                Some(json) => fs::write(&local, json).unwrap(),
                None => {
                    let _ = fs::remove_file(&local);
                }
            }
            config::resolver::resolve_auth_runtime_config_with(
                dir.path(),
                |_| None,
                |path| fs::read_to_string(path).map_err(anyhow::Error::from),
                Path::exists,
                || Ok(global.clone()),
            )
            .is_ok_and(|config| config.access_policy.telemetry_enabled())
        };

        assert!(enabled(r#"{"telemetry":{"enabled":true}}"#, None));
        assert!(!enabled("{}", None));
        assert!(!enabled("{}", Some(r#"{"telemetry":{"enabled":true}}"#)));
        assert!(!enabled(
            r#"{"telemetry":{"enabled":true}}"#,
            Some(r#"{"telemetry":{"enabled":false}}"#)
        ));
        assert!(!enabled(
            r#"{"telemetry":{"enabled":true}}"#,
            Some(r#"{"auth":{"mode":"disabled"}}"#)
        ));
        assert!(!enabled(
            r#"{"telemetry":{"enabled":true}}"#,
            Some(r#"{"telemetry":{"enabled":"yes"}}"#)
        ));
    }

    #[test]
    fn setup_claude_telemetry_leaves_settings_untouched_when_claude_is_not_installed() {
        let hooks_only = resolve_setup_request(SetupCliOptions {
            hooks: true,
            ..SetupCliOptions::default()
        })
        .unwrap();
        let bootstrap = resolve_setup_request(SetupCliOptions {
            bootstrap_context: true,
            ..SetupCliOptions::default()
        })
        .unwrap();
        assert_eq!(hooks_only.config_mode, None);
        assert!(bootstrap.context_only);

        let opencode = SetupMode::NonInteractive(SetupTarget::OpenCode);
        let dispatches = [
            (Some(opencode), Some(dispatch_for(opencode, None))),
            (
                Some(SetupMode::Interactive),
                Some(dispatch_for(SetupMode::Interactive, None)),
            ),
            (hooks_only.config_mode, None),
            (bootstrap.config_mode, None),
        ];
        for (mode, dispatch) in &dispatches {
            let dir = tempfile::tempdir().unwrap();

            assert!(
                run_claude_telemetry_stage(*mode, dispatch.as_ref(), true, dir.path()).is_none()
            );
            assert_untouched(dir.path());
        }
    }

    const SCE_GLOBAL_CONSENT: &str = r#"{"telemetry":{"enabled":true}}"#;
    const SCE_GLOBAL_WITHDRAWAL: &str = r#"{"telemetry":{"enabled":false}}"#;
    const SCE_GLOBAL_AUTH_DISABLED: &str = r#"{"auth":{"mode":"disabled"}}"#;
    const UNRELATED_CLAUDE_SETTINGS: &str = r#"{"env":{"FOO":"bar"},"model":"opus"}"#;

    fn removed_line(settings: &Path) -> String {
        format!(
            "Claude telemetry: removed SCE-managed telemetry settings from user-level Claude settings '{}' because the global SCE config disables telemetry.",
            settings.display()
        )
    }

    fn authorization_must_not_run() -> std::future::Ready<bool> {
        panic!("repository authorization must not run");
    }

    fn run_claude_telemetry_action(
        action: ClaudeTelemetryAction,
        claude_selected: bool,
        settings: &Path,
        authorization_check: RepositoryAuthorizationCheck,
    ) -> Option<String> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(render_claude_telemetry_action(
                action,
                claude_selected,
                Some(settings),
                &settings.with_file_name("managed.json"),
                authorization_check,
                authorization_must_not_run,
            ))
    }

    fn install_sce_claude_settings(settings: &Path) -> Vec<u8> {
        fs::write(settings, UNRELATED_CLAUDE_SETTINGS).unwrap();
        run_claude_telemetry_action(
            ClaudeTelemetryAction::Install,
            true,
            settings,
            RepositoryAuthorizationCheck::Skip,
        );
        assert_installed(settings.parent().unwrap());
        fs::read(settings).unwrap()
    }

    #[test]
    fn claude_telemetry_action_follows_global_withdrawal_only() {
        use ClaudeTelemetryAction::{Install, Remove, SkipDisabled};

        let valid = |auth_mode, telemetry_enabled| GlobalAccessPreferences::Valid {
            auth_mode,
            telemetry_enabled,
        };
        let withdrawn = [
            valid(None, Some(false)),
            valid(Some(AuthMode::Disabled), None),
            valid(Some(AuthMode::Disabled), Some(true)),
            valid(Some(AuthMode::Optional), Some(false)),
        ];
        let not_withdrawn = [
            valid(None, Some(true)),
            valid(Some(AuthMode::Optional), Some(true)),
            valid(Some(AuthMode::Mandatory), None),
            valid(None, None),
            GlobalAccessPreferences::Missing,
            GlobalAccessPreferences::Invalid,
        ];
        let modes = [(true, true), (true, false), (false, true), (false, false)];

        for global in withdrawn {
            for (claude_selected, interactive) in modes {
                assert_eq!(
                    claude_telemetry_action(global, false, claude_selected, interactive),
                    (claude_selected || interactive).then_some(Remove),
                    "{global:?} claude_selected={claude_selected} interactive={interactive}"
                );
            }
        }
        for global in not_withdrawn {
            for repository_opt_out in [false, true] {
                for (claude_selected, interactive) in modes {
                    let expected = match (claude_selected, repository_opt_out) {
                        (false, _) => None,
                        (true, false) => Some(Install),
                        (true, true) => Some(SkipDisabled),
                    };
                    assert_eq!(
                        claude_telemetry_action(
                            global,
                            !repository_opt_out,
                            claude_selected,
                            interactive
                        ),
                        expected,
                        "{global:?} repository_opt_out={repository_opt_out} claude_selected={claude_selected} interactive={interactive}"
                    );
                }
            }
        }
    }

    #[test]
    fn setup_claude_telemetry_consent_cycle_is_idempotent() {
        let fixture = AccessFixture::new();
        let settings = fixture.global.with_file_name("settings.json");
        fs::write(&settings, UNRELATED_CLAUDE_SETTINGS).unwrap();
        let run = |global_json: &str, interactive: bool| {
            fs::write(&fixture.global, global_json).unwrap();
            let action = claude_telemetry_action(
                read_global_preferences(&fixture.global),
                fixture.policy().telemetry_enabled(),
                true,
                interactive,
            )
            .unwrap();
            run_claude_telemetry_action(
                action,
                true,
                &settings,
                RepositoryAuthorizationCheck::for_mode(Some(if interactive {
                    SetupMode::Interactive
                } else {
                    SetupMode::NonInteractive(SetupTarget::Claude)
                })),
            )
            .unwrap()
        };
        let modified = || fs::metadata(&settings).unwrap().modified().unwrap();

        assert_eq!(run(SCE_GLOBAL_CONSENT, false), CLAUDE_NOT_CHECKED_LINE);
        let installed = fs::read(&settings).unwrap();

        assert_eq!(run(SCE_GLOBAL_WITHDRAWAL, true), removed_line(&settings));
        let removed = fs::read(&settings).unwrap();
        let parsed: Value = serde_json::from_slice(&removed).unwrap();
        assert_eq!(
            parsed,
            serde_json::from_str::<Value>(UNRELATED_CLAUDE_SETTINGS).unwrap()
        );
        let removed_at = modified();

        assert_eq!(
            run(SCE_GLOBAL_WITHDRAWAL, false),
            CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE
        );
        assert_eq!(fs::read(&settings).unwrap(), removed);
        assert_eq!(modified(), removed_at);

        assert_eq!(run(SCE_GLOBAL_CONSENT, false), CLAUDE_NOT_CHECKED_LINE);
        assert_eq!(fs::read(&settings).unwrap(), installed);

        assert_eq!(
            run(SCE_GLOBAL_AUTH_DISABLED, false),
            removed_line(&settings)
        );
        assert_eq!(fs::read(&settings).unwrap(), removed);
    }

    #[test]
    fn setup_claude_telemetry_repository_opt_out_preserves_user_settings() {
        let fixture = AccessFixture::new();
        fs::write(&fixture.global, SCE_GLOBAL_CONSENT).unwrap();
        let settings = fixture.global.with_file_name("settings.json");
        let installed = install_sce_claude_settings(&settings);
        let explicit = fixture.global.with_file_name("explicit.json");
        fs::write(&explicit, SCE_GLOBAL_WITHDRAWAL).unwrap();
        let explicit_policy = |root: &Path| {
            config::resolver::resolve_auth_runtime_config_with(
                root,
                |key| (key == "SCE_CONFIG_FILE").then(|| explicit.display().to_string()),
                |path| fs::read_to_string(path).map_err(anyhow::Error::from),
                Path::exists,
                || Ok(fixture.global.clone()),
            )
            .unwrap()
            .access_policy
        };

        let cases = [
            (
                "repository A telemetry opt-out",
                Some(SCE_GLOBAL_WITHDRAWAL),
                false,
            ),
            (
                "repository A auth disabled",
                Some(SCE_GLOBAL_AUTH_DISABLED),
                false,
            ),
            ("explicit layer telemetry opt-out", None, true),
            ("repository B without opt-out", None, false),
        ];
        for (name, repository_json, explicit_layer) in cases {
            match repository_json {
                Some(json) => fixture.write_repo_config(json),
                None => {
                    let _ = fs::remove_file(fixture.repo.join(".sce").join("config.json"));
                }
            }
            let policy = if explicit_layer {
                explicit_policy(&fixture.repo)
            } else {
                fixture.policy()
            };
            let opted_out = repository_json.is_some() || explicit_layer;
            assert_eq!(!policy.telemetry_enabled(), opted_out, "{name}");

            for (claude_selected, interactive) in
                [(true, true), (true, false), (false, true), (false, false)]
            {
                let action = claude_telemetry_action(
                    read_global_preferences(&fixture.global),
                    policy.telemetry_enabled(),
                    claude_selected,
                    interactive,
                );
                let expected = claude_selected.then_some(if opted_out {
                    ClaudeTelemetryAction::SkipDisabled
                } else {
                    ClaudeTelemetryAction::Install
                });
                assert_eq!(action, expected, "{name}");
                if let Some(action) = action {
                    let report = run_claude_telemetry_action(
                        action,
                        claude_selected,
                        &settings,
                        RepositoryAuthorizationCheck::Skip,
                    );
                    if opted_out {
                        assert_eq!(
                            report.as_deref(),
                            Some(CLAUDE_TELEMETRY_DISABLED_BY_CONFIG_LINE),
                            "{name}"
                        );
                    }
                }
                assert_eq!(fs::read(&settings).unwrap(), installed, "{name}");
            }
        }
    }

    #[test]
    fn setup_claude_telemetry_removal_never_checks_authorization() {
        for (mode, claude_selected) in [
            (SetupMode::Interactive, true),
            (SetupMode::Interactive, false),
            (SetupMode::NonInteractive(SetupTarget::Claude), true),
            (SetupMode::NonInteractive(SetupTarget::All), true),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let settings = dir.path().join("settings.json");
            install_sce_claude_settings(&settings);

            let action = claude_telemetry_action(
                GlobalAccessPreferences::Valid {
                    auth_mode: None,
                    telemetry_enabled: Some(false),
                },
                false,
                claude_selected,
                mode == SetupMode::Interactive,
            );
            assert_eq!(action, Some(ClaudeTelemetryAction::Remove), "{mode:?}");

            let report = run_claude_telemetry_action(
                ClaudeTelemetryAction::Remove,
                claude_selected,
                &settings,
                RepositoryAuthorizationCheck::for_mode(Some(mode)),
            );
            assert_eq!(report, Some(removed_line(&settings)), "{mode:?}");
            assert_eq!(
                fs::read_to_string(&settings).unwrap(),
                "{\n  \"env\": {\n    \"FOO\": \"bar\"\n  },\n  \"model\": \"opus\"\n}\n",
                "{mode:?}"
            );
        }
    }

    struct AccessFixture {
        _dir: tempfile::TempDir,
        repo: std::path::PathBuf,
        global: std::path::PathBuf,
    }

    impl AccessFixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let repo = dir.path().join("repo");
            fs::create_dir_all(repo.join(".git")).unwrap();
            let global = dir.path().join("global").join("config.json");
            fs::create_dir_all(global.parent().unwrap()).unwrap();
            Self {
                _dir: dir,
                repo,
                global,
            }
        }

        fn write_repo_config(&self, json: &str) {
            fs::create_dir_all(self.repo.join(".sce")).unwrap();
            fs::write(self.repo.join(".sce").join("config.json"), json).unwrap();
        }

        fn resolve_policy(&self) -> impl Fn(&Path) -> Result<ResolvedAccessPolicy> + '_ {
            move |root| {
                config::resolver::resolve_auth_runtime_config_with(
                    root,
                    |_| None,
                    |path| fs::read_to_string(path).map_err(anyhow::Error::from),
                    Path::exists,
                    || Ok(self.global.clone()),
                )
                .map(|config| config.access_policy)
            }
        }

        fn prepare(&self, prompter: &FakePrompter) -> Result<InteractiveSetup, CliError> {
            self.prepare_with(prompter, false)
        }

        fn prepare_with(
            &self,
            prompter: &FakePrompter,
            reconfigure: bool,
        ) -> Result<InteractiveSetup, CliError> {
            prepare_interactive_setup(
                &self.repo,
                prompter,
                &[],
                &self.global,
                self.resolve_policy(),
                false,
                reconfigure,
            )
        }

        fn report(&self, access_save: &SetupAccessSave, auth_db_present: bool) -> String {
            setup_access_report(
                &self.repo,
                access_save,
                self.resolve_policy(),
                auth_db_present,
            )
            .unwrap()
            .0
        }

        fn policy(&self) -> ResolvedAccessPolicy {
            self.resolve_policy()(&self.repo).unwrap()
        }
    }

    fn ready(setup: Result<InteractiveSetup, CliError>) -> (SetupDispatch, SetupAccessSave) {
        match setup.expect("interactive setup should prepare") {
            InteractiveSetup::Ready {
                dispatch,
                access_save,
            } => (dispatch, access_save),
            InteractiveSetup::Cancelled => panic!("interactive setup should not cancel"),
        }
    }

    fn read_json_file(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn interactive_setup_saves_access_answers_only_to_the_global_config() {
        let fixture = AccessFixture::new();
        fs::write(&fixture.global, "{\"log_level\": \"debug\"}\n").unwrap();

        let (dispatch, access_save) =
            ready(fixture.prepare(&FakePrompter::answering(AuthMode::Mandatory, true)));

        let global = read_json_file(&fixture.global);
        assert_eq!(global["auth"]["mode"], "mandatory");
        assert_eq!(global["telemetry"]["enabled"], true);
        assert_eq!(global["log_level"], "debug");
        assert_eq!(access_save.outcome, GlobalWriteOutcome::Written);

        let SetupDispatch::Proceed {
            targets,
            target_persistence,
            agent_trace_auto_sync,
            attribution_hooks_enabled,
            ..
        } = dispatch
        else {
            panic!("dispatch should proceed");
        };
        setup::persist_integration_targets(
            &fixture.repo,
            &targets,
            target_persistence,
            &[],
            agent_trace_auto_sync,
            attribution_hooks_enabled,
        )
        .unwrap();

        let repository = read_json_file(&fixture.repo.join(".sce").join("config.json"));
        let repository = repository.as_object().unwrap();
        assert!(!repository.contains_key("auth"));
        assert!(!repository.contains_key("telemetry"));
    }

    #[test]
    fn interactive_setup_rerun_leaves_the_global_config_bytes_unchanged() {
        let fixture = AccessFixture::new();
        let prompter = FakePrompter::answering(AuthMode::Optional, false);

        ready(fixture.prepare(&prompter));
        let first = fs::read(&fixture.global).unwrap();

        let (_, rerun) = ready(fixture.prepare(&prompter));
        assert_eq!(
            rerun,
            SetupAccessSave {
                global_config_path: fixture.global.clone(),
                auth_mode: None,
                telemetry_enabled: None,
                outcome: GlobalWriteOutcome::Unchanged,
                prompts_suppressed: true,
            }
        );
        assert_eq!(fs::read(&fixture.global).unwrap(), first);

        let repeated = persist_setup_access_preferences(
            &fixture.global,
            Some(AuthMode::Optional),
            Some(false),
        )
        .unwrap();
        assert_eq!(repeated.outcome, GlobalWriteOutcome::Unchanged);
        assert_eq!(fs::read(&fixture.global).unwrap(), first);
    }

    #[test]
    fn interactive_setup_skips_access_writes_when_the_global_config_is_invalid() {
        let fixture = AccessFixture::new();
        let invalid = b"{\"auth\": {\"mode\": \"sometimes\"}}\n";
        fs::write(&fixture.global, invalid).unwrap();

        let (dispatch, access_save) =
            ready(fixture.prepare(&FakePrompter::answering(AuthMode::Mandatory, true)));

        assert!(matches!(
            dispatch,
            SetupDispatch::Proceed {
                auth_mode: None,
                telemetry_enabled: None,
                ..
            }
        ));
        assert_eq!(access_save.outcome, GlobalWriteOutcome::Unchanged);
        assert_eq!(fs::read(&fixture.global).unwrap(), invalid);
    }

    #[cfg(unix)]
    #[test]
    fn interactive_setup_aborts_before_repository_writes_when_the_global_config_is_read_only() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = AccessFixture::new();
        let global_dir = fixture.global.parent().unwrap().to_path_buf();
        fs::set_permissions(&global_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let result = fixture.prepare(&FakePrompter::answering(AuthMode::Optional, true));
        fs::set_permissions(&global_dir, fs::Permissions::from_mode(0o755)).unwrap();

        let error = result.expect_err("read-only global config should abort setup");
        let message = error.to_string();
        assert!(message.starts_with(&format!(
            "Failed to save SCE access preferences to the global SCE config '{}'.\n",
            fixture.global.display()
        )));
        assert!(message.ends_with(GLOBAL_ACCESS_WRITE_REMEDIATION));
        assert_eq!(error.code(), "SCE-ERR-RUNTIME");
        assert!(!fixture.global.exists());
        assert!(!fixture.repo.join("context").exists());
        assert!(!fixture.repo.join(".sce").exists());
    }

    #[cfg(unix)]
    #[test]
    fn interactive_setup_aborts_before_repository_writes_when_the_global_config_is_a_symlink() {
        let fixture = AccessFixture::new();
        let target = fixture.global.with_file_name("managed.json");
        let original = b"{\"log_level\": \"info\"}\n";
        fs::write(&target, original).unwrap();
        std::os::unix::fs::symlink(&target, &fixture.global).unwrap();

        let error = fixture
            .prepare(&FakePrompter::answering(AuthMode::Optional, true))
            .expect_err("symlinked global config should abort setup");

        let message = error.to_string();
        assert!(message.contains("is a symlink and was left unchanged"));
        assert!(message.ends_with(GLOBAL_ACCESS_SYMLINK_REMEDIATION));
        assert!(!message.contains("sce config validate"));
        assert_eq!(fs::read_link(&fixture.global).unwrap(), target);
        assert_eq!(fs::read(&target).unwrap(), original);
        assert!(!fixture.repo.join("context").exists());
        assert!(!fixture.repo.join(".sce").exists());
    }

    #[test]
    fn setup_access_write_refuses_an_invalid_global_config_with_a_fixed_message() {
        let fixture = AccessFixture::new();
        fs::write(&fixture.global, "{not json").unwrap();

        let error = persist_setup_access_preferences(&fixture.global, None, Some(true))
            .expect_err("invalid global config should not be overwritten");

        let message = error.to_string();
        assert!(message.starts_with(&format!(
            "Failed to save SCE access preferences to the global SCE config '{}'.\nGlobal SCE config '{}' is invalid and was left unchanged: ",
            fixture.global.display(),
            fixture.global.display()
        )));
        assert!(message.ends_with(&format!("\n{GLOBAL_ACCESS_WRITE_REMEDIATION}")));
        assert_eq!(fs::read(&fixture.global).unwrap(), b"{not json");
    }

    #[test]
    fn setup_access_section_reports_repository_opt_out_after_global_consent() {
        let fixture = AccessFixture::new();
        fixture.write_repo_config(r#"{"telemetry":{"enabled":false}}"#);

        let (_, access_save) =
            ready(fixture.prepare(&FakePrompter::answering(AuthMode::Optional, true)));

        assert_eq!(
            fixture.report(&access_save, false),
            format!(
                "SCE access:\n\
                 - auth.mode: optional (saved to global SCE config '{path}'; applies to all repositories on this machine)\n\
                 - telemetry.enabled: true (saved to global SCE config '{path}'; applies to all repositories on this machine)\n\
                 - effective auth.mode: optional (source: config_file, config_source: default_discovered_global)\n\
                 - effective telemetry: disabled (reason: repository_opt_out)",
                path = fixture.global.display()
            )
        );
    }

    #[test]
    fn setup_access_section_renders_exact_lines() {
        let unchanged = |fixture: &AccessFixture| SetupAccessSave {
            global_config_path: fixture.global.clone(),
            auth_mode: None,
            telemetry_enabled: None,
            outcome: GlobalWriteOutcome::Unchanged,
            prompts_suppressed: false,
        };

        let fixture = AccessFixture::new();
        fs::write(
            &fixture.global,
            r#"{"auth":{"mode":"optional"},"telemetry":{"enabled":true}}"#,
        )
        .unwrap();
        assert_eq!(
            fixture.report(&unchanged(&fixture), true),
            format!(
                "SCE access:\n\
                 - saved access preferences: unchanged (global SCE config '{}')\n\
                 - effective auth.mode: optional (source: config_file, config_source: default_discovered_global)\n\
                 - effective telemetry: enabled",
                fixture.global.display()
            )
        );

        let fixture = AccessFixture::new();
        let access_save =
            persist_setup_access_preferences(&fixture.global, Some(AuthMode::Disabled), None)
                .unwrap();
        assert_eq!(
            fixture.report(&access_save, true),
            format!(
                "SCE access:\n\
                 - auth.mode: disabled (saved to global SCE config '{}'; applies to all repositories on this machine)\n\
                 - effective auth.mode: disabled (source: config_file, config_source: default_discovered_global)\n\
                 - effective telemetry: disabled (reason: auth_disabled)\n\
                 {AUTH_LOGOUT_HINT_LINE}",
                fixture.global.display()
            )
        );
        assert!(!fixture
            .report(&access_save, false)
            .contains("sce auth logout"));

        let fixture = AccessFixture::new();
        fixture.write_repo_config(r#"{"auth":{"mode":"disabled"}}"#);
        assert_eq!(
            fixture.report(&unchanged(&fixture), false),
            format!(
                "SCE access:\n\
                 - saved access preferences: unchanged (global SCE config '{}')\n\
                 - effective auth.mode: disabled (source: config_file, config_source: default_discovered_local)\n\
                 - effective telemetry: disabled (reason: auth_disabled)\n\
                 - Authentication is disabled by `auth.mode: disabled` in the repository SCE config (`.sce/config.json`).",
                fixture.global.display()
            )
        );
    }

    #[test]
    fn setup_access_section_hints_reconfigure_when_saved_answers_suppressed_prompts() {
        let fixture = AccessFixture::new();
        let prompter = FakePrompter::answering(AuthMode::Optional, false);
        ready(fixture.prepare(&prompter));

        let (_, rerun) = ready(fixture.prepare(&prompter));
        assert_eq!(
            fixture.report(&rerun, false),
            format!(
                "SCE access:\n\
                 - saved access preferences: unchanged (global SCE config '{}')\n\
                 - effective auth.mode: optional (source: config_file, config_source: default_discovered_global)\n\
                 - effective telemetry: disabled (reason: user_opt_out)\n\
                 - Run 'sce setup --reconfigure' to change saved setup answers.",
                fixture.global.display()
            )
        );

        let (_, reconfigured) = ready(fixture.prepare_with(&prompter, true));
        assert!(!reconfigured.prompts_suppressed);
        assert!(!fixture
            .report(&reconfigured, false)
            .contains(RECONFIGURE_HINT_LINE));
    }

    #[test]
    fn reconfigure_rewrites_saved_global_answers() {
        let fixture = AccessFixture::new();
        ready(fixture.prepare(&FakePrompter::answering(AuthMode::Disabled, false)));
        assert_eq!(read_json_file(&fixture.global)["auth"]["mode"], "disabled");

        let (rerun, _) =
            ready(fixture.prepare(&FakePrompter::answering(AuthMode::Mandatory, true)));
        assert!(matches!(
            rerun,
            SetupDispatch::Proceed {
                auth_mode: None,
                telemetry_enabled: None,
                ..
            }
        ));
        assert_eq!(read_json_file(&fixture.global)["auth"]["mode"], "disabled");

        let (dispatch, access_save) =
            ready(fixture.prepare_with(&FakePrompter::answering(AuthMode::Mandatory, true), true));
        assert!(matches!(
            dispatch,
            SetupDispatch::Proceed {
                auth_mode: Some(AuthMode::Mandatory),
                telemetry_enabled: Some(true),
                ..
            }
        ));
        assert_eq!(access_save.outcome, GlobalWriteOutcome::Written);
        let global = read_json_file(&fixture.global);
        assert_eq!(global["auth"]["mode"], "mandatory");
        assert_eq!(global["telemetry"]["enabled"], true);
    }

    const LOGIN_RENDERED: &str = "Logged in to SCE.\nTelemetry credential provisioned.";

    fn login_success(telemetry: TelemetryProvisioningResult) -> SetupLoginStatus {
        SetupLoginStatus::Authenticated(LoginReport {
            rendered: LOGIN_RENDERED.to_string(),
            telemetry,
        })
    }

    fn policy_for(global: Option<&str>, repository: Option<&str>) -> ResolvedAccessPolicy {
        let fixture = AccessFixture::new();
        if let Some(global) = global {
            fs::write(&fixture.global, global).unwrap();
        }
        if let Some(repository) = repository {
            fixture.write_repo_config(repository);
        }
        fixture.policy()
    }

    fn run_login_step<F>(action: SetupLoginAction, login: F) -> Result<SetupLoginStatus, CliError>
    where
        F: FnOnce() -> Result<LoginReport, CliError>,
    {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(run_setup_login_step(action, || async move { login() }))
    }

    fn login_must_not_run() -> Result<LoginReport, CliError> {
        panic!("login must not be invoked")
    }

    #[test]
    fn setup_login_action_covers_every_effective_mode_and_intent() {
        use crate::services::setup::SetupLoginIntent::{Declined, NotOffered, Requested};

        let optional = policy_for(Some(r#"{"auth":{"mode":"optional"}}"#), None);
        let mandatory = policy_for(Some(r#"{"auth":{"mode":"mandatory"}}"#), None);
        let disabled = policy_for(Some(r#"{"auth":{"mode":"disabled"}}"#), None);
        let repository_mandatory = policy_for(
            Some(r#"{"auth":{"mode":"optional"}}"#),
            Some(r#"{"auth":{"mode":"mandatory"}}"#),
        );
        let repository_disabled = policy_for(
            Some(r#"{"auth":{"mode":"mandatory"}}"#),
            Some(r#"{"auth":{"mode":"disabled"}}"#),
        );
        let default = policy_for(None, None);

        let auth_disabled = SetupLoginAction::Skip(SetupLoginSkip::AuthDisabled);
        let required = SetupLoginAction::Run { required: true };
        for (policy, intent, expected) in [
            (
                &optional,
                Requested,
                SetupLoginAction::Run { required: false },
            ),
            (
                &optional,
                Declined,
                SetupLoginAction::Skip(SetupLoginSkip::Declined),
            ),
            (
                &optional,
                NotOffered,
                SetupLoginAction::Skip(SetupLoginSkip::NotOffered),
            ),
            (
                &default,
                Requested,
                SetupLoginAction::Run { required: false },
            ),
            (
                &default,
                NotOffered,
                SetupLoginAction::Skip(SetupLoginSkip::NotOffered),
            ),
            (&mandatory, Requested, required),
            (&mandatory, Declined, required),
            (&mandatory, NotOffered, required),
            (&repository_mandatory, Declined, required),
            (&repository_mandatory, NotOffered, required),
            (&disabled, Requested, auth_disabled),
            (&disabled, Declined, auth_disabled),
            (&disabled, NotOffered, auth_disabled),
            (&repository_disabled, Requested, auth_disabled),
            (&repository_disabled, NotOffered, auth_disabled),
        ] {
            assert_eq!(
                setup_login_action(policy, intent),
                expected,
                "{:?} {intent:?}",
                policy.auth_mode
            );
        }
    }

    #[test]
    fn setup_login_step_never_invokes_login_when_skipped() {
        for skip in [
            SetupLoginSkip::AuthDisabled,
            SetupLoginSkip::Declined,
            SetupLoginSkip::NotOffered,
        ] {
            let status = run_login_step(SetupLoginAction::Skip(skip), login_must_not_run).unwrap();
            assert_eq!(status, SetupLoginStatus::Skipped(skip));
        }
    }

    #[test]
    fn setup_login_step_reports_optional_failure_as_successful_setup() {
        let status = run_login_step(SetupLoginAction::Run { required: false }, || {
            Err(CliError::user_with_source(
                UserError::NotAuthenticated,
                anyhow::anyhow!("device code expired"),
            ))
        })
        .unwrap();
        assert_eq!(
            status,
            SetupLoginStatus::Failed(UserError::NotAuthenticated)
        );
        assert_eq!(
            render_setup_login_section(&status),
            format!(
                "SCE login: failed: {}\nRun 'sce auth login' to retry.",
                UserError::NotAuthenticated.message()
            )
        );

        let status = run_login_step(SetupLoginAction::Run { required: false }, || {
            Err(CliError::runtime(anyhow::anyhow!(
                "access_token=secret-value leaked in a diagnostic"
            )))
        })
        .unwrap();
        assert_eq!(
            status,
            SetupLoginStatus::Failed(UserError::UnexpectedFailure)
        );
        assert!(!render_setup_login_section(&status).contains("secret-value"));
    }

    #[test]
    fn setup_login_step_fails_mandatory_login_with_setup_login_required() {
        let error = run_login_step(SetupLoginAction::Run { required: true }, || {
            Err(CliError::user_with_source(
                UserError::NotAuthenticated,
                anyhow::anyhow!("device code expired"),
            ))
        })
        .unwrap_err();

        let CliError::User {
            error: user_error,
            source: Some(source),
        } = &error
        else {
            panic!("expected a user error with the login error as source");
        };
        assert_eq!(*user_error, UserError::SetupLoginRequired);
        assert_eq!(user_error.key(), "auth.setup_login_required");
        assert_eq!(error.code(), "SCE-ERR-RUNTIME");
        assert_eq!(source.to_string(), UserError::NotAuthenticated.message());
    }

    #[test]
    fn setup_login_step_includes_login_text_for_mandatory_and_accepted_optional_login() {
        for required in [true, false] {
            let status = run_login_step(SetupLoginAction::Run { required }, || {
                Ok(LoginReport {
                    rendered: LOGIN_RENDERED.to_string(),
                    telemetry: TelemetryProvisioningResult::Issued,
                })
            })
            .unwrap();
            assert_eq!(status, login_success(TelemetryProvisioningResult::Issued));
            assert_eq!(render_setup_login_section(&status), LOGIN_RENDERED);
        }
    }

    #[test]
    fn setup_login_section_renders_exact_skip_lines_without_credentials() {
        for (skip, expected) in [
            (
                SetupLoginSkip::AuthDisabled,
                "SCE login: skipped; SCE authentication is disabled by config.",
            ),
            (
                SetupLoginSkip::Declined,
                "SCE login: skipped; run 'sce auth login' to log in.",
            ),
            (
                SetupLoginSkip::NotOffered,
                "SCE login: not requested; run 'sce auth login' to log in or refresh credentials.",
            ),
        ] {
            assert_eq!(
                render_setup_login_section(&SetupLoginStatus::Skipped(skip)),
                expected
            );
        }

        let tokens = crate::services::token_storage::StoredTokens {
            access_token: "secret-access-token-value".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 3600,
            refresh_token: "secret-refresh-token-value".to_string(),
            scope: None,
            stored_at_unix_seconds: 1,
        };
        let rendered = auth_command::render_login_refresh_result_with_telemetry(
            &tokens,
            AuthFormat::Text,
            TelemetryProvisioningResult::Issued,
        )
        .unwrap();
        let output = render_setup_login_section(&SetupLoginStatus::Authenticated(LoginReport {
            rendered,
            telemetry: TelemetryProvisioningResult::Issued,
        }));
        assert!(output.contains("Authentication succeeded."));
        assert!(!output.contains("secret-access-token-value"));
        assert!(!output.contains("secret-refresh-token-value"));
    }
}
