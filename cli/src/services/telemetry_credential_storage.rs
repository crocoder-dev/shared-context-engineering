use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::services::db::{DbSpec, EncryptedTursoDb};
use crate::services::token_storage::{get_auth_db, TokenStorageError};

const TELEMETRY_CREDENTIAL_ROW_ID: i64 = 1;
const REDACTED: &str = "<redacted>";

#[derive(Clone, Eq, PartialEq)]
pub struct TelemetryCredential {
    pub token: String,
    pub accessor: String,
    pub expires_at_unix_seconds: u64,
}

impl fmt::Debug for TelemetryCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelemetryCredential")
            .field("token", &REDACTED)
            .field("accessor", &REDACTED)
            .field("expires_at_unix_seconds", &self.expires_at_unix_seconds)
            .finish()
    }
}

impl TelemetryCredential {
    fn is_valid_at(&self, now_unix_seconds: u64) -> bool {
        !self.token.trim().is_empty()
            && !self.accessor.trim().is_empty()
            && self.expires_at_unix_seconds > now_unix_seconds
    }
}

#[allow(dead_code)]
pub async fn save_telemetry_credential(
    credential: &TelemetryCredential,
) -> Result<(), TokenStorageError> {
    save_with(get_auth_db().await?, credential).await
}

#[allow(dead_code)]
pub async fn load_telemetry_credential() -> Result<Option<TelemetryCredential>, TokenStorageError> {
    load_with(get_auth_db().await?, current_unix_timestamp_seconds()?).await
}

pub async fn delete_telemetry_credential() -> Result<bool, TokenStorageError> {
    delete_with(get_auth_db().await?).await
}

async fn save_with<M: DbSpec>(
    db: &EncryptedTursoDb<M>,
    credential: &TelemetryCredential,
) -> Result<(), TokenStorageError> {
    let expires_at = i64::try_from(credential.expires_at_unix_seconds).map_err(|_| {
        TokenStorageError::Database(
            "telemetry credential expiry is out of range for database storage".to_string(),
        )
    })?;
    let stored_at = i64::try_from(current_unix_timestamp_seconds()?).map_err(|_| {
        TokenStorageError::Database(
            "telemetry credential timestamp is out of range for database storage".to_string(),
        )
    })?;

    db.execute(
        "INSERT OR REPLACE INTO telemetry_credentials \
         (id, token, accessor, expires_at_unix_seconds, stored_at_unix_seconds) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (
            TELEMETRY_CREDENTIAL_ROW_ID,
            credential.token.as_str(),
            credential.accessor.as_str(),
            expires_at,
            stored_at,
        ),
    )
    .await
    .map_err(|_| TokenStorageError::Database("failed to save telemetry credential".to_string()))?;

    Ok(())
}

async fn load_with<M: DbSpec>(
    db: &EncryptedTursoDb<M>,
    now_unix_seconds: u64,
) -> Result<Option<TelemetryCredential>, TokenStorageError> {
    let rows: Vec<Option<TelemetryCredential>> = db
        .query_map(
            "SELECT token, accessor, expires_at_unix_seconds \
             FROM telemetry_credentials WHERE id = ?1",
            (TELEMETRY_CREDENTIAL_ROW_ID,),
            |row| {
                let token: String = row.get(0)?;
                let accessor: String = row.get(1)?;
                let expires_at: i64 = row.get(2)?;
                Ok(u64::try_from(expires_at)
                    .ok()
                    .map(|expires_at_unix_seconds| TelemetryCredential {
                        token,
                        accessor,
                        expires_at_unix_seconds,
                    }))
            },
        )
        .await
        .map_err(|_| {
            TokenStorageError::Database("failed to load telemetry credential".to_string())
        })?;

    Ok(rows
        .into_iter()
        .next()
        .flatten()
        .filter(|credential| credential.is_valid_at(now_unix_seconds)))
}

async fn delete_with<M: DbSpec>(db: &EncryptedTursoDb<M>) -> Result<bool, TokenStorageError> {
    let affected = db
        .execute(
            "DELETE FROM telemetry_credentials WHERE id = ?1",
            (TELEMETRY_CREDENTIAL_ROW_ID,),
        )
        .await
        .map_err(|_| {
            TokenStorageError::Database("failed to delete telemetry credential".to_string())
        })?;

    Ok(affected > 0)
}

