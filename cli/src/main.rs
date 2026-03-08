mod app;
mod cli_schema;
mod command_surface;
mod dependency_contract;
mod services;
#[cfg(test)]
mod test_support;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run(std::env::args())
}
