mod adapters;
mod app;
mod application;
mod cli_schema;
mod command_surface;
mod composition;
mod domain;
#[allow(dead_code)]
mod generated_migrations {
    include!(concat!(env!("OUT_DIR"), "/generated_migrations.rs"));
}
mod services;

use std::process::ExitCode;

fn main() -> ExitCode {
    composition::run(std::env::args())
}
