use clap::Parser;

/// Standalone command-contract integration runner for `sce` CLI surfaces.
#[derive(Debug, Parser)]
#[command(name = "cli-integration-tests")]
pub(crate) struct Args {
    /// Run a single command suite (for example: --command help).
    #[arg(long = "command", value_name = "name")]
    pub(crate) command: Option<String>,
}
