mod app;
mod command_surface;
mod dependency_contract;
mod services;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run(std::env::args())
}
