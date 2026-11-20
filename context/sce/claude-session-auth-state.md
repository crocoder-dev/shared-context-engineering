# Claude Session Auth State

## Status

The pure detector lives in `cli/src/services/hooks/claude_auth_detection.rs`
(`pub(crate)`). Migration `008_claude_session_auth_state` and a typed
persistence interface on `RepositoryAgentTraceDb` exist (see
[Persistence](#persistence)), but no hook calls either one yet. Until a runtime
caller exists, the detector module declaration and the two public adapter
methods carry `#[cfg_attr(not(test), expect(dead_code, ...))]`.
Plan: [`claude-auth-billing-observations`](../plans/claude-auth-billing-observations.md).

## Detector

`detect_claude_auth(ClaudeAuthSignals) -> ClaudeAuthDetection` is pure and
deterministic. It classifies Claude Code's *configured* authentication from the
presence of documented environment variables.

- `ClaudeAuthSignals` holds only presence booleans and never stores values.
  `ClaudeAuthSignals::from_env_lookup(Fn(&str) -> Option<String>)` builds it and
  drops each looked-up value right after its check, so tests inject a closure
  instead of changing the process environment.
- A cloud flag (`CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`,
  `CLAUDE_CODE_USE_FOUNDRY`) counts only when it is `1` or `true` (any case,
  trimmed).
- A credential variable (`ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY`,
  `CLAUDE_CODE_OAUTH_TOKEN`) counts when it is present and non-blank.
- `ANTHROPIC_BASE_URL` counts as a custom gateway when it is non-blank and not
  `https://api.anthropic.com`. The comparison trims whitespace, strips a trailing
  `/`, and ignores case. The base URL feeds classification but is not a
  credential, so it has no `AuthEnvKey` variant.

### Precedence and classification

Cloud flags are checked first, then `ANTHROPIC_AUTH_TOKEN`, then `ANTHROPIC_API_KEY`,
then `CLAUDE_CODE_OAUTH_TOKEN`. Credentials from interactive `/login` cannot be
observed.

| Selected signal | Provider | Billing mode | Auth env key |
|---|---|---|---|
| `CLAUDE_CODE_USE_BEDROCK` | `aws_bedrock` | `cloud_provider` | `claude_code_use_bedrock` |
| `CLAUDE_CODE_USE_VERTEX` | `google_vertex` | `cloud_provider` | `claude_code_use_vertex` |
| `CLAUDE_CODE_USE_FOUNDRY` | `microsoft_foundry` | `cloud_provider` | `claude_code_use_foundry` |
| `ANTHROPIC_AUTH_TOKEN`, default base URL | `anthropic` | `unknown` | `anthropic_auth_token` |
| `ANTHROPIC_AUTH_TOKEN`, custom base URL | `custom_gateway` | `unknown` | `anthropic_auth_token` |
| `ANTHROPIC_API_KEY`, default base URL | `anthropic` | `api_key` | `anthropic_api_key` |
| `ANTHROPIC_API_KEY`, custom base URL | `custom_gateway` | `unknown` | `anthropic_api_key` |
| `CLAUDE_CODE_OAUTH_TOKEN` | `anthropic` | `unknown` | `claude_code_oauth_token` |
| No signal, or more than one cloud flag | `unknown` | `unknown` | none |

Every recognized signal yields confidence `inferred`. No signal, or a provider
conflict, yields confidence `unknown`. The strings shown are the stable
`as_str`/`from_str` mappings of `AuthProvider`, `BillingMode`, `AuthEnvKey`, and
`DetectionConfidence`. `from_str` rejects any other value.

## Configuration is not billing

Environment variables describe how authentication is configured. They do not
prove that authentication succeeded or show what is actually billed.

- `ANTHROPIC_AUTH_TOKEN` may authenticate against a proxy or gateway, so it
  never implies `api_key` billing.
- OAuth never implies a paid subscription. `BillingMode::Subscription` exists as
  a valid stored value, but the detector never returns it.
- `DetectionConfidence` has no verified level, because none of the permitted
  inputs can verify authentication.

## Limitations

- Interactive `/login` users with no environment signal always classify as
  `unknown`, because the detector does not read credential files, the keychain,
  or settings.
- `apiKeyHelper` is configured in settings rather than the environment, so the
  detector cannot see it.

## Persistence

`cli/src/services/agent_trace_db/` stores one latest-state row per canonical
`claude:<session_id>` in `claude_session_auth_state`. The columns are listed in
[agent-trace-db-schema.md](agent-trace-db-schema.md).

- `ClaudeSessionAuthState` holds the session ID, the four detector enums
  (`auth_env_key` optional), and `observed_at_ms`. It has no field that can
  hold a credential value. Enums are mapped to strings only at the SQL boundary.
- `RepositoryAgentTraceDb::upsert_claude_session_auth_state` first looks up
  `008_claude_session_auth_state` in the `__sce_migrations` ledger. If it is
  absent, it returns `ClaudeSessionAuthStateUpsertOutcome::SkippedMissingMigration`
  without writing. This is the counterpart of the hook-runtime readiness that
  tolerates a pre-`008` ledger (see
  [agent-trace-db.md](agent-trace-db.md#migration-008-compatibility-note)).
- Otherwise it runs one `INSERT ... ON CONFLICT (session_id) DO UPDATE ... WHERE`
  statement that changes the row only when `provider`, `billing_mode`,
  `confidence`, or `auth_env_key` (compared null-safely with `IS NOT`) differs.
  There is no explicit transaction and no read of the row first. It returns
  `Upserted { rows_changed }`, where `0` means an identical replay that left the
  row untouched. So `observed_at_ms` is when the current classification was
  first seen. A changed classification updates the row in place, and no history
  is kept.
- `claude_session_auth_state_by_session(session_id)` returns the exact session's
  typed row or `None`. Parsing uses the detector `from_str` mappings, so an
  unknown stored value is an error, not a silent fallback.
