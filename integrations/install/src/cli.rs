use clap::Parser;

use crate::channels::Channel;

/// Opt-in install-channel integration runner for `sce`.
///
/// The npm and Bun channels now perform real install-and-verify flows through the
/// Rust runner, while Cargo remains a shared-harness smoke path until a later task.
#[derive(Parser, Debug)]
#[command(name = "install-channel-integration-tests")]
pub(crate) struct Args {
    /// Channel selector: npm, bun, cargo, or all (default: all)
    #[arg(short, long, value_enum, default_value = "all")]
    pub(crate) channel: ChannelArg,
}

/// Channel selector for integration tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum ChannelArg {
    Npm,
    Bun,
    Cargo,
    All,
}

impl From<ChannelArg> for Vec<Channel> {
    fn from(arg: ChannelArg) -> Self {
        match arg {
            ChannelArg::Npm => vec![Channel::Npm],
            ChannelArg::Bun => vec![Channel::Bun],
            ChannelArg::Cargo => vec![Channel::Cargo],
            ChannelArg::All => vec![Channel::Npm, Channel::Bun, Channel::Cargo],
        }
    }
}
