use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum HarnessError {
    #[error("failed to create temp directory: {0}")]
    TempDirCreate(String),

    #[error("failed to create {path}: {error}")]
    DirectoryCreate { path: PathBuf, error: String },

    #[error("failed to copy directory from {src} to {dest}: {error}")]
    DirectoryCopy {
        src: PathBuf,
        dest: PathBuf,
        error: String,
    },

    #[error("Unable to resolve executable '{program}' for channel={channel}. Set {env} or ensure it is on PATH.")]
    ExecutableResolve {
        program: String,
        channel: String,
        env: String,
    },

    #[error("[FAIL] channel={channel} expected executable not found: {path} ({reason})")]
    ExecutableNotFound {
        channel: String,
        path: PathBuf,
        reason: String,
    },

    #[error("[FAIL] channel={channel} sce version failed via {path}")]
    SceVersionFailed {
        channel: String,
        path: PathBuf,
        stderr: Option<String>,
    },

    #[error("[FAIL] channel={channel} unexpected sce version output: {output}")]
    SceVersionUnexpected { channel: String, output: String },

    #[error("[FAIL] channel={channel} expected empty stderr for sce version.\n{stderr}")]
    SceVersionStderr { channel: String, stderr: String },

    #[error("[FAIL] channel={channel} failed to run {program}: {error}")]
    CommandFailed {
        channel: String,
        program: String,
        error: String,
    },

    #[error("failed to inspect {path}: {error}")]
    FileInspect { path: PathBuf, error: String },

    #[error("failed to set executable permissions on {path}: {error}")]
    PermissionSet { path: PathBuf, error: String },

    #[error("failed to read {path}: {error}")]
    FileRead { path: PathBuf, error: String },

    #[error("failed to write {path}: {error}")]
    FileWrite { path: PathBuf, error: String },

    #[error("[FAIL] channel={channel} npm install failed for {tarball}")]
    NpmInstallFailed {
        channel: String,
        tarball: PathBuf,
        stdout: Option<String>,
        stderr: Option<String>,
    },

    #[error("[FAIL] channel={channel} npm pack failed for local fixture")]
    NpmPackFailed {
        channel: String,
        stdout: Option<String>,
        stderr: Option<String>,
    },

    #[error("[FAIL] channel={channel} npm pack did not report a tarball name.")]
    NpmPackNoTarball { channel: String },

    #[error("[FAIL] channel={channel} expected packed tarball was not created: {path}")]
    NpmPackTarballMissing { channel: String, path: PathBuf },

    #[error("[FAIL] channel={channel} bun global install failed for {tarball}")]
    BunInstallFailed {
        channel: String,
        tarball: PathBuf,
        stdout: Option<String>,
        stderr: Option<String>,
    },

    #[error("[FAIL] channel={channel} cargo install failed for {path}")]
    CargoInstallFailed {
        channel: String,
        path: PathBuf,
        stdout: Option<String>,
        stderr: Option<String>,
    },

    #[error("failed to inject runtime/ into staged package manifest {path}")]
    ManifestInject { path: PathBuf },

    #[error("failed to stage {binary} into {path}: {error}")]
    BinaryStage {
        binary: PathBuf,
        path: PathBuf,
        error: String,
    },

    #[error("failed to resolve current directory: {error}")]
    CurrentDir { error: String },

    #[error("[FAIL] channel={channel} could not locate repository root containing flake.nix.")]
    RepoRootMissing { channel: String },

    #[error("executable permissions are only supported on Unix systems")]
    UnixOnly,
}
