//! Encrypted auth Turso database adapter.

use std::path::PathBuf;

use anyhow::Result;

use crate::{
    generated_migrations,
    services::{
        db::{DbSpec, EncryptedTursoDb},
        default_paths::auth_db_path,
    },
};

/// Encrypted auth database configuration.
pub struct AuthDbSpec;

impl DbSpec for AuthDbSpec {
    fn db_name() -> &'static str {
        "auth DB"
    }

    fn db_path() -> Result<PathBuf> {
        auth_db_path()
    }

    fn migrations() -> &'static [(&'static str, &'static str)] {
        generated_migrations::AUTH_MIGRATIONS
    }

    fn db_config_key() -> &'static str {
        "auth_db"
    }
}

/// Encrypted auth Turso database adapter.
pub type AuthDb = EncryptedTursoDb<AuthDbSpec>;

pub mod lifecycle;

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        path::PathBuf,
        sync::OnceLock,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::services::{
        db::{DbSpec, TursoDb},
        lifecycle::{lifecycle_providers, LifecycleProviderId},
    };
    use anyhow::Context;

    static TEST_DB_PATH: OnceLock<PathBuf> = OnceLock::new();

    struct TestAuthDbSpec;

    impl DbSpec for TestAuthDbSpec {
        fn db_name() -> &'static str {
            "test auth DB"
        }

        fn db_path() -> Result<PathBuf> {
            TEST_DB_PATH
                .get()
                .cloned()
                .context("test DB path should be initialized")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            generated_migrations::AUTH_MIGRATIONS
        }

        fn db_config_key() -> &'static str {
            "auth_db"
        }
    }

    fn unique_test_db_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("sce-auth-db-test-{}-{nonce}", std::process::id()))
            .join("auth.db")
    }

    fn sqlite_object_exists<M: DbSpec>(db: &TursoDb<M>, object_type: &str, name: &str) -> bool {
        let rows: Vec<String> = db
            .query_map(
                "SELECT name FROM sqlite_master WHERE type = ?1 AND name = ?2",
                (object_type, name),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .expect("sqlite_master query should succeed");
        !rows.is_empty()
    }

    fn applied_migration_ids<M: DbSpec>(db: &TursoDb<M>) -> Vec<String> {
        db.query_map(
            "SELECT id FROM __sce_migrations ORDER BY id ASC",
            (),
            |row| row.get::<String>(0).map_err(anyhow::Error::from),
        )
        .expect("migration metadata query should succeed")
    }

    #[test]
    fn auth_db_baseline_migration_creates_table_index_and_constraints() {
        let db_path = unique_test_db_path();
        TEST_DB_PATH
            .set(db_path.clone())
            .expect("test DB path should only be initialized once");

        let db = TursoDb::<TestAuthDbSpec>::new().expect("test auth DB should open");

        // Verify table, trigger, and migration IDs
        assert!(sqlite_object_exists(&db, "table", "auth_credentials"));
        assert!(sqlite_object_exists(
            &db,
            "trigger",
            "auth_credentials_set_updated_at"
        ));

        // Verify migration IDs are ordered
        let expected_migration_ids: Vec<String> = generated_migrations::AUTH_MIGRATIONS
            .iter()
            .map(|(id, _)| (*id).to_owned())
            .collect();
        assert_eq!(applied_migration_ids(&db), expected_migration_ids);

        // Verify column NOT NULL constraints via PRAGMA table_info
        // Returns: cid, name, type, notnull, dflt_value, pk
        let columns: Vec<(String, i32)> = db
            .query_map(
                "SELECT name, \"notnull\" FROM pragma_table_info('auth_credentials') ORDER BY cid",
                (),
                |row| {
                    let name: String = row.get::<String>(0)?;
                    let notnull: i32 = row.get::<i32>(1)?;
                    Ok((name, notnull))
                },
            )
            .expect("pragma table_info should succeed");

        let col_map: HashMap<String, i32> = columns.into_iter().collect();

        // Required columns must be NOT NULL
        for col in &[
            "id",
            "access_token",
            "token_type",
            "expires_in",
            "refresh_token",
            "stored_at_unix_seconds",
            "created_at",
            "updated_at",
        ] {
            assert_eq!(
                col_map.get(*col),
                Some(&1),
                "column '{col}' should be NOT NULL"
            );
        }

        // scope must allow NULL
        assert_eq!(
            col_map.get("scope"),
            Some(&0),
            "column 'scope' should allow NULL"
        );

        if let Some(parent) = db_path.parent() {
            fs::remove_dir_all(parent).expect("test DB directory should be removed");
        }
    }

    #[test]
    fn auth_db_lifecycle_provider_included() {
        let providers = lifecycle_providers(false);

        let auth_count = providers
            .iter()
            .filter(|p| p.id() == LifecycleProviderId::AuthDb)
            .count();
        assert_eq!(
            auth_count, 1,
            "AuthDb lifecycle provider should be registered exactly once"
        );

        // Verify deterministic order: Config -> LocalDb -> AuthDb -> AgentTraceDb
        let provider_ids: Vec<LifecycleProviderId> = providers.iter().map(|p| p.id()).collect();
        assert_eq!(
            provider_ids,
            vec![
                LifecycleProviderId::Config,
                LifecycleProviderId::LocalDb,
                LifecycleProviderId::AuthDb,
                LifecycleProviderId::AgentTraceDb,
            ],
            "lifecycle provider order should be deterministic: Config -> LocalDb -> AuthDb -> AgentTraceDb"
        );
    }
}
