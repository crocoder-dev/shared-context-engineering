use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum HarnessError {
    #[error("missing required environment variable: {env}")]
    MissingEnv { env: &'static str },

    #[error("failed to run command '{program}': {error}")]
    CommandRunFailed { program: String, error: String },

    #[error(
        "unknown command suite '{selected}'. Choose one of: {available}. Run '--help' for usage."
    )]
    UnknownCommandSelector { selected: String, available: String },

    #[error("[FAIL] case '{case}' failed: {reason}\ncommand: {command}\nstatus: {status}\nstdout: {stdout}\nstderr: {stderr}")]
    AssertionFailed {
        case: &'static str,
        reason: String,
        command: String,
        status: i32,
        stdout: String,
        stderr: String,
    },
}
