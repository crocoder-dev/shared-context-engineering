mod app;
mod cli_schema;
mod command_surface;
mod services;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run(std::env::args())
}
