//! The canonical durable-context baseline: the directory/file manifest
//! `sce setup` bootstraps additively into a repository.

use std::path::PathBuf;

/// A baseline file within the context tree: its repository-relative path
/// and the content it is created with when the file does not yet exist.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BaselineFile {
    pub(crate) relative_path: PathBuf,
    pub(crate) initial_content: &'static str,
}

/// The canonical set of durable-context directories and files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextBaseline {
    pub(crate) directories: Vec<PathBuf>,
    pub(crate) files: Vec<BaselineFile>,
}

const CONTEXT_TMP_GITIGNORE_CONTENT: &str = "*\n!.gitignore\n";

const CONTEXT_OVERVIEW_TEMPLATE: &str = "# Overview\n\n";
const CONTEXT_ARCHITECTURE_TEMPLATE: &str = "# Architecture\n\n";
const CONTEXT_PATTERNS_TEMPLATE: &str = "# Patterns\n\n";
const CONTEXT_GLOSSARY_TEMPLATE: &str = "# Glossary\n\n";
const CONTEXT_MAP_TEMPLATE: &str = "\
# Context Map

Primary context files:

- `context/overview.md`
- `context/architecture.md`
- `context/patterns.md`
- `context/glossary.md`

Working areas:

- `context/plans/`
- `context/handovers/`
- `context/decisions/`
- `context/tmp/`
";

impl ContextBaseline {
    /// The SCE-canonical durable-context baseline.
    pub(crate) fn sce_default() -> Self {
        Self {
            directories: vec![
                PathBuf::from("context"),
                PathBuf::from("context/plans"),
                PathBuf::from("context/handovers"),
                PathBuf::from("context/decisions"),
                PathBuf::from("context/tmp"),
            ],
            files: vec![
                BaselineFile {
                    relative_path: PathBuf::from("context/overview.md"),
                    initial_content: CONTEXT_OVERVIEW_TEMPLATE,
                },
                BaselineFile {
                    relative_path: PathBuf::from("context/architecture.md"),
                    initial_content: CONTEXT_ARCHITECTURE_TEMPLATE,
                },
                BaselineFile {
                    relative_path: PathBuf::from("context/patterns.md"),
                    initial_content: CONTEXT_PATTERNS_TEMPLATE,
                },
                BaselineFile {
                    relative_path: PathBuf::from("context/glossary.md"),
                    initial_content: CONTEXT_GLOSSARY_TEMPLATE,
                },
                BaselineFile {
                    relative_path: PathBuf::from("context/context-map.md"),
                    initial_content: CONTEXT_MAP_TEMPLATE,
                },
                BaselineFile {
                    relative_path: PathBuf::from("context/tmp/.gitignore"),
                    initial_content: CONTEXT_TMP_GITIGNORE_CONTENT,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sce_default_has_the_canonical_directories_and_files() {
        let baseline = ContextBaseline::sce_default();

        let directories: Vec<&str> = baseline
            .directories
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect();
        assert_eq!(
            directories,
            vec![
                "context",
                "context/plans",
                "context/handovers",
                "context/decisions",
                "context/tmp",
            ]
        );

        let files: Vec<&str> = baseline
            .files
            .iter()
            .map(|file| file.relative_path.to_str().unwrap())
            .collect();
        assert_eq!(
            files,
            vec![
                "context/overview.md",
                "context/architecture.md",
                "context/patterns.md",
                "context/glossary.md",
                "context/context-map.md",
                "context/tmp/.gitignore",
            ]
        );
    }

    #[test]
    fn sce_default_file_content_matches_legacy_templates() {
        let baseline = ContextBaseline::sce_default();

        let content_for = |relative_path: &str| {
            baseline
                .files
                .iter()
                .find(|file| file.relative_path.to_str().unwrap() == relative_path)
                .unwrap()
                .initial_content
        };

        assert_eq!(content_for("context/overview.md"), "# Overview\n\n");
        assert_eq!(content_for("context/architecture.md"), "# Architecture\n\n");
        assert_eq!(content_for("context/patterns.md"), "# Patterns\n\n");
        assert_eq!(content_for("context/glossary.md"), "# Glossary\n\n");
        assert_eq!(content_for("context/tmp/.gitignore"), "*\n!.gitignore\n");
        assert!(content_for("context/context-map.md").starts_with("# Context Map"));
    }
}
