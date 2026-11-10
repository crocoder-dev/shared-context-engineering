use std::process::Command;

use super::*;
use crate::services::db::encryption_key::AUTH_DB_ENCRYPTION_KEY_ENV;

const PROBE_CASE: &str = "SCE_TEST_SESSION_PROBE_CASE";

#[test]
fn saved_session_inspection_is_independent_of_telemetry_and_does_not_migrate() {
    for case in [
        "missing_db",
        "missing_session",
        "missing_telemetry",
        "expired_telemetry",
        "unbound_repository",
        "outdated_schema",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "services::token_storage::tests::session_probe_child",
                "--nocapture",
            ])
            .env(PROBE_CASE, case)
            .env("HOME", directory.path())
            .env("USERPROFILE", directory.path())
            .env("LOCALAPPDATA", directory.path().join("state"))
            .env("APPDATA", directory.path().join("config"))
            .env("XDG_STATE_HOME", directory.path().join("state"))
            .env("XDG_CONFIG_HOME", directory.path().join("config"))
            .env("XDG_DATA_HOME", directory.path().join("data"))
            .env("XDG_CACHE_HOME", directory.path().join("cache"))
            .env(
                AUTH_DB_ENCRYPTION_KEY_ENV,
                if case == "missing_db" {
                    ""
                } else {
                    "session-probe-secret"
                },
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[tokio::test]
async fn session_probe_child() {
    let Ok(case) = std::env::var(PROBE_CASE) else {
        return;
    };
    let path = auth_db_path().unwrap();
    if case == "missing_db" {
        assert!(matches!(
            load_current_tokens().await.unwrap(),
            StoredTokensLookup::Missing
        ));
        assert!(!path.parent().unwrap().exists());
        return;
    }
    let db = AuthDb::new().await.unwrap();
    if case != "missing_session" {
        db.execute(
            "INSERT INTO auth_credentials \
             (id, access_token, token_type, expires_in, refresh_token, stored_at_unix_seconds) \
             VALUES (1, 'expired-access', 'Bearer', 0, 'saved-refresh', 0)",
            (),
        )
        .await
        .unwrap();
    }
    if matches!(case.as_str(), "expired_telemetry" | "unbound_repository") {
        let expires_at = if case == "expired_telemetry" {
            1
        } else {
            4_102_444_800_i64
        };
        db.execute(
            "INSERT INTO telemetry_credentials \
             (id, token, accessor, expires_at_unix_seconds, stored_at_unix_seconds, user_id, workspace_id) \
             VALUES (1, 'telemetry-token', 'accessor', ?1, 0, 'user', 'workspace')",
            (expires_at,),
        )
        .await
        .unwrap();
    }
    if case == "outdated_schema" {
        db.execute(
            "DELETE FROM __sce_migrations WHERE id = ?1",
            (crate::generated_migrations::AUTH_MIGRATIONS
                .last()
                .unwrap()
                .0,),
        )
        .await
        .unwrap();
    }
    drop(db);
    let before = std::fs::read(&path).unwrap();
    let lookup = load_current_tokens().await.unwrap();
    match case.as_str() {
        "outdated_schema" => {
            assert!(matches!(lookup, StoredTokensLookup::SchemaNotCurrent));
            assert_eq!(std::fs::read(path).unwrap(), before);
        }
        "missing_session" => assert!(matches!(lookup, StoredTokensLookup::Missing)),
        _ => {
            let StoredTokensLookup::Found(tokens) = lookup else {
                panic!("saved WorkOS session must be found regardless of telemetry readiness");
            };
            assert_eq!(tokens.refresh_token, "saved-refresh");
            assert_eq!(tokens.expires_in, 0);
        }
    }
}
