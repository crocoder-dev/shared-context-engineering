use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;

use crate::services::db::encryption_key::CredentialAccess;
use crate::services::db::{DbSpec, EncryptedTursoDb};
use crate::services::token_storage::{
    open_existing_auth_db, open_or_create_auth_db, TokenStorageError,
};

const TELEMETRY_CREDENTIAL_ROW_ID: i64 = 1;
const WORKOS_SESSION_ROW_ID: i64 = 1;
const REDACTED: &str = "<redacted>";
const JWT_PAYLOAD_SEGMENT: usize = 1;
const BASE64_PADDING: char = '=';

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetryIdentity {
    pub user_id: String,
    pub workspace_id: String,
}

#[derive(Deserialize)]
struct AccessTokenClaims {
    sub: String,
    workspace_id: Option<String>,
}

impl TelemetryIdentity {
    pub fn from_access_token(access_token: &str) -> Option<Self> {
        let payload = access_token.split('.').nth(JWT_PAYLOAD_SEGMENT)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches(BASE64_PADDING))
            .ok()?;
        let claims: AccessTokenClaims = serde_json::from_slice(&bytes).ok()?;
        let identity = Self {
            user_id: claims.sub,
            workspace_id: claims.workspace_id?,
        };
        identity.is_complete().then_some(identity)
    }

    fn is_complete(&self) -> bool {
        !self.user_id.trim().is_empty() && !self.workspace_id.trim().is_empty()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct TelemetryCredential {
    pub token: String,
    pub accessor: String,
    pub expires_at_unix_seconds: u64,
    pub identity: TelemetryIdentity,
}

impl fmt::Debug for TelemetryCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelemetryCredential")
            .field("token", &REDACTED)
            .field("accessor", &REDACTED)
            .field("expires_at_unix_seconds", &self.expires_at_unix_seconds)
            .field("identity", &self.identity)
            .finish()
    }
}

