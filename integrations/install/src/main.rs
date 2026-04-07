mod channels;
mod cli;
mod harness;

use std::process::ExitCode;

use channels::ChannelRunner;
use cli::{parse_args, Command};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    match parse_args(std::env::args().skip(1))? {
        Command::Help(help_text) => {
            println!("{help_text}");
            Ok(())
        }
        Command::Run { channels } => {
            let runner = ChannelRunner::new();
            runner.run(&channels)
        }
    }
}
