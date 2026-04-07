mod bun;
mod cargo;
mod npm;

use crate::harness::{HarnessMode, HarnessRequest};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Channel {
    Npm,
    Bun,
    Cargo,
}

impl Channel {
    pub(crate) fn from_selector(selector: &str) -> Result<Vec<Self>, String> {
        match selector {
            "npm" => Ok(vec![Self::Npm]),
            "bun" => Ok(vec![Self::Bun]),
            "cargo" => Ok(vec![Self::Cargo]),
            "all" => Ok(vec![Self::Npm, Self::Bun, Self::Cargo]),
            other => Err(format!(
                "Unsupported channel selector: {other}\nTry --channel npm, --channel bun, --channel cargo, or --channel all."
            )),
        }
    }

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
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn run(&self, channels: &[Channel]) -> Result<(), String> {
        for channel in channels {
            let request = HarnessRequest::new(*channel, HarnessMode::SharedHarnessSmoke);
            match channel {
                Channel::Npm => npm::run(request),
                Channel::Bun => bun::run(request),
                Channel::Cargo => cargo::run(request),
            }?;
        }

        Ok(())
    }
}
