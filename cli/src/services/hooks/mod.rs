use anyhow::Context;

mod claude_transforms;
mod commit_hooks;
mod conversation_trace;
mod diff_trace;

pub(crate) mod mutation_scope_lock;

mod runtime;

pub mod claude_bridge_session;
pub mod claude_model_state;
pub mod claude_mutation_scope;
pub mod claude_transcript;
pub mod codex;
pub mod codex_mutation_scope;
pub mod command;
pub mod lifecycle;
pub mod mutation_scope;
pub mod mutation_scope_health;
pub mod mutation_scope_owner;
pub mod opencode_mutation_scope;
pub mod pi_mutation_scope;

pub(crate) use commit_hooks::*;
pub(crate) use conversation_trace::*;
pub(crate) use diff_trace::*;
pub(crate) use runtime::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HookSubcommand {
    PreCommit,
    CommitMsg {
        message_file: std::path::PathBuf,
    },
    PostCommit {
        vcs_type: Option<crate::services::agent_trace::AgentTraceVcsType>,
        remote_url: Option<String>,
    },
    PostRewrite {
        rewrite_method: String,
    },
    DiffTrace,
    ConversationTrace,
    Codex,
    ClaudeModelState,
    MutationScope,
    ClaudeMutationScope,
    CodexMutationScope,
    OpenCodeMutationScope,
    PiMutationScope,
    ExternalMutationGuard,
}

pub const NAME: &str = "hooks";
pub const CANONICAL_SCE_COAUTHOR_TRAILER: &str = "Co-authored-by: SCE <sce@crocoder.dev>";

pub async fn run_hooks_subcommand<L: crate::services::observability::traits::Logger>(
    subcommand: &HookSubcommand,
    logger: Option<&L>,
) -> anyhow::Result<String> {
    let repository_root = std::env::current_dir().with_context(|| {
        format!(
            "Failed to determine current directory for {}.",
            hook_runtime_invocation_name(subcommand)
        )
    })?;

    run_hooks_subcommand_in_repo(&repository_root, subcommand, logger).await
}

async fn run_hooks_subcommand_in_repo<L: crate::services::observability::traits::Logger>(
    repository_root: &std::path::Path,
    subcommand: &HookSubcommand,
    logger: Option<&L>,
) -> anyhow::Result<String> {
    match subcommand {
        HookSubcommand::PreCommit => run_pre_commit_subcommand_with_trace(repository_root),
        HookSubcommand::CommitMsg { message_file } => {
            run_commit_msg_subcommand_with_trace(repository_root, subcommand, message_file, logger)
                .await
        }
        HookSubcommand::PostCommit {
            vcs_type,
            remote_url,
        } => {
            run_post_commit_subcommand_with_trace(
                repository_root,
                *vcs_type,
                remote_url.as_deref(),
                logger,
            )
            .await
        }
        HookSubcommand::PostRewrite { rewrite_method } => {
            run_post_rewrite_subcommand_with_trace(repository_root, subcommand, rewrite_method)
        }
        HookSubcommand::DiffTrace => Ok(run_diff_trace_subcommand(repository_root, logger).await),
        HookSubcommand::ConversationTrace => {
            Ok(run_conversation_trace_subcommand(repository_root, logger).await)
        }
        HookSubcommand::Codex => Ok(codex::run_codex_subcommand(repository_root, logger).await),
        HookSubcommand::ClaudeModelState => Ok(
            claude_model_state::run_claude_model_state_subcommand(repository_root, logger).await,
        ),
        HookSubcommand::MutationScope => {
            mutation_scope::run_mutation_scope_subcommand(repository_root, logger).await
        }
        HookSubcommand::ClaudeMutationScope => {
            claude_mutation_scope::run_claude_mutation_scope_subcommand(logger).await
        }
        HookSubcommand::CodexMutationScope => {
            codex_mutation_scope::run_codex_mutation_scope_subcommand(logger).await
        }
        HookSubcommand::OpenCodeMutationScope => {
            opencode_mutation_scope::run_opencode_mutation_scope_subcommand(logger).await
        }
        HookSubcommand::PiMutationScope => {
            pi_mutation_scope::run_pi_mutation_scope_subcommand(logger).await
        }
        HookSubcommand::ExternalMutationGuard => {
            mutation_scope::run_external_mutation_guard_subcommand(repository_root, logger).await
        }
    }
}
