use clap::Parser;

/// Standalone command-contract integration runner for `sce` CLI surfaces.
#[derive(Debug, Parser)]
#[command(name = "cli-integration-tests")]
pub(crate) struct Args {
    /// Run a single command suite (for example: nix run .#cli-integration-tests -- --command version).
    #[arg(long = "command", value_name = "name")]
    pub(crate) command: Option<String>,
}
