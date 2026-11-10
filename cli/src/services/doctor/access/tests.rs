use super::*;
use crate::services::token_storage::StoredTokens;

fn session(access_token: &str, refresh_token: &str, expires_in: u64) -> StoredTokensLookup {
    StoredTokensLookup::Found(StoredTokens {
        access_token: access_token.to_string(),
        refresh_token: refresh_token.to_string(),
        token_type: "Bearer".to_string(),
        expires_in,
        scope: None,
        stored_at_unix_seconds: 0,
    })
}

#[tokio::test]
async fn saved_workos_session_does_not_require_telemetry_or_repository_enrollment() {
    let login = inspect_login_with(CREDENTIAL_LOOKUP_DEADLINE, async {
        Ok(session("expired-access", "saved-refresh", 0))
    })
    .await;
    assert_eq!(login, LoginState::LoggedIn);
}

#[tokio::test]
async fn current_access_token_without_refresh_token_is_logged_in() {
    let login = inspect_login_with(CREDENTIAL_LOOKUP_DEADLINE, async {
        Ok(session("current-access", "", u64::MAX))
    })
    .await;
    assert_eq!(login, LoginState::LoggedIn);
}

#[tokio::test]
async fn missing_empty_or_expired_unrenewable_session_requires_login() {
    for lookup in [
        StoredTokensLookup::Missing,
        session("", " ", u64::MAX),
        session("expired-access", "", 0),
    ] {
        let login = inspect_login_with(CREDENTIAL_LOOKUP_DEADLINE, async { Ok(lookup) }).await;
        assert_eq!(login, LoginState::NotLoggedIn);
    }
}

#[tokio::test]
async fn storage_failures_are_unknown_login_status_instead_of_logout() {
    for (error, reason) in [
        (
            TokenStorageError::CredentialAccessUnavailable,
            LoginUnavailableReason::CredentialInaccessible,
        ),
        (
            TokenStorageError::DatabaseLocked("private detail".to_string()),
            LoginUnavailableReason::DatabaseLocked,
        ),
        (
            TokenStorageError::Database("private detail".to_string()),
            LoginUnavailableReason::DatabaseError,
        ),
        (
            TokenStorageError::PathResolution("private detail".to_string()),
            LoginUnavailableReason::PathResolutionError,
        ),
    ] {
        let login = inspect_login_with(CREDENTIAL_LOOKUP_DEADLINE, async { Err(error) }).await;
        assert_eq!(login, LoginState::Unavailable(reason));
        let (summary, remediation) = reason.guidance();
        assert!(!summary.contains("private detail"));
        assert!(!remediation.contains("private detail"));
    }
}

#[tokio::test]
async fn outdated_schema_is_unavailable_instead_of_logged_out() {
    let login = inspect_login_with(CREDENTIAL_LOOKUP_DEADLINE, async {
        Ok(StoredTokensLookup::SchemaNotCurrent)
    })
    .await;
    assert_eq!(
        login,
        LoginState::Unavailable(LoginUnavailableReason::SchemaNotCurrent)
    );
}

#[tokio::test]
async fn lookup_timeout_does_not_claim_the_session_is_missing() {
    let login = inspect_login_with(Duration::from_millis(1), std::future::pending()).await;
    assert_eq!(
        login,
        LoginState::Unavailable(LoginUnavailableReason::LookupTimeout)
    );
}
