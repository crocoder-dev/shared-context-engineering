mod channels;
mod cli;
mod error;
mod harness;
mod platform;

use std::process::ExitCode;

use channels::ChannelRunner;
use clap::Parser;
use cli::Args;
use error::HarnessError;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), HarnessError> {
    let args = Args::parse();
    let channels = Vec::from(args.channel);
    let runner = ChannelRunner;
    runner.run(&channels, args.repo_root.as_deref())
}
