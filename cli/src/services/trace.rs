use anyhow::{Context, Result};

use crate::services::local_db;

pub const NAME: &str = "trace";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceSubcommand {
    AppendPrompt { prompt: String },
}

pub fn run_trace_subcommand(subcommand: TraceSubcommand) -> Result<String> {
    match subcommand {
        TraceSubcommand::AppendPrompt { prompt } => run_append_prompt_subcommand(&prompt),
    }
}

fn run_append_prompt_subcommand(prompt: &str) -> Result<String> {
    let repository_root = std::env::current_dir()
        .context("Failed to determine current directory for trace runtime invocation.")?;

    local_db::append_prompt_with_auto_init(&repository_root, prompt)
        .context("Failed to persist submitted prompt to local Agent Trace DB.")?;

    Ok(String::from(
        "trace append-prompt persisted submitted prompt to local Agent Trace DB.",
    ))
}
