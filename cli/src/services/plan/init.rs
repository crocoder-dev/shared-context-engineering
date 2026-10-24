use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context};

use crate::services::error::CliError;
use crate::services::plan::{SCAFFOLD, SCAFFOLD_PLAN_NAME_TOKEN};

pub const PLAN_NAME_MAX_BYTES: usize = 64;
pub const PLANS_RELATIVE_DIR: &str = "context/plans";
const PLAN_EXTENSION: &str = ".md";
const CREATED_PREFIX: &str = "Created plan: ";
const NEXT_LINE: &str = "Next: complete the plan and run";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanName(String);

impl PlanName {
    pub fn parse(value: &str) -> Result<Self, CliError> {
        if value.ends_with(PLAN_EXTENSION) {
            return Err(invalid_name(value, "drop the '.md' suffix"));
        }
        if value.len() > PLAN_NAME_MAX_BYTES {
            return Err(invalid_name(
                value,
                &format!("use at most {PLAN_NAME_MAX_BYTES} bytes"),
            ));
        }
        if !is_kebab_case(value) {
            return Err(invalid_name(
                value,
                "use lowercase letters and digits separated by single hyphens",
            ));
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_kebab_case(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn invalid_name(value: &str, remedy: &str) -> CliError {
    CliError::validation(format!(
        "Invalid plan name '{value}'. Try: {remedy}; plan names match [a-z0-9]+(-[a-z0-9]+)*."
    ))
}

pub fn create(name: &PlanName, start: &Path, stderr: &mut impl Write) -> Result<String, CliError> {
    create_with(
        name,
        start,
        stderr,
        |temporary, destination| std::fs::hard_link(temporary, destination),
        |temporary| std::fs::remove_file(temporary),
    )
}

pub(super) fn create_with(
    name: &PlanName,
    start: &Path,
    stderr: &mut impl Write,
    link: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    remove: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<String, CliError> {
    let root = resolve_repository_root(start)?;
    let context_dir = Path::new(&root).join("context");
    let plans_dir = Path::new(&root).join(PLANS_RELATIVE_DIR);
    ensure_directory(&context_dir)?;
    ensure_directory(&plans_dir)?;

    let file_name = format!("{}{PLAN_EXTENSION}", name.as_str());
    let relative_path = format!("{PLANS_RELATIVE_DIR}/{file_name}");
    let content = SCAFFOLD.replacen(SCAFFOLD_PLAN_NAME_TOKEN, name.as_str(), 1);
    if let Some(warning) = publish(
        &plans_dir,
        &file_name,
        &relative_path,
        &content,
        link,
        remove,
    )? {
        let _ = writeln!(stderr, "{warning}");
    }

    let absolute_path = format!("{root}/{relative_path}");
    Ok([
        format!("{CREATED_PREFIX}{relative_path}"),
        NEXT_LINE.to_string(),
        format!("sce plan check {}", shell_quote(&absolute_path)),
    ]
    .join("\n"))
}

fn resolve_repository_root(start: &Path) -> Result<String, CliError> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .context("Failed to run 'git rev-parse --show-toplevel'. Try: install Git and rerun 'sce plan init'.")
        .map_err(CliError::runtime)?;
    if !output.status.success() {
        return Err(CliError::runtime(anyhow!(
            "Not inside a Git repository: '{}' ({}). Try: run 'sce plan init' inside a Git repository.",
            start.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let stdout = String::from_utf8(output.stdout).map_err(|_| {
        CliError::runtime(anyhow!(
            "Repository root is not valid UTF-8. Try: move the repository to a UTF-8 path."
        ))
    })?;
    let root = stdout.strip_suffix('\n').unwrap_or(&stdout);
    if root.is_empty() || root.contains('\n') {
        return Err(CliError::runtime(anyhow!(
            "Repository root cannot be printed on one line. Try: move the repository to a path without newlines."
        )));
    }
    Ok(root.to_string())
}

fn ensure_directory(path: &Path) -> Result<(), CliError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(not_a_directory(path)),
        Err(error) if error.kind() == ErrorKind::NotFound => match std::fs::create_dir(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                match std::fs::symlink_metadata(path) {
                    Ok(metadata) if metadata.is_dir() => Ok(()),
                    _ => Err(not_a_directory(path)),
                }
            }
            Err(error) => Err(CliError::runtime(
                anyhow!(error).context(format!("Failed to create directory '{}'", path.display())),
            )),
        },
        Err(error) => Err(CliError::runtime(
            anyhow!(error).context(format!("Failed to inspect '{}'", path.display())),
        )),
    }
}

fn not_a_directory(path: &Path) -> CliError {
    CliError::runtime(anyhow!(
        "'{}' is not a directory; symlinked or non-directory entries are refused. Try: replace it with a real directory.",
        path.display()
    ))
}

pub(super) fn temporary_path(plans_dir: &Path, file_name: &str) -> PathBuf {
    plans_dir.join(format!(".{file_name}.{}.tmp", std::process::id()))
}

fn publish(
    plans_dir: &Path,
    file_name: &str,
    relative_path: &str,
    content: &str,
    link: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    remove: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<Option<String>, CliError> {
    let temporary = temporary_path(plans_dir, file_name);
    let destination = plans_dir.join(file_name);

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("Failed to create temporary file '{}'", temporary.display()))
        .map_err(CliError::runtime)?;
    let written = file
        .write_all(content.as_bytes())
        .and_then(|()| file.flush())
        .with_context(|| format!("Failed to write temporary file '{}'", temporary.display()));
    drop(file);

    let published = written.map_err(CliError::runtime).and_then(|()| {
        link(&temporary, &destination).map_err(|error| {
            if error.kind() == ErrorKind::AlreadyExists {
                CliError::validation(format!(
                    "Plan already exists: {relative_path}. Try: choose another plan name; existing plans are never overwritten."
                ))
            } else {
                CliError::runtime(anyhow!(error).context(format!(
                    "Failed to publish '{}'",
                    destination.display()
                )))
            }
        })
    });
    let removed = remove(&temporary);

    match (published, removed) {
        (Ok(()), Ok(())) => Ok(None),
        (Ok(()), Err(error)) => Ok(Some(format!(
            "Warning: Created plan {relative_path}, but failed to remove temporary file '{}': {error}. Try: remove '{}' manually; the plan was created and does not need to be initialized again.",
            temporary.display(),
            temporary.display()
        ))),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => Err(CliError::internal(
            error.class(),
            anyhow!(
                "{error}\nCleanup also failed: could not remove temporary file '{}': {cleanup}. Try: remove it manually.",
                temporary.display()
            ),
        )),
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
