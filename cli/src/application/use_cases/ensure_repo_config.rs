//! `EnsureRepoConfig` use case: ensures the repo-local integration
//! configuration file exists, delegating persistence to an injected
//! `IntegrationConfigRepository`.

use std::path::PathBuf;

use crate::application::ports::integration_config_repository::IntegrationConfigRepository;

/// The repository root to ensure repo-local integration configuration
/// against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnsureRepoConfigRequest {
    pub(crate) repository_root: PathBuf,
}

/// Ensures the repo-local integration configuration file exists, via an
/// injected `IntegrationConfigRepository`.
pub(crate) struct EnsureRepoConfig<R: IntegrationConfigRepository> {
    repository: R,
}

impl<R: IntegrationConfigRepository> EnsureRepoConfig<R> {
    pub(crate) fn new(repository: R) -> Self {
        Self { repository }
    }

    pub(crate) fn execute(&self, request: &EnsureRepoConfigRequest) -> Result<(), R::Error> {
        self.repository.ensure_exists(&request.repository_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::Path;

    use crate::domain::integration::IntegrationTarget;

    #[derive(Default)]
    struct FakeRepository {
        ensure_exists_calls: RefCell<Vec<PathBuf>>,
        ensure_exists_error: Option<&'static str>,
    }

    impl IntegrationConfigRepository for FakeRepository {
        type Error = &'static str;

        fn ensure_exists(&self, repository_root: &Path) -> Result<(), Self::Error> {
            self.ensure_exists_calls
                .borrow_mut()
                .push(repository_root.to_path_buf());

            self.ensure_exists_error.map_or(Ok(()), Err)
        }

        fn load_optional_workflows(
            &self,
            _repository_root: &Path,
        ) -> Result<Vec<String>, Self::Error> {
            unreachable!("not exercised by EnsureRepoConfig")
        }

        fn record_installation(
            &self,
            _repository_root: &Path,
            _targets: &[IntegrationTarget],
            _optional_workflows: &[String],
        ) -> Result<(), Self::Error> {
            unreachable!("not exercised by EnsureRepoConfig")
        }
    }

    #[test]
    fn execute_delegates_to_ensure_exists_with_the_resolved_root() {
        let repository = FakeRepository::default();
        let use_case = EnsureRepoConfig::new(repository);
        let repository_root = PathBuf::from("/repo");

        use_case
            .execute(&EnsureRepoConfigRequest {
                repository_root: repository_root.clone(),
            })
            .unwrap();

        assert_eq!(
            use_case.repository.ensure_exists_calls.borrow().as_slice(),
            [repository_root]
        );
    }

    #[test]
    fn execute_propagates_repository_errors() {
        let repository = FakeRepository {
            ensure_exists_calls: RefCell::new(Vec::new()),
            ensure_exists_error: Some("ensure_exists failed"),
        };
        let use_case = EnsureRepoConfig::new(repository);

        let result = use_case.execute(&EnsureRepoConfigRequest {
            repository_root: PathBuf::from("/repo"),
        });

        assert_eq!(result, Err("ensure_exists failed"));
    }
}
