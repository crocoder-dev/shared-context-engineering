mod bun;
mod cargo;
mod npm;

use std::path::Path;

use crate::error::HarnessError;
use crate::harness::{HarnessMode, HarnessRequest};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Channel {
    Npm,
    Bun,
    Cargo,
}

impl Channel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Bun => "bun",
            Self::Cargo => "cargo",
        }
    }
}

pub(crate) struct ChannelRunner;

impl ChannelRunner {
    pub(crate) fn run(
        &self,
        channels: &[Channel],
        repo_root: Option<&Path>,
    ) -> Result<(), HarnessError> {
        for channel in channels {
            let request = HarnessRequest::new(*channel, HarnessMode::SharedHarnessSmoke);
            match channel {
                Channel::Npm => npm::run(request, repo_root)?,
                Channel::Bun => bun::run(request, repo_root)?,
                Channel::Cargo => cargo::run(request, repo_root)?,
            };
        }

        Ok(())
    }
}
