pub mod command;
pub mod lifecycle;
pub mod policy;
pub mod resolver;
pub mod schema;
pub mod types;

mod render;

pub use types::*;

use anyhow::{Context, Result};
use render::{format_show_output, format_validate_output};
use resolver::resolve_runtime_config;

pub(crate) use resolver::{
    resolve_auth_runtime_config, resolve_hook_runtime_config, resolve_observability_runtime_config,
};
pub(crate) use schema::validate_config_file;

pub fn run_config_subcommand(subcommand: ConfigSubcommand) -> Result<String> {
    match subcommand {
        ConfigSubcommand::Show(request) => {
            let cwd = std::env::current_dir().context("Failed to determine current directory")?;
            let runtime = resolve_runtime_config(&request, &cwd)?;
            Ok(format_show_output(&runtime, request.report_format))
        }
        ConfigSubcommand::Validate(request) => {
            let cwd = std::env::current_dir().context("Failed to determine current directory")?;
            let runtime = resolve_runtime_config(&request, &cwd)?;
            Ok(format_validate_output(&runtime, request.report_format))
        }
    }
}