fn current_unix_timestamp_seconds() -> Result<u64, TokenStorageError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            TokenStorageError::PathResolution(format!("system clock is invalid: {error}"))
        })?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{LazyLock, Mutex as StdMutex};

    use anyhow::{Context, Result};
    use tokio::sync::{Mutex, MutexGuard};

    use super::*;
    use crate::generated_migrations;
    use crate::services::db::DatabaseKind;

    static TEST_DB_PATH: LazyLock<StdMutex<Option<PathBuf>>> =
        LazyLock::new(|| StdMutex::new(None));
    static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct TestAuthDbSpec;

    impl DbSpec for TestAuthDbSpec {
        fn db_name() -> &'static str {
            "test telemetry credential auth DB"
        }

        fn db_path() -> Result<PathBuf> {
            TEST_DB_PATH
                .lock()
                .expect("test DB path lock should not be poisoned")
                .clone()
                .context("test DB path should be initialized")
        }

        fn migrations() -> &'static [(&'static str, &'static str)] {
            generated_migrations::AUTH_MIGRATIONS
        }

        const KIND: DatabaseKind = DatabaseKind::Auth;
    }

    type TestDb = EncryptedTursoDb<TestAuthDbSpec>;

    const FAR_FUTURE: u64 = 4_102_444_800;

    async fn open_db() -> (MutexGuard<'static, ()>, TestDb) {
        let guard = TEST_LOCK.lock().await;
        std::env::set_var(
            crate::services::db::encryption_key::AUTH_DB_ENCRYPTION_KEY_ENV,
            "telemetry-credential-test-secret",
        );
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let db_path = std::env::temp_dir()
            .join(format!(
                "sce-telemetry-cred-test-{}-{nonce}",
                std::process::id()
            ))
            .join("auth.db");
        *TEST_DB_PATH
            .lock()
            .expect("test DB path lock should not be poisoned") = Some(db_path);
        let db = TestDb::new().await.expect("test auth DB should open");
        delete_with(&db)
            .await
            .expect("telemetry row should be cleared");
        db.execute("DELETE FROM auth_credentials", ())
            .await
            .expect("workos row should be cleared");
        (guard, db)
    }

    fn credential(token: &str, expires_at: u64) -> TelemetryCredential {
        TelemetryCredential {
            token: token.to_string(),
            accessor: format!("accessor-{token}"),
            expires_at_unix_seconds: expires_at,
        }
    }

    fn now() -> u64 {
        current_unix_timestamp_seconds().expect("clock should be valid")
    }

    #[tokio::test]
    async fn telemetry_credential_storage_round_trips_across_reopen() {
        let (_guard, db) = open_db().await;
        let saved = credential("tok-a", FAR_FUTURE);
        save_with(&db, &saved).await.expect("save should succeed");
        drop(db);

        let reopened = TestDb::new().await.expect("reopened auth DB should open");
        let loaded = load_with(&reopened, now())
            .await
            .expect("load should succeed");
        assert_eq!(loaded, Some(saved));
    }

    #[tokio::test]
    async fn telemetry_credential_storage_does_not_return_expired_record() {
        let (_guard, db) = open_db().await;
        save_with(&db, &credential("tok-expired", now() - 10))
            .await
            .expect("save should succeed");

        assert_eq!(
            load_with(&db, now()).await.expect("load should succeed"),
            None
        );
    }

    #[tokio::test]
    async fn telemetry_credential_storage_does_not_return_corrupt_record() {
        let (_guard, db) = open_db().await;
        db.execute(
            "INSERT OR REPLACE INTO telemetry_credentials \
             (id, token, accessor, expires_at_unix_seconds, stored_at_unix_seconds) \
             VALUES (1, '', 'acc', ?1, 0)",
            (i64::try_from(FAR_FUTURE).expect("fits"),),
        )
        .await
        .expect("empty token row should insert");
        assert_eq!(
            load_with(&db, now()).await.expect("load should succeed"),
            None
        );

        db.execute(
            "INSERT OR REPLACE INTO telemetry_credentials \
             (id, token, accessor, expires_at_unix_seconds, stored_at_unix_seconds) \
             VALUES (1, 'tok', 'acc', -5, 0)",
            (),
        )
        .await
        .expect("negative expiry row should insert");
        assert_eq!(
            load_with(&db, now()).await.expect("load should succeed"),
            None
        );
    }

    #[tokio::test]
    async fn telemetry_credential_storage_leaves_workos_credentials_untouched() {
        let (_guard, db) = open_db().await;
        db.execute(
            "INSERT INTO auth_credentials \
             (id, access_token, token_type, expires_in, refresh_token, scope, stored_at_unix_seconds) \
             VALUES (1, 'workos-access', 'Bearer', 3600, 'workos-refresh', NULL, 1)",
            (),
        )
        .await
        .expect("workos row should insert");

        save_with(&db, &credential("tok-b", FAR_FUTURE))
            .await
            .expect("save should succeed");
        assert!(delete_with(&db).await.expect("delete should succeed"));
        assert!(!delete_with(&db)
            .await
            .expect("second delete should succeed"));

        let tokens: Vec<String> = db
            .query_map(
                "SELECT access_token FROM auth_credentials WHERE id = 1",
                (),
                |row| row.get::<String>(0).map_err(anyhow::Error::from),
            )
            .await
            .expect("workos row query should succeed");
        assert_eq!(tokens, vec!["workos-access".to_string()]);
    }

    #[test]
    fn telemetry_credential_storage_debug_output_is_redacted() {
        let rendered = format!("{:?}", credential("super-secret-token", FAR_FUTURE));
        assert!(!rendered.contains("super-secret-token"));
        assert!(!rendered.contains("accessor-super-secret-token"));
        assert!(rendered.contains(REDACTED));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn telemetry_credential_concurrent_saves_leave_one_complete_record() {
        let (_guard, first) = open_db().await;
        let mut dbs = vec![first];
        for _ in 1..4 {
            dbs.push(TestDb::new().await.expect("concurrent auth DB should open"));
        }

        let handles: Vec<_> = dbs
            .into_iter()
            .enumerate()
            .map(|(index, db)| {
                tokio::spawn(async move {
                    for round in 0..3 {
                        save_with(
                            &db,
                            &credential(&format!("tok-{index}-{round}"), FAR_FUTURE),
                        )
                        .await
                        .expect("concurrent save should succeed");
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.await.expect("writer task should not panic");
        }

        let db = TestDb::new()
            .await
            .expect("verification auth DB should open");
        let count: Vec<i64> = db
            .query_map("SELECT COUNT(*) FROM telemetry_credentials", (), |row| {
                row.get::<i64>(0).map_err(anyhow::Error::from)
            })
            .await
            .expect("count query should succeed");
        assert_eq!(count, vec![1]);

        let loaded = load_with(&db, now())
            .await
            .expect("load should succeed")
            .expect("one valid record should exist");
        assert!(loaded.token.starts_with("tok-"));
        assert_eq!(loaded.accessor, format!("accessor-{}", loaded.token));
    }
}
