# Repository identity canonicalization and hashing

Module at `cli/src/services/repository_identity/` that turns an explicit configured identity or a Git remote URL into a scheme-neutral canonical identity, then derives a stable repository ID as `sha256("sce-repository-id-v1\0" + canonical_identity)` lowercase hex (64 chars). Implemented in T02/T03 of the `repository-scoped-agent-trace-db` plan; the T04 storage resolver in `cli/src/services/agent_trace_storage/` selects the repository-scoped Agent Trace DB path `<state_root>/sce/repos/<repository-id>/agent-trace.db` by this ID (see [agent-trace-storage.md](agent-trace-storage.md)).

The root module (`mod.rs`) performs no I/O: it never opens databases, reads Git config, or touches the filesystem. Runtime precedence resolution and Git remote lookup live in the `resolve` submodule.

## Public API

- `RepositoryIdentity { canonical_identity, repository_id }` — safe canonical form plus its hash.
- `repository_identity_from_explicit(raw)` — for `agent_trace.repository_id` config values; canonicalization is trimming only (explicit identities are operator-chosen opaque strings, not URL-parsed). Empty after trim → `EmptyExplicitIdentity`.
- `repository_identity_from_remote_url(raw)` — canonicalizes a Git remote URL, then hashes.
- `canonicalize_remote_url(raw) -> Result<String, RepositoryIdentityError>` — canonicalization without hashing.
- `derive_repository_id(canonical_identity) -> String` — domain-separated SHA-256 hex.
- `REPOSITORY_ID_HASH_DOMAIN` — the `b"sce-repository-id-v1\0"` prefix constant.
- `repository_dir_segment(canonical_identity) -> String` — pure human-readable on-disk directory segment `<slug>-<short>`, where `slug` is the lowercased canonical identity with non-alphanumeric runs collapsed to a single `-` and leading/trailing `-` trimmed, and `short` is the first 4 hex chars of `SHA256(canonical_identity)` with **no** domain prefix (deliberately distinct from `repository_id`, which keeps its `sce-repository-id-v1\0` prefix). All-non-alphanumeric input slugs to empty and the segment falls back to just `short`. Display/layout helper only — the authoritative identity stays `repository_id`. Not yet wired into path construction (T02 of `human-readable-repo-db-directory`).
- `RepositoryIdentity::dir_segment(&self) -> String` — convenience wrapper over `repository_dir_segment` for `self.canonical_identity`.

## Canonicalization rules (remote URLs)

Canonical form is scheme-neutral `host[:port]/path` so equivalent SSH/SCP/HTTPS remotes converge:

- Supported scheme URLs: `ssh://`, `git+ssh://`, `ssh+git://`, `http://`, `https://`, `git://`. Anything else with `://` (e.g. `file://`) → `UnsupportedRemoteUrl`.
- SCP-style `[user@]host:path` is supported when the first `:` precedes any `/`; it implies SSH with no port component. Inputs without `://` that don't match this shape (e.g. local paths) → `UnsupportedRemoteUrl`.
- Userinfo (including credentials) is stripped at the last `@` of the authority.
- Hostname is lowercased; path case is preserved. Bracketed IPv6 hosts are supported and kept bracketed.
- Default ports are removed per scheme (ssh 22, http 80, https 443, git 9418); non-default ports are preserved as `host:port`.
- Path cleanup: query (`?...`) and fragment (`#...`) dropped, leading/trailing slashes trimmed, one trailing `.git` stripped, then trailing slashes trimmed again. Empty result → `MissingPath`.

Example: `git@GitHub.com:Acme/Widgets.git`, `ssh://git@github.com:22/Acme/Widgets.git`, and `https://user:pass@github.com/Acme/Widgets.git?x#y` all canonicalize to `github.com/Acme/Widgets` and hash to the same repository ID.

## Runtime resolution (`resolve` submodule)

`repository_identity/resolve.rs` applies the repository identity precedence at runtime:

1. Explicit `agent_trace.repository_id` config value (trim-only canonicalization; invalid explicit values error, they do not fall back to remotes).
2. URL of the configured Git remote (`agent_trace.repository_remote`, default `origin`), read via `git config --get remote.<name>.url`.
3. Otherwise an actionable error pointing at `.sce/config.json`.

- `resolve_repository_identity(repository_root, explicit_identity, remote_name)` — process-spawning entrypoint.
- `resolve_repository_identity_with_lookup(explicit, remote_name, lookup)` — precedence core with injectable remote lookup for tests/callers.
- `lookup_remote_url(repository_root, remote_name) -> Option<String>` — returns `None` when git is unavailable, the directory is not a repository, or the remote has no URL.
- `ResolvedRepositoryIdentity { identity, source }` with `RepositoryIdentitySource::{ExplicitConfig, RemoteUrl { remote_name }}` — source is retained for later diagnostics rendering (T10).
- `RepositoryIdentityResolutionError::{InvalidExplicitIdentity, InvalidRemoteUrl, MissingIdentity}` — every `Display` message includes `.sce/config.json` guidance naming the `agent_trace.*` keys; variants carry only the configured remote name, never URLs or identity values.

Local paths are never used implicitly: a local-path remote URL fails canonicalization and surfaces as `InvalidRemoteUrl` rather than falling back.

## Credential-safety contract

- The returned canonical identity and repository ID never contain userinfo/credentials.
- `RepositoryIdentityError` variants (`EmptyExplicitIdentity`, `EmptyRemoteUrl`, `UnsupportedRemoteUrl`, `MissingHost`, `MissingPath`, `InvalidPort`) are fieldless and their `Display` messages never echo the raw input, so credential-bearing URLs cannot leak through diagnostics.
- `RepositoryIdentityResolutionError` follows the same rule: it never echoes remote URLs or explicit identity values; only operator-chosen remote names appear in messages.

## Status

Registered in `cli/src/services/mod.rs`; consumed by the `agent_trace_storage` resolver, which is now used by active hook runtime and Agent Trace lifecycle setup/health. Covered by in-module unit tests, including temp-Git-repo remote lookup tests (repo-preferred path is `nix flake check` / `nix build .#checks.<system>.cli-tests`).

See also: [config-precedence-contract.md](config-precedence-contract.md) (owns the `agent_trace.repository_id` / `agent_trace.repository_remote` config keys), [checkout-identity.md](checkout-identity.md), [../sce/agent-trace-db.md](../sce/agent-trace-db.md).
