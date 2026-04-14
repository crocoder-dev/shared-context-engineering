use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum HarnessError {
    #[error("missing required environment variable: {env}")]
    MissingEnv { env: &'static str },

    #[error("failed to run command '{program}': {error}")]
    CommandRunFailed { program: String, error: String },

    #[error("[FAIL] sce --help exited with status {status}\nstdout: {stdout}\nstderr: {stderr}")]
    HelpCommandNonZero {
        status: i32,
        stdout: String,
        stderr: String,
    },
}
