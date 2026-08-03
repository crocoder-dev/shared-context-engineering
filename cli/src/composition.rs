//! Composition root: wires runtime dependencies for the CLI binary.
//!
//! `run` is the sole entrypoint called by `main`. It currently delegates to
//! the legacy `app::run` runtime unchanged; ownership of dependency wiring
//! moves here incrementally as commands migrate onto the hexagonal layers.

use std::process::ExitCode;

use crate::app;

pub(crate) fn run<I>(args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    app::run(args)
}
