use std::path::{Component, Path};

use crate::services::agent_trace_db::DiffTracePathOrigin;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RepositoryPath(String);

impl RepositoryPath {
    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryRoot {
    roots: Vec<Vec<String>>,
}

impl RepositoryRoot {
    pub fn new(root: &Path) -> Self {
        RepositoryRoot {
            roots: absolute_components(root).into_iter().collect(),
        }
    }

    pub fn resolve(root: &Path) -> Self {
        let mut repository_root = Self::new(root);
        if let Some(canonical) = root
            .canonicalize()
            .ok()
            .and_then(|canonical| absolute_components(&canonical))
        {
            if !repository_root.roots.contains(&canonical) {
                repository_root.roots.push(canonical);
            }
        }
        repository_root
    }

    /// Resolves `path` to its repository identity. Absolute paths stand on
    /// their own; a relative path is interpreted only in the coordinate system
    /// `origin` establishes, and is rejected when there is none.
    pub(crate) fn repository_path(
        &self,
        path: &str,
        origin: &DiffTracePathOrigin,
    ) -> Option<RepositoryPath> {
        if path.is_empty() {
            return None;
        }
        let path = Path::new(path);
        if path.is_absolute() {
            return self.strip_root(&absolute_components(path)?);
        }
        match origin {
            DiffTracePathOrigin::RepositoryRelative => {
                relative_components(path).map(|components| RepositoryPath(components.join("/")))
            }
            DiffTracePathOrigin::WorkingDirectory(cwd) => {
                self.strip_root(&absolute_components(&Path::new(cwd).join(path))?)
            }
            DiffTracePathOrigin::AbsoluteOnly => None,
        }
    }

    fn strip_root(&self, components: &[String]) -> Option<RepositoryPath> {
        self.roots.iter().find_map(|root| {
            (components.len() > root.len() && components.starts_with(root))
                .then(|| RepositoryPath(components[root.len()..].join("/")))
        })
    }
}

fn absolute_components(path: &Path) -> Option<Vec<String>> {
    if !path.is_absolute() {
        return None;
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => components.push(prefix.as_os_str().to_str()?.to_owned()),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                components.pop()?;
            }
            Component::Normal(part) => components.push(part.to_str()?.to_owned()),
        }
    }
    Some(components)
}

fn relative_components(path: &Path) -> Option<Vec<String>> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                components.pop()?;
            }
            Component::Normal(part) => components.push(part.to_str()?.to_owned()),
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    (!components.is_empty()).then_some(components)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::RepositoryRoot;
    use crate::services::agent_trace_db::DiffTracePathOrigin;

    const ROOT: &str = "/workspace/project";

    fn resolve(path: &str, origin: &DiffTracePathOrigin) -> Option<String> {
        RepositoryRoot::new(Path::new(ROOT))
            .repository_path(path, origin)
            .map(|path| path.as_str().to_owned())
    }

    fn identity(path: &str) -> Option<String> {
        resolve(path, &DiffTracePathOrigin::RepositoryRelative)
    }

    fn in_cwd(path: &str, cwd: &str) -> Option<String> {
        resolve(path, &DiffTracePathOrigin::WorkingDirectory(cwd.to_owned()))
    }

    fn absolute_only(path: &str) -> Option<String> {
        resolve(path, &DiffTracePathOrigin::AbsoluteOnly)
    }

    #[test]
    fn relative_paths_normalize_to_exact_repository_identity() {
        assert_eq!(identity("src/lib.rs").as_deref(), Some("src/lib.rs"));
        assert_eq!(
            identity("./src/../src/lib.rs").as_deref(),
            Some("src/lib.rs")
        );
        assert_ne!(identity("src/lib.rs"), identity("packages/foo/src/lib.rs"));
    }

    #[test]
    fn absolute_paths_inside_the_repository_become_repository_relative() {
        for resolve_absolute in [identity, absolute_only] {
            assert_eq!(
                resolve_absolute("/workspace/project/src/lib.rs").as_deref(),
                Some("src/lib.rs")
            );
            assert_eq!(
                resolve_absolute("/workspace/project/./src/../src/lib.rs").as_deref(),
                Some("src/lib.rs")
            );
        }
        assert_eq!(
            in_cwd("/workspace/project/src/lib.rs", "/elsewhere").as_deref(),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn absolute_paths_outside_the_repository_are_rejected() {
        for resolve_absolute in [identity, absolute_only] {
            assert_eq!(
                resolve_absolute("/workspace/another-project/src/lib.rs"),
                None
            );
            assert_eq!(resolve_absolute("/workspace/project-two/src/lib.rs"), None);
            assert_eq!(resolve_absolute("/workspace/project"), None);
            assert_eq!(resolve_absolute("/src/lib.rs"), None);
        }
        assert_eq!(in_cwd("/workspace/another-project/src/lib.rs", ROOT), None);
    }

    #[test]
    fn traversal_cannot_escape_the_repository() {
        assert_eq!(identity("../project/src/lib.rs"), None);
        assert_eq!(identity("src/../../lib.rs"), None);
        assert_eq!(
            identity("/workspace/project/../another-project/src/lib.rs"),
            None
        );
        assert_eq!(in_cwd("../src/lib.rs", ROOT), None);
        assert_eq!(
            in_cwd("../../other/src/lib.rs", "/workspace/project/cli"),
            None
        );
        assert_eq!(
            in_cwd("../../../../../src/lib.rs", "/workspace/project/cli"),
            None
        );
        assert_eq!(identity(""), None);
    }

    #[test]
    fn relative_paths_resolve_against_an_absolute_working_directory() {
        assert_eq!(
            in_cwd("src/lib.rs", "/workspace/project/cli").as_deref(),
            Some("cli/src/lib.rs")
        );
        assert_eq!(in_cwd("src/lib.rs", ROOT).as_deref(), Some("src/lib.rs"));
        assert_eq!(
            in_cwd("../cli/src/lib.rs", "/workspace/project/config").as_deref(),
            Some("cli/src/lib.rs")
        );
        assert_eq!(in_cwd("src/lib.rs", "/workspace/another"), None);
    }

    #[test]
    fn relative_paths_without_an_established_coordinate_system_are_rejected() {
        assert_eq!(absolute_only("src/lib.rs"), None);
        assert_eq!(absolute_only("./src/lib.rs"), None);
        assert_eq!(in_cwd("src/lib.rs", "relative/base"), None);
        assert_eq!(in_cwd("src/lib.rs", ""), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_checkout_spelling_resolves_through_the_canonical_root() {
        let real = tempfile::tempdir().expect("real root");
        let links = tempfile::tempdir().expect("link dir");
        let link = links.path().join("checkout");
        std::os::unix::fs::symlink(real.path(), &link).expect("symlink checkout");

        let root = RepositoryRoot::resolve(&link);
        let canonical = real.path().canonicalize().expect("canonical root");
        for spelling in [link.join("src/lib.rs"), canonical.join("src/lib.rs")] {
            assert_eq!(
                root.repository_path(
                    spelling.to_str().expect("utf-8 path"),
                    &DiffTracePathOrigin::AbsoluteOnly
                )
                .map(|path| path.as_str().to_owned())
                .as_deref(),
                Some("src/lib.rs")
            );
        }
    }
}
