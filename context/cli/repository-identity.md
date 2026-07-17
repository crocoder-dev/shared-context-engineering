# Repository identity canonicalization and hashing

Pure module at `cli/src/services/repository_identity.rs` that turns an explicit configured identity or a Git remote URL into a scheme-neutral canonical identity, then derives a stable repository ID as `sha256("sce-repository-id-v1\0" + canonical_identity)` lowercase hex (64 chars). Implemented in T02 of the `repository-scoped-agent-trace-db` plan; the repository-scoped Agent Trace DB path `<state_root>/sce/repos/<repository-id>/agent-trace.db` will be selected by this ID in later tasks.

The module performs no I/O: it never opens databases, reads Git config, or touches the filesystem. Runtime resolution (config precedence, Git remote lookup) is a separate later seam (T03).

## Public API

- `RepositoryIdentity { canonical_identity, repository_id }` — safe canonical form plus its hash.
- `repository_identity_from_explicit(raw)` — for `agent_trace.repository_id` config values; canonicalization is trimming only (explicit identities are operator-chosen opaque strings, not URL-parsed). Empty after trim → `EmptyExplicitIdentity`.
- `repository_identity_from_remote_url(raw)` — canonicalizes a Git remote URL, then hashes.
- `canonicalize_remote_url(raw) -> Result<String, RepositoryIdentityError>` — canonicalization without hashing.
- `derive_repository_id(canonical_identity) -> String` — domain-separated SHA-256 hex.
- `REPOSITORY_ID_HASH_DOMAIN` — the `b"sce-repository-id-v1\0"` prefix constant.

## Canonicalization rules (remote URLs)

Canonical form is scheme-neutral `host[:port]/path` so equivalent SSH/SCP/HTTPS remotes converge:

- Supported scheme URLs: `ssh://`, `git+ssh://`, `ssh+git://`, `http://`, `https://`, `git://`. Anything else with `://` (e.g. `file://`) → `UnsupportedRemoteUrl`.
- SCP-style `[user@]host:path` is supported when the first `:` precedes any `/`; it implies SSH with no port component. Inputs without `://` that don't match this shape (e.g. local paths) → `UnsupportedRemoteUrl`.
- Userinfo (including credentials) is stripped at the last `@` of the authority.
- Hostname is lowercased; path case is preserved. Bracketed IPv6 hosts are supported and kept bracketed.
- Default ports are removed per scheme (ssh 22, http 80, https 443, git 9418); non-default ports are preserved as `host:port`.
- Path cleanup: query (`?...`) and fragment (`#...`) dropped, leading/trailing slashes trimmed, one trailing `.git` stripped, then trailing slashes trimmed again. Empty result → `MissingPath`.

Example: `git@GitHub.com:Acme/Widgets.git`, `ssh://git@github.com:22/Acme/Widgets.git`, and `https://user:pass@github.com/Acme/Widgets.git?x#y` all canonicalize to `github.com/Acme/Widgets` and hash to the same repository ID.

## Credential-safety contract

- The returned canonical identity and repository ID never contain userinfo/credentials.
- `RepositoryIdentityError` variants (`EmptyExplicitIdentity`, `EmptyRemoteUrl`, `UnsupportedRemoteUrl`, `MissingHost`, `MissingPath`, `InvalidPort`) are fieldless and their `Display` messages never echo the raw input, so credential-bearing URLs cannot leak through diagnostics.

## Status

Registered in `cli/src/services/mod.rs` behind `#[allow(dead_code)]` until the T03 runtime resolver consumes it (same pattern as `bash_policy`). Covered by in-module unit tests (`cargo test repository_identity` filter; repo-preferred path is `nix flake check` / `nix build .#checks.<system>.cli-tests`).

See also: [config-precedence-contract.md](config-precedence-contract.md) (owns the `agent_trace.repository_id` / `agent_trace.repository_remote` config keys), [checkout-identity.md](checkout-identity.md), [../sce/agent-trace-db.md](../sce/agent-trace-db.md).
