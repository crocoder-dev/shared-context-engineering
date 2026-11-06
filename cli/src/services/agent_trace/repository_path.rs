use std::path::{Component, Path};

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

    pub(crate) fn repository_path(
        &self,
        path: &str,
        base_dir: Option<&str>,
    ) -> Option<RepositoryPath> {
        if path.is_empty() {
            return None;
        }
        let path = Path::new(path);
        if path.is_absolute() {
            return self.strip_root(&absolute_components(path)?);
        }
        match base_dir.map(Path::new).filter(|base| base.is_absolute()) {
            Some(base) => self.strip_root(&absolute_components(&base.join(path))?),
            None => {
                relative_components(path).map(|components| RepositoryPath(components.join("/")))
            }
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

    const ROOT: &str = "/workspace/project";

    fn identity(path: &str, base_dir: Option<&str>) -> Option<String> {
        RepositoryRoot::new(Path::new(ROOT))
            .repository_path(path, base_dir)
            .map(|path| path.as_str().to_owned())
    }

    #[test]
    fn relative_paths_normalize_to_exact_repository_identity() {
        assert_eq!(identity("src/lib.rs", None).as_deref(), Some("src/lib.rs"));
        assert_eq!(
            identity("./src/../src/lib.rs", None).as_deref(),
            Some("src/lib.rs")
        );
        assert_ne!(
            identity("src/lib.rs", None),
            identity("packages/foo/src/lib.rs", None)
        );
    }

    #[test]
    fn absolute_paths_inside_the_repository_become_repository_relative() {
        assert_eq!(
            identity("/workspace/project/src/lib.rs", None).as_deref(),
            Some("src/lib.rs")
        );
        assert_eq!(
            identity("/workspace/project/./src/../src/lib.rs", None).as_deref(),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn absolute_paths_outside_the_repository_are_rejected() {
        assert_eq!(
            identity("/workspace/another-project/src/lib.rs", None),
            None
        );
        assert_eq!(identity("/workspace/project-two/src/lib.rs", None), None);
        assert_eq!(identity("/workspace/project", None), None);
        assert_eq!(identity("/src/lib.rs", None), None);
    }

    #[test]
    fn traversal_cannot_escape_the_repository() {
        assert_eq!(identity("../project/src/lib.rs", None), None);
        assert_eq!(identity("src/../../lib.rs", None), None);
        assert_eq!(
            identity("/workspace/project/../another-project/src/lib.rs", None),
            None
        );
        assert_eq!(identity("../src/lib.rs", Some("/workspace/project")), None);
        assert_eq!(identity("", None), None);
    }

    #[test]
    fn relative_paths_resolve_against_an_absolute_base_directory() {
        assert_eq!(
            identity("src/lib.rs", Some("/workspace/project/cli")).as_deref(),
            Some("cli/src/lib.rs")
        );
        assert_eq!(
            identity("src/lib.rs", Some("/workspace/project")).as_deref(),
            Some("src/lib.rs")
        );
        assert_eq!(
            identity("../cli/src/lib.rs", Some("/workspace/project/config")).as_deref(),
            Some("cli/src/lib.rs")
        );
        assert_eq!(identity("src/lib.rs", Some("/workspace/another")), None);
        assert_eq!(
            identity("src/lib.rs", Some("relative/base")).as_deref(),
            Some("src/lib.rs")
        );
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
                root.repository_path(spelling.to_str().expect("utf-8 path"), None)
                    .map(|path| path.as_str().to_owned())
                    .as_deref(),
                Some("src/lib.rs")
            );
        }
    }
}
