use clap::Parser;

/// Standalone command-contract integration runner for `sce` CLI surfaces.
#[derive(Debug, Parser)]
#[command(name = "cli-integration-tests")]
pub(crate) struct Args {}
