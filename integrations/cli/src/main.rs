mod cli;
mod error;
mod runner;

use std::process::ExitCode;

use clap::Parser;
use cli::Args;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), error::HarnessError> {
    let args = Args::parse();
    runner::Runner::new().run(args)
}