impl TelemetryCredential {
    fn is_valid_at(&self, now_unix_seconds: u64) -> bool {
        !self.token.trim().is_empty()
            && !self.accessor.trim().is_empty()
            && self.identity.is_complete()
            && self.expires_at_unix_seconds > now_unix_seconds
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryBindingMode {
    #[cfg_attr(not(test), expect(dead_code))]
    Enroll,
    EnrollIfUnbound,
}

pub async fn save_telemetry_credential(
    credential: &TelemetryCredential,
) -> Result<(), TokenStorageError> {
    save_with(open_or_create_auth_db().await?, credential).await
}

pub async fn load_telemetry_credential() -> Result<Option<TelemetryCredential>, TokenStorageError> {
    let Some(db) = open_existing_auth_db(CredentialAccess::NonInteractive).await? else {
        return Ok(None);
    };
    load_with(db, current_unix_timestamp_seconds()?).await
}

pub async fn load_repository_telemetry_credential(
    repository_id: String,
) -> Result<Option<TelemetryCredential>, TokenStorageError> {
    let Some(db) = open_existing_auth_db(CredentialAccess::NonInteractive).await? else {
        return Ok(None);
    };
    load_for_repository_with(db, current_unix_timestamp_seconds()?, &repository_id).await
}

pub async fn bind_repository_telemetry(
    repository_id: String,
    mode: RepositoryBindingMode,
) -> Result<bool, TokenStorageError> {
    let Some(db) = open_existing_auth_db(CredentialAccess::Interactive).await? else {
        return Ok(false);
    };
    bind_repository_with(db, current_unix_timestamp_seconds()?, &repository_id, mode).await
}

pub async fn delete_telemetry_credential() -> Result<bool, TokenStorageError> {
    let Some(db) = open_existing_auth_db(CredentialAccess::Interactive).await? else {
        return Ok(false);
    };
    delete_with(db).await
}

fn database_error(message: &str) -> TokenStorageError {
    TokenStorageError::Database(message.to_string())
}

fn timestamp_to_db(value: u64) -> Result<i64, TokenStorageError> {
    i64::try_from(value).map_err(|_| {
        database_error("telemetry credential timestamp is out of range for database storage")
    })
}

async fn save_with<M: DbSpec>(
    db: &EncryptedTursoDb<M>,
    credential: &TelemetryCredential,
) -> Result<(), TokenStorageError> {
    if !credential.identity.is_complete() {
        return Err(database_error(
            "telemetry credential identity is incomplete",
        ));
    }
    let expires_at = timestamp_to_db(credential.expires_at_unix_seconds)?;
    let stored_at = timestamp_to_db(current_unix_timestamp_seconds()?)?;

    db.execute(
        "INSERT OR REPLACE INTO telemetry_credentials \
         (id, token, accessor, expires_at_unix_seconds, stored_at_unix_seconds, user_id, workspace_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            TELEMETRY_CREDENTIAL_ROW_ID,
            credential.token.as_str(),
            credential.accessor.as_str(),
            expires_at,
            stored_at,
            credential.identity.user_id.as_str(),
            credential.identity.workspace_id.as_str(),
        ),
    )
    .await
    .map_err(|_| database_error("failed to save telemetry credential"))?;

    Ok(())
}

async fn load_with<M: DbSpec>(
    db: &EncryptedTursoDb<M>,
    now_unix_seconds: u64,
) -> Result<Option<TelemetryCredential>, TokenStorageError> {
    let rows: Vec<Option<(TelemetryCredential, String)>> = db
        .query_map(
            "SELECT t.token, t.accessor, t.expires_at_unix_seconds, t.user_id, t.workspace_id, \
             a.access_token \
             FROM telemetry_credentials t JOIN auth_credentials a ON a.id = ?2 \
             WHERE t.id = ?1",
            (TELEMETRY_CREDENTIAL_ROW_ID, WORKOS_SESSION_ROW_ID),
            |row| {
                let token: String = row.get(0)?;
                let accessor: String = row.get(1)?;
                let expires_at: i64 = row.get(2)?;
                let user_id: String = row.get(3)?;
                let workspace_id: String = row.get(4)?;
                let session_access_token: String = row.get(5)?;
                Ok(u64::try_from(expires_at)
                    .ok()
                    .map(|expires_at_unix_seconds| {
                        (
                            TelemetryCredential {
                                token,
                                accessor,
                                expires_at_unix_seconds,
                                identity: TelemetryIdentity {
                                    user_id,
                                    workspace_id,
                                },
                            },
                            session_access_token,
                        )
                    }))
            },
        )
        .await
        .map_err(|_| database_error("failed to load telemetry credential"))?;

    Ok(rows
        .into_iter()
        .next()
        .flatten()
        .filter(|(credential, session_access_token)| {
            credential.is_valid_at(now_unix_seconds)
                && TelemetryIdentity::from_access_token(session_access_token).as_ref()
                    == Some(&credential.identity)
        })
        .map(|(credential, _)| credential))
}

async fn load_for_repository_with<M: DbSpec>(
    db: &EncryptedTursoDb<M>,
    now_unix_seconds: u64,
    repository_id: &str,
) -> Result<Option<TelemetryCredential>, TokenStorageError> {
    let Some(credential) = load_with(db, now_unix_seconds).await? else {
        return Ok(None);
    };
    let authorized = repository_bound_to(db, repository_id, &credential.identity).await?;
    Ok(authorized.then_some(credential))
}

async fn repository_bound_to<M: DbSpec>(
    db: &EncryptedTursoDb<M>,
    repository_id: &str,
    identity: &TelemetryIdentity,
) -> Result<bool, TokenStorageError> {
    if repository_id.trim().is_empty() {
        return Ok(false);
    }
    let matches: Vec<i64> = db
        .query_map(
            "SELECT 1 FROM telemetry_repository_bindings \
             WHERE repository_id = ?1 AND user_id = ?2 AND workspace_id = ?3",
            (
                repository_id,
                identity.user_id.as_str(),
                identity.workspace_id.as_str(),
            ),
            |row| row.get::<i64>(0).map_err(anyhow::Error::from),
        )
        .await
        .map_err(|_| database_error("failed to load telemetry repository binding"))?;
    Ok(!matches.is_empty())
}

async fn bind_repository_with<M: DbSpec>(
    db: &EncryptedTursoDb<M>,
    now_unix_seconds: u64,
    repository_id: &str,
    mode: RepositoryBindingMode,
) -> Result<bool, TokenStorageError> {
    if repository_id.trim().is_empty() {
        return Ok(false);
    }
    let Some(credential) = load_with(db, now_unix_seconds).await? else {
        return Ok(false);
    };
    let sql = match mode {
        RepositoryBindingMode::Enroll => {
            "INSERT OR REPLACE INTO telemetry_repository_bindings \
             (repository_id, user_id, workspace_id, bound_at_unix_seconds) VALUES (?1, ?2, ?3, ?4)"
        }
        RepositoryBindingMode::EnrollIfUnbound => {
            "INSERT OR IGNORE INTO telemetry_repository_bindings \
             (repository_id, user_id, workspace_id, bound_at_unix_seconds) VALUES (?1, ?2, ?3, ?4)"
        }
    };
    db.execute(
        sql,
        (
            repository_id,
            credential.identity.user_id.as_str(),
            credential.identity.workspace_id.as_str(),
            timestamp_to_db(now_unix_seconds)?,
        ),
    )
    .await
    .map_err(|_| database_error("failed to save telemetry repository binding"))?;

    repository_bound_to(db, repository_id, &credential.identity).await
}

async fn delete_with<M: DbSpec>(db: &EncryptedTursoDb<M>) -> Result<bool, TokenStorageError> {
    let affected = db
        .execute(
            "DELETE FROM telemetry_credentials WHERE id = ?1",
            (TELEMETRY_CREDENTIAL_ROW_ID,),
        )
        .await
        .map_err(|_| database_error("failed to delete telemetry credential"))?;

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
pub(crate) fn test_access_token(user_id: &str, workspace_id: Option<&str>) -> String {
    let claims = match workspace_id {
        Some(workspace_id) => serde_json::json!({"sub": user_id, "workspace_id": workspace_id}),
        None => serde_json::json!({"sub": user_id}),
    };
    format!(
        "eyJhbGciOiJSUzI1NiJ9.{}.signature",
        URL_SAFE_NO_PAD.encode(claims.to_string())
    )
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
        seed_workos_session(&db).await;
        (guard, db)
    }

    const USER_A: &str = "user_a";
    const WORKSPACE_A: &str = "workspace_a";
    const USER_B: &str = "user_b";
    const WORKSPACE_B: &str = "workspace_b";
    const REPOSITORY_A: &str = "repository-a";
    const REPOSITORY_B: &str = "repository-b";

    async fn seed_workos_session(db: &TestDb) {
        seed_session(db, USER_A, WORKSPACE_A).await;
    }

    async fn seed_session(db: &TestDb, user_id: &str, workspace_id: &str) {
        db.execute(
            "INSERT OR REPLACE INTO auth_credentials \
             (id, access_token, token_type, expires_in, refresh_token, scope, stored_at_unix_seconds) \
             VALUES (1, ?1, 'Bearer', 3600, 'workos-refresh', NULL, 1)",
            (test_access_token(user_id, Some(workspace_id)),),
        )
        .await
        .expect("workos row should insert");
    }

    fn identity(user_id: &str, workspace_id: &str) -> TelemetryIdentity {
        TelemetryIdentity {
            user_id: user_id.to_string(),
            workspace_id: workspace_id.to_string(),
        }
    }

    fn credential(token: &str, expires_at: u64) -> TelemetryCredential {
        credential_for(token, expires_at, identity(USER_A, WORKSPACE_A))
    }

    fn credential_for(
        token: &str,
        expires_at: u64,
        identity: TelemetryIdentity,
    ) -> TelemetryCredential {
        TelemetryCredential {
            token: token.to_string(),
            accessor: format!("accessor-{token}"),
            expires_at_unix_seconds: expires_at,
            identity,
        }
    }

    async fn log_in_as(db: &TestDb, user_id: &str, workspace_id: &str, token: &str) {
        delete_with(db).await.expect("delete should succeed");
        seed_session(db, user_id, workspace_id).await;
        save_with(
            db,
            &credential_for(token, FAR_FUTURE, identity(user_id, workspace_id)),
        )
        .await
        .expect("save should succeed");
    }

    async fn repository_token(db: &TestDb, repository_id: &str) -> Option<String> {
        load_for_repository_with(db, now(), repository_id)
            .await
            .expect("load should succeed")
            .map(|credential| credential.token)
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
    async fn telemetry_credential_is_rejected_without_owning_workos_session() {
        let (_guard, db) = open_db().await;
        save_with(&db, &credential("tok-orphan", FAR_FUTURE))
            .await
            .expect("save should succeed");
        db.execute("DELETE FROM auth_credentials", ())
            .await
            .expect("workos row should be cleared");

        assert_eq!(
            load_with(&db, now()).await.expect("load should succeed"),
            None
        );
    }

    #[tokio::test]
    async fn telemetry_credential_storage_leaves_workos_credentials_untouched() {
        let (_guard, db) = open_db().await;
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
        assert_eq!(tokens, vec![test_access_token(USER_A, Some(WORKSPACE_A))]);
    }

    #[tokio::test]
    async fn telemetry_headers_helper_reads_stored_credential_and_follows_replacement() {
        use crate::services::auth_command::render_telemetry_headers;

        let (_guard, db) = open_db().await;
        save_with(&db, &credential("tok-first", FAR_FUTURE))
            .await
            .expect("save should succeed");
        let first = render_telemetry_headers(load_with(&db, now()).await)
            .expect("stored credential should render");
        assert_eq!(first, r#"{"Authorization":"Bearer tok-first"}"#);

        save_with(&db, &credential("tok-second", FAR_FUTURE))
            .await
            .expect("replacement save should succeed");
        let second = render_telemetry_headers(load_with(&db, now()).await)
            .expect("replacement should render");
        assert_eq!(second, r#"{"Authorization":"Bearer tok-second"}"#);
    }

    #[tokio::test]
    async fn telemetry_headers_helper_fails_with_fixed_diagnostic_for_unusable_stored_state() {
        use crate::services::auth_command::render_telemetry_headers;

        let (_guard, db) = open_db().await;
        let missing = render_telemetry_headers(load_with(&db, now()).await)
            .expect_err("missing credential should fail");

        save_with(&db, &credential("tok-expired", now() - 10))
            .await
            .expect("save should succeed");
        let expired = render_telemetry_headers(load_with(&db, now()).await)
            .expect_err("expired credential should fail");

        db.execute(
            "INSERT OR REPLACE INTO telemetry_credentials \
             (id, token, accessor, expires_at_unix_seconds, stored_at_unix_seconds) \
             VALUES (1, '', 'acc', ?1, 0)",
            (i64::try_from(FAR_FUTURE).expect("fits"),),
        )
        .await
        .expect("malformed row should insert");
        let malformed = render_telemetry_headers(load_with(&db, now()).await)
            .expect_err("malformed credential should fail");

        for error in [missing, expired, malformed] {
            assert_eq!(error.class().exit_code(), 4);
            let text = error.to_string();
            assert_eq!(text, "Telemetry credential is unavailable.");
            assert!(!text.contains("tok-"));
        }
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
                    let mut saved = 0_usize;
                    for round in 0..3 {
                        let result = save_with(
                            &db,
                            &credential(&format!("tok-{index}-{round}"), FAR_FUTURE),
                        )
                        .await;
                        if result.is_ok() {
                            saved += 1;
                        }
                    }
                    saved
                })
            })
            .collect();
        let mut total_saved = 0_usize;
        for handle in handles {
            total_saved += handle.await.expect("writer task should not panic");
        }
        assert!(
            total_saved > 0,
            "at least one contended save should succeed"
        );

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

    #[test]
    fn telemetry_identity_comes_only_from_complete_access_token_claims() {
        assert_eq!(
            TelemetryIdentity::from_access_token(&test_access_token(USER_A, Some(WORKSPACE_A))),
            Some(identity(USER_A, WORKSPACE_A))
        );
        for token in [
            test_access_token(USER_A, None),
            test_access_token(USER_A, Some(" ")),
            test_access_token("", Some(WORKSPACE_A)),
            "opaque".to_string(),
            "a.%%%.c".to_string(),
        ] {
            assert_eq!(TelemetryIdentity::from_access_token(&token), None);
        }
    }

    #[tokio::test]
    async fn telemetry_credential_is_rejected_after_session_switches_identity() {
        let (_guard, db) = open_db().await;
        save_with(&db, &credential("tok-a", FAR_FUTURE))
            .await
            .expect("save should succeed");

        for (user_id, workspace_id) in [(USER_A, WORKSPACE_B), (USER_B, WORKSPACE_A)] {
            seed_session(&db, user_id, workspace_id).await;
            assert_eq!(
                load_with(&db, now()).await.expect("load should succeed"),
                None
            );
        }

        seed_session(&db, USER_A, WORKSPACE_A).await;
        assert_eq!(
            load_with(&db, now())
                .await
                .expect("load should succeed")
                .map(|credential| credential.token),
            Some("tok-a".to_string())
        );
    }

    #[tokio::test]
    async fn legacy_credential_without_identity_is_never_returned() {
        let (_guard, db) = open_db().await;
        db.execute(
            "INSERT OR REPLACE INTO telemetry_credentials \
             (id, token, accessor, expires_at_unix_seconds, stored_at_unix_seconds) \
             VALUES (1, 'tok-legacy', 'acc', ?1, 0)",
            (i64::try_from(FAR_FUTURE).expect("fits"),),
        )
        .await
        .expect("legacy row should insert");

        assert_eq!(
            load_with(&db, now()).await.expect("load should succeed"),
            None
        );
        assert!(
            !bind_repository_with(&db, now(), REPOSITORY_A, RepositoryBindingMode::Enroll)
                .await
                .expect("bind should succeed")
        );
    }

    #[tokio::test]
    async fn workspace_b_repository_cannot_use_workspace_a_credential() {
        let (_guard, db) = open_db().await;

        log_in_as(&db, USER_B, WORKSPACE_B, "tok-b").await;
        assert!(
            bind_repository_with(&db, now(), REPOSITORY_B, RepositoryBindingMode::Enroll)
                .await
                .expect("bind should succeed")
        );
        assert_eq!(
            repository_token(&db, REPOSITORY_B).await,
            Some("tok-b".to_string())
        );

        log_in_as(&db, USER_A, WORKSPACE_A, "tok-a").await;
        assert!(bind_repository_with(
            &db,
            now(),
            REPOSITORY_A,
            RepositoryBindingMode::EnrollIfUnbound
        )
        .await
        .expect("bind should succeed"));
        assert!(
            !bind_repository_with(
                &db,
                now(),
                REPOSITORY_B,
                RepositoryBindingMode::EnrollIfUnbound
            )
            .await
            .expect("bind should succeed"),
            "implicit enrollment must not move a repository to another workspace"
        );

        assert_eq!(
            repository_token(&db, REPOSITORY_A).await,
            Some("tok-a".to_string())
        );
        assert_eq!(repository_token(&db, REPOSITORY_B).await, None);
        assert_eq!(repository_token(&db, "repository-unknown").await, None);
        assert_eq!(repository_token(&db, "").await, None);

        log_in_as(&db, USER_A, WORKSPACE_B, "tok-a-in-b").await;
        assert_eq!(repository_token(&db, REPOSITORY_A).await, None);
        assert_eq!(repository_token(&db, REPOSITORY_B).await, None);
    }

    #[tokio::test]
    async fn explicit_enrollment_rebinds_repository_to_current_identity() {
        let (_guard, db) = open_db().await;
        log_in_as(&db, USER_B, WORKSPACE_B, "tok-b").await;
        assert!(
            bind_repository_with(&db, now(), REPOSITORY_B, RepositoryBindingMode::Enroll)
                .await
                .expect("bind should succeed")
        );

        log_in_as(&db, USER_A, WORKSPACE_A, "tok-a").await;
        assert!(
            bind_repository_with(&db, now(), REPOSITORY_B, RepositoryBindingMode::Enroll)
                .await
                .expect("bind should succeed")
        );
        assert_eq!(
            repository_token(&db, REPOSITORY_B).await,
            Some("tok-a".to_string())
        );

        log_in_as(&db, USER_B, WORKSPACE_B, "tok-b-2").await;
        assert_eq!(repository_token(&db, REPOSITORY_B).await, None);
    }

    #[tokio::test]
    async fn failed_credential_replacement_logout_and_rotation_keep_bindings_fail_closed() {
        let (_guard, db) = open_db().await;
        log_in_as(&db, USER_A, WORKSPACE_A, "tok-1").await;
        assert!(
            bind_repository_with(&db, now(), REPOSITORY_A, RepositoryBindingMode::Enroll)
                .await
                .expect("bind should succeed")
        );

        save_with(&db, &credential("tok-2", FAR_FUTURE))
            .await
            .expect("rotation save should succeed");
        assert_eq!(
            repository_token(&db, REPOSITORY_A).await,
            Some("tok-2".to_string())
        );

        delete_with(&db).await.expect("delete should succeed");
        seed_session(&db, USER_A, WORKSPACE_A).await;
        assert_eq!(repository_token(&db, REPOSITORY_A).await, None);

        delete_with(&db)
            .await
            .expect("logout delete should succeed");
        db.execute("DELETE FROM auth_credentials", ())
            .await
            .expect("session delete should succeed");
        assert_eq!(repository_token(&db, REPOSITORY_A).await, None);
        assert!(
            !bind_repository_with(&db, now(), REPOSITORY_A, RepositoryBindingMode::Enroll)
                .await
                .expect("bind should succeed")
        );

        log_in_as(&db, USER_A, WORKSPACE_A, "tok-3").await;
        assert_eq!(
            repository_token(&db, REPOSITORY_A).await,
            Some("tok-3".to_string())
        );

        save_with(&db, &credential("tok-expired", now() - 1))
            .await
            .expect("save should succeed");
        assert_eq!(repository_token(&db, REPOSITORY_A).await, None);
    }

    #[tokio::test]
    async fn incomplete_identity_is_never_saved() {
        let (_guard, db) = open_db().await;
        let result = save_with(
            &db,
            &credential_for("tok-anon", FAR_FUTURE, identity(USER_A, "")),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(
            load_with(&db, now()).await.expect("load should succeed"),
            None
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn headers_helper_in_claude_runtime_environment_only_serves_bound_repository() {
        use crate::services::auth_command::telemetry_headers_with;
        use crate::services::managed_telemetry::claude_helper::tests::git_repository_with_remote;
        use crate::services::managed_telemetry::claude_helper::{
            repository_id_for, visible_destination_conflict,
        };

        let (_guard, db) = open_db().await;
        let dir = tempfile::tempdir().unwrap();
        let repo_a =
            git_repository_with_remote(dir.path(), "repo-a", "git@github.com:acme/repo-a.git");
        let repo_b =
            git_repository_with_remote(dir.path(), "repo-b", "https://github.com/acme/repo-b");
        let unrelated =
            git_repository_with_remote(dir.path(), "unrelated", "https://github.com/acme/other");
        let bound_repository_id = repository_id_for(&repo_a).expect("repo a identity");
        let foreign_repository_id = repository_id_for(&repo_b).expect("repo b identity");

        log_in_as(&db, USER_B, WORKSPACE_B, "tok-b").await;
        assert!(bind_repository_with(
            &db,
            now(),
            &foreign_repository_id,
            RepositoryBindingMode::Enroll
        )
        .await
        .expect("bind should succeed"));
        log_in_as(&db, USER_A, WORKSPACE_A, "tok-a").await;
        assert!(bind_repository_with(
            &db,
            now(),
            &bound_repository_id,
            RepositoryBindingMode::Enroll
        )
        .await
        .expect("bind should succeed"));
        drop(db);

        let claude_helper_env = |_: &str| None::<String>;
        let headers_for = |working_dir: std::path::PathBuf| {
            telemetry_headers_with(
                Some(working_dir),
                visible_destination_conflict(claude_helper_env),
                true,
                repository_id_for,
                |repository_id: String| async move {
                    let db = TestDb::new()
                        .await
                        .map_err(|_| TokenStorageError::Database("open".to_string()))?;
                    load_for_repository_with(&db, now(), &repository_id).await
                },
            )
        };

        let headers = headers_for(repo_a.join("src").join("module"))
            .await
            .expect("bound repository launched from a subdirectory should render");
        assert_eq!(headers, r#"{"Authorization":"Bearer tok-a"}"#);

        for working_dir in [repo_b.clone(), unrelated, dir.path().to_path_buf()] {
            let error = headers_for(working_dir)
                .await
                .expect_err("unbound or foreign-workspace repository must fail closed");
            assert_eq!(error.to_string(), "Telemetry credential is unavailable.");
        }
    }
}
