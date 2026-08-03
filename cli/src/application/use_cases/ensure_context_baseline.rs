//! `EnsureContextBaseline` use case: ensures the durable-context baseline
//! exists in a repository, delegating persistence to an injected
//! `ContextStore`.

use std::path::PathBuf;

use crate::application::ports::context_store::{ContextBaselineChanges, ContextStore};
use crate::domain::context::baseline::ContextBaseline;

/// The repository root to ensure the durable-context baseline against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnsureContextBaselineRequest {
    pub(crate) repository_root: PathBuf,
}

/// The outcome of ensuring the durable-context baseline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnsureContextBaselineReport {
    pub(crate) repository_root: PathBuf,
    pub(crate) changes: ContextBaselineChanges,
}

/// Ensures the SCE-canonical durable-context baseline exists in a
/// repository, via an injected `ContextStore`.
pub(crate) struct EnsureContextBaseline<S: ContextStore> {
    store: S,
}

impl<S: ContextStore> EnsureContextBaseline<S> {
    pub(crate) fn new(store: S) -> Self {
        Self { store }
    }

    pub(crate) fn execute(
        &self,
        request: EnsureContextBaselineRequest,
    ) -> Result<EnsureContextBaselineReport, S::Error> {
        let baseline = ContextBaseline::sce_default();
        let changes = self
            .store
            .ensure_baseline(&request.repository_root, &baseline)?;

        Ok(EnsureContextBaselineReport {
            repository_root: request.repository_root,
            changes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::Path;

    #[derive(Default)]
    struct FakeContextStore {
        calls: RefCell<Vec<(PathBuf, ContextBaseline)>>,
    }

    impl ContextStore for FakeContextStore {
        type Error = ();

        fn ensure_baseline(
            &self,
            repository_root: &Path,
            baseline: &ContextBaseline,
        ) -> Result<ContextBaselineChanges, Self::Error> {
            self.calls
                .borrow_mut()
                .push((repository_root.to_path_buf(), baseline.clone()));
            Ok(ContextBaselineChanges::default())
        }
    }

    #[test]
    fn execute_calls_ensure_baseline_with_resolved_root_and_default_baseline() {
        let store = FakeContextStore::default();
        let use_case = EnsureContextBaseline::new(store);
        let repository_root = PathBuf::from("/repo");

        let report = use_case
            .execute(EnsureContextBaselineRequest {
                repository_root: repository_root.clone(),
            })
            .unwrap();

        assert_eq!(report.repository_root, repository_root);
        assert_eq!(report.changes, ContextBaselineChanges::default());

        let calls = use_case.store.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, repository_root);
        assert_eq!(calls[0].1, ContextBaseline::sce_default());
    }
}
