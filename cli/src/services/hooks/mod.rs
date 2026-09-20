use anyhow::Context;

#[cfg(test)]
use crate::services::agent_trace::{validate_agent_trace_value, AgentTrace, AgentTraceVcsType};
#[cfg(test)]
use crate::services::agent_trace_db::repository::RepositoryAgentTraceDb;
#[cfg(test)]
use crate::services::agent_trace_db::{
    DiffTraceInsert, MessageRole, PartType, RecentDiffTracePatches, PAYLOAD_TYPE_PATCH,
    PAYLOAD_TYPE_STRUCTURED,
};
#[cfg(test)]
use crate::services::agent_trace_storage::{
    resolve_agent_trace_storage_at_state_root, AgentTraceStorageContext,
};
#[cfg(test)]
use crate::services::observability::traits::Logger;
#[cfg(test)]
use crate::services::patch::{
    intersect_patches as intersect_patches_fn, load_patch_from_json,
    parse_patch as parse_patch_from_text, ParsedPatch,
};
#[cfg(test)]
use anyhow::{anyhow, Result};
#[cfg(test)]
use serde_json::{json, to_string as serialize_to_json, Value};

mod claude_transforms;
mod commit_hooks;
mod conversation_trace;
mod diff_trace;
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
pub mod opencode_mutation_scope;
pub mod pi_mutation_scope;

pub(crate) use commit_hooks::*;
pub(crate) use conversation_trace::*;
pub(crate) use diff_trace::*;
pub(crate) use runtime::*;

#[cfg(test)]
mod tests;

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

pub fn run_hooks_subcommand(
    subcommand: &HookSubcommand,
    logger: Option<&dyn crate::services::observability::traits::Logger>,
) -> anyhow::Result<String> {
    let repository_root = std::env::current_dir().with_context(|| {
        format!(
            "Failed to determine current directory for {}.",
            hook_runtime_invocation_name(subcommand)
        )
    })?;

    run_hooks_subcommand_in_repo(&repository_root, subcommand, logger)
}

fn run_hooks_subcommand_in_repo(
    repository_root: &std::path::Path,
    subcommand: &HookSubcommand,
    logger: Option<&dyn crate::services::observability::traits::Logger>,
) -> anyhow::Result<String> {
    match subcommand {
        HookSubcommand::PreCommit => run_pre_commit_subcommand_with_trace(repository_root),
        HookSubcommand::CommitMsg { message_file } => {
            run_commit_msg_subcommand_with_trace(repository_root, subcommand, message_file, logger)
        }
        HookSubcommand::PostCommit {
            vcs_type,
            remote_url,
        } => run_post_commit_subcommand_with_trace(
            repository_root,
            *vcs_type,
            remote_url.as_deref(),
            logger,
        ),
        HookSubcommand::PostRewrite { rewrite_method } => {
            run_post_rewrite_subcommand_with_trace(repository_root, subcommand, rewrite_method)
        }
        HookSubcommand::DiffTrace => Ok(run_diff_trace_subcommand(repository_root, logger)),
        HookSubcommand::ConversationTrace => {
            Ok(run_conversation_trace_subcommand(repository_root, logger))
        }
        HookSubcommand::Codex => Ok(codex::run_codex_subcommand(repository_root, logger)),
        HookSubcommand::ClaudeModelState => Ok(
            claude_model_state::run_claude_model_state_subcommand(repository_root, logger),
        ),
        HookSubcommand::MutationScope => {
            mutation_scope::run_mutation_scope_subcommand(repository_root, logger)
        }
        HookSubcommand::ClaudeMutationScope => {
            claude_mutation_scope::run_claude_mutation_scope_subcommand(logger)
        }
        HookSubcommand::CodexMutationScope => {
            codex_mutation_scope::run_codex_mutation_scope_subcommand(logger)
        }
        HookSubcommand::OpenCodeMutationScope => {
            opencode_mutation_scope::run_opencode_mutation_scope_subcommand(logger)
        }
        HookSubcommand::PiMutationScope => {
            pi_mutation_scope::run_pi_mutation_scope_subcommand(logger)
        }
        HookSubcommand::ExternalMutationGuard => {
            mutation_scope::run_external_mutation_guard_subcommand(repository_root, logger)
        }
    }
}
