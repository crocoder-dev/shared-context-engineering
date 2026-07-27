mod app;
mod cli_schema;
mod command_surface;
#[allow(dead_code)]
mod generated_migrations {
    include!(concat!(env!("OUT_DIR"), "/generated_migrations.rs"));
}
mod services;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run(std::env::args())
}
