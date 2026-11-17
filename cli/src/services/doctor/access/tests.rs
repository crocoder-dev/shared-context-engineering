use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::services::token_storage::StoredTokens;

const HOUR: u64 = 60 * 60;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_secs()
}

fn tokens(
    access_token: &str,
    refresh_token: &str,
    expires_in: u64,
    stored_at: u64,
) -> StoredTokens {
    StoredTokens {
        access_token: access_token.to_string(),
        refresh_token: refresh_token.to_string(),
        token_type: "Bearer".to_string(),
        expires_in,
        scope: None,
        stored_at_unix_seconds: stored_at,
    }
}

fn session(access_token: &str, refresh_token: &str, expires_in: u64) -> StoredTokensLookup {
    StoredTokensLookup::Found(tokens(access_token, refresh_token, expires_in, 0))
}

fn expired_session_confirmed(seconds_ago: u64) -> StoredTokens {
    tokens("expired-access", "saved-refresh", 0, now() - seconds_ago)
}

fn found(stored: &StoredTokens) -> StoredTokensLookup {
    StoredTokensLookup::Found(stored.clone())
}

fn no_renewal(_: String) -> std::future::Ready<Result<StoredTokens, SessionRenewalError>> {
    panic!("renewal must not run")
}

fn renewed() -> StoredTokens {
    tokens("renewed-access", "rotated-refresh", HOUR, now())
}

async fn inspect_lookup(lookup: StoredTokensLookup) -> LoginState {
    let lookup = Mutex::new(Some(lookup));
    inspect_login_with(
        CREDENTIAL_LOOKUP_DEADLINE,
        || {
            let next = lookup.lock().unwrap().take();
            async move { Ok(next.expect("lookup runs once")) }
        },
        no_renewal,
    )
    .await
}

async fn inspect_renewal(
    stored: StoredTokens,
    renewal: Result<StoredTokens, SessionRenewalError>,
) -> LoginState {
    inspect_login_with(
        CREDENTIAL_LOOKUP_DEADLINE,
        move || {
            let stored = stored.clone();
            async move { Ok(found(&stored)) }
        },
        move |_| async move { renewal },
    )
    .await
}

#[tokio::test]
async fn current_access_token_is_logged_in_without_contacting_workos() {
    for lookup in [
        session("current-access", "saved-refresh", u64::MAX),
        session("current-access", "", u64::MAX),
    ] {
        assert_eq!(inspect_lookup(lookup).await, LoginState::LoggedIn);
    }
}

#[tokio::test]
async fn missing_empty_or_expired_unrenewable_session_requires_login() {
    for lookup in [
        StoredTokensLookup::Missing,
        session("", " ", u64::MAX),
        session("expired-access", "", 0),
    ] {
        assert_eq!(inspect_lookup(lookup).await, LoginState::NotLoggedIn);
    }
}

#[tokio::test]
async fn expired_access_token_is_renewed_and_saved() {
    let renewed_with = Arc::new(Mutex::new(None));
    let captured = Arc::clone(&renewed_with);
    let stored = expired_session_confirmed(2 * HOUR);
    let login = inspect_login_with(
        CREDENTIAL_LOOKUP_DEADLINE,
        move || {
            let stored = stored.clone();
            async move { Ok(found(&stored)) }
        },
        move |refresh_token| async move {
            *captured.lock().unwrap() = Some(refresh_token);
            Ok(renewed())
        },
    )
    .await;
    assert_eq!(login, LoginState::LoggedIn);
    assert_eq!(
        renewed_with.lock().unwrap().as_deref(),
        Some("saved-refresh")
    );
}

#[tokio::test]
async fn rejected_renewal_requires_login() {
    let login = inspect_renewal(
        expired_session_confirmed(HOUR),
        Err(SessionRenewalError::Auth(AuthError::Unauthorized(
            "Session ended due to inactivity.".to_string(),
        ))),
    )
    .await;
    assert_eq!(login, LoginState::NotLoggedIn);
}

#[tokio::test]
async fn rejected_renewal_after_rotation_by_another_process_is_logged_in() {
    let stored = expired_session_confirmed(2 * HOUR);
    let lookups = AtomicUsize::new(0);
    let login = inspect_login_with(
        CREDENTIAL_LOOKUP_DEADLINE,
        || {
            let next = if lookups.fetch_add(1, Ordering::SeqCst) == 0 {
                found(&stored)
            } else {
                StoredTokensLookup::Found(renewed())
            };
            async move { Ok(next) }
        },
        |_| async {
            Err(SessionRenewalError::Auth(AuthError::Unauthorized(
                "refresh token already used".to_string(),
            )))
        },
    )
    .await;
    assert_eq!(login, LoginState::LoggedIn);
    assert_eq!(lookups.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unreachable_workos_keeps_recently_confirmed_session_logged_in() {
    let login = inspect_login_with(
        Duration::from_millis(20),
        || {
            let stored = expired_session_confirmed(23 * HOUR);
            async move { Ok(found(&stored)) }
        },
        |_| std::future::pending(),
    )
    .await;
    assert_eq!(login, LoginState::LoggedIn);
}

#[tokio::test]
async fn unreachable_workos_reports_stale_session_as_unavailable() {
    let login = inspect_renewal(
        expired_session_confirmed(25 * HOUR),
        Err(SessionRenewalError::Auth(AuthError::InvalidResponse(
            "upstream unavailable".to_string(),
        ))),
    )
    .await;
    assert_eq!(
        login,
        LoginState::Unavailable(LoginUnavailableReason::SessionUnreachable)
    );
}

#[tokio::test]
async fn missing_client_id_is_unavailable_instead_of_logged_out() {
    let login = inspect_renewal(
        expired_session_confirmed(HOUR),
        Err(SessionRenewalError::Auth(AuthError::MissingClientId)),
    )
    .await;
    assert_eq!(
        login,
        LoginState::Unavailable(LoginUnavailableReason::AuthConfigUnavailable)
    );
}

#[tokio::test]
async fn failed_save_after_renewal_is_unavailable() {
    let login = inspect_renewal(
        expired_session_confirmed(HOUR),
        Err(SessionRenewalError::Storage(
            TokenStorageError::DatabaseLocked("private detail".to_string()),
        )),
    )
    .await;
    assert_eq!(
        login,
        LoginState::Unavailable(LoginUnavailableReason::DatabaseLocked)
    );
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
        let error = Mutex::new(Some(error));
        let login = inspect_login_with(
            CREDENTIAL_LOOKUP_DEADLINE,
            || {
                let error = error.lock().unwrap().take().expect("lookup runs once");
                async move { Err(error) }
            },
            no_renewal,
        )
        .await;
        assert_eq!(login, LoginState::Unavailable(reason));
        let (summary, remediation) = reason.guidance();
        assert!(!summary.contains("private detail"));
        assert!(!remediation.contains("private detail"));
    }
}

#[tokio::test]
async fn outdated_schema_is_unavailable_instead_of_logged_out() {
    assert_eq!(
        inspect_lookup(StoredTokensLookup::SchemaNotCurrent).await,
        LoginState::Unavailable(LoginUnavailableReason::SchemaNotCurrent)
    );
}

#[tokio::test]
async fn lookup_timeout_does_not_claim_the_session_is_missing() {
    let login =
        inspect_login_with(Duration::from_millis(1), std::future::pending, no_renewal).await;
    assert_eq!(
        login,
        LoginState::Unavailable(LoginUnavailableReason::LookupTimeout)
    );
}
