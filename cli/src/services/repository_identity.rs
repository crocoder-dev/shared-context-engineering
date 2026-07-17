//! Pure repository identity canonicalization and hashing.
//!
//! Turns an explicit configured identity or a Git remote URL into a
//! scheme-neutral canonical identity, then derives a stable repository ID as
//! `sha256("sce-repository-id-v1\0" + canonical_identity)` hex.
//!
//! This module performs no I/O: it never opens databases, reads Git config,
//! or touches the filesystem. Errors intentionally never echo the raw input
//! so credential-bearing remote URLs cannot leak through diagnostics.

use sha2::{Digest, Sha256};

/// Domain-separation prefix hashed before the canonical identity.
pub const REPOSITORY_ID_HASH_DOMAIN: &[u8] = b"sce-repository-id-v1\0";

/// A resolved repository identity: the safe canonical form plus its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryIdentity {
    /// Credential-free canonical identity, safe to display and store.
    pub canonical_identity: String,
    /// Lowercase hex SHA-256 of the domain prefix plus canonical identity.
    pub repository_id: String,
}

/// Canonicalization failure. Variants carry no input fragments so
/// credential-bearing URLs never leak into error output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryIdentityError {
    /// Explicit identity was empty after trimming whitespace.
    EmptyExplicitIdentity,
    /// Remote URL was empty after trimming whitespace.
    EmptyRemoteUrl,
    /// Remote URL scheme is not a supported Git transport.
    UnsupportedRemoteUrl,
    /// Remote URL has no usable host component.
    MissingHost,
    /// Remote URL has no usable repository path component.
    MissingPath,
    /// Remote URL port component is not a valid number.
    InvalidPort,
}

impl std::fmt::Display for RepositoryIdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::EmptyExplicitIdentity => "explicit repository identity is empty",
            Self::EmptyRemoteUrl => "remote URL is empty",
            Self::UnsupportedRemoteUrl => {
                "remote URL is not a supported Git transport (ssh, scp-style ssh, http, https, git)"
            }
            Self::MissingHost => "remote URL has no host",
            Self::MissingPath => "remote URL has no repository path",
            Self::InvalidPort => "remote URL has an invalid port",
        };
        f.write_str(message)
    }
}

impl std::error::Error for RepositoryIdentityError {}

/// Builds a repository identity from an explicitly configured identity
/// string (`agent_trace.repository_id`). Canonicalization is trimming only:
/// explicit identities are operator-chosen opaque values, not URLs.
pub fn repository_identity_from_explicit(
    raw: &str,
) -> Result<RepositoryIdentity, RepositoryIdentityError> {
    let canonical = raw.trim();
    if canonical.is_empty() {
        return Err(RepositoryIdentityError::EmptyExplicitIdentity);
    }
    Ok(identity_from_canonical(canonical.to_string()))
}

/// Builds a repository identity from a Git remote URL. Equivalent SSH,
/// SCP-style, and HTTPS URLs canonicalize to the same identity.
pub fn repository_identity_from_remote_url(
    raw: &str,
) -> Result<RepositoryIdentity, RepositoryIdentityError> {
    let canonical = canonicalize_remote_url(raw)?;
    Ok(identity_from_canonical(canonical))
}

/// Derives the repository ID hex digest for a canonical identity.
pub fn derive_repository_id(canonical_identity: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(REPOSITORY_ID_HASH_DOMAIN);
    hasher.update(canonical_identity.as_bytes());
    hex_encode(&hasher.finalize())
}

/// Canonicalizes a Git remote URL to the scheme-neutral form
/// `host[:port]/path` with credentials stripped, hostname lowercased,
/// default ports removed, and query/fragment/trailing-slash/trailing-`.git`
/// cleaned up. The returned string never contains credentials.
pub fn canonicalize_remote_url(raw: &str) -> Result<String, RepositoryIdentityError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(RepositoryIdentityError::EmptyRemoteUrl);
    }

    if let Some((scheme, rest)) = trimmed.split_once("://") {
        canonicalize_scheme_url(scheme, rest)
    } else {
        canonicalize_scp_style(trimmed)
    }
}

fn identity_from_canonical(canonical_identity: String) -> RepositoryIdentity {
    let repository_id = derive_repository_id(&canonical_identity);
    RepositoryIdentity {
        canonical_identity,
        repository_id,
    }
}

fn canonicalize_scheme_url(scheme: &str, rest: &str) -> Result<String, RepositoryIdentityError> {
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "ssh" | "git+ssh" | "ssh+git" => Some(22),
        "http" => Some(80),
        "https" => Some(443),
        "git" => Some(9418),
        _ => return Err(RepositoryIdentityError::UnsupportedRemoteUrl),
    };

    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, path),
        None => (rest, ""),
    };

    let host_port = strip_userinfo(authority);
    let (host, port) = split_host_port(host_port)?;
    if host.is_empty() {
        return Err(RepositoryIdentityError::MissingHost);
    }

    let path = clean_path(path)?;
    Ok(render_canonical(&host, port, default_port, &path))
}

fn canonicalize_scp_style(input: &str) -> Result<String, RepositoryIdentityError> {
    // SCP-style form: [user@]host:path — the colon must come before any '/'.
    let host_port_end = input.find(':');
    let first_slash = input.find('/');
    let colon = match (host_port_end, first_slash) {
        (Some(colon), Some(slash)) if colon < slash => colon,
        (Some(colon), None) => colon,
        _ => return Err(RepositoryIdentityError::UnsupportedRemoteUrl),
    };

    let authority = &input[..colon];
    let path = &input[colon + 1..];

    let host = strip_userinfo(authority).to_ascii_lowercase();
    if host.is_empty() {
        return Err(RepositoryIdentityError::MissingHost);
    }
    // SCP-style implies SSH on the default port; no port component exists.
    let path = clean_path(path)?;
    Ok(format!("{host}/{path}"))
}

fn strip_userinfo(authority: &str) -> &str {
    match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    }
}

fn split_host_port(host_port: &str) -> Result<(String, Option<u16>), RepositoryIdentityError> {
    // IPv6 literals are bracketed: [::1]:2222
    if let Some(rest) = host_port.strip_prefix('[') {
        let Some(close) = rest.find(']') else {
            return Err(RepositoryIdentityError::MissingHost);
        };
        let host = rest[..close].to_ascii_lowercase();
        let after = &rest[close + 1..];
        if after.is_empty() {
            return Ok((format!("[{host}]"), None));
        }
        let Some(port) = after.strip_prefix(':') else {
            return Err(RepositoryIdentityError::InvalidPort);
        };
        let port = parse_port(port)?;
        return Ok((format!("[{host}]"), Some(port)));
    }

    match host_port.rsplit_once(':') {
        Some((host, port)) => Ok((host.to_ascii_lowercase(), Some(parse_port(port)?))),
        None => Ok((host_port.to_ascii_lowercase(), None)),
    }
}

fn parse_port(port: &str) -> Result<u16, RepositoryIdentityError> {
    port.parse::<u16>()
        .map_err(|_| RepositoryIdentityError::InvalidPort)
}

fn clean_path(path: &str) -> Result<String, RepositoryIdentityError> {
    let path = path.split(['?', '#']).next().unwrap_or("");
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let path = path.trim_matches('/');
    if path.is_empty() {
        return Err(RepositoryIdentityError::MissingPath);
    }
    Ok(path.to_string())
}

fn render_canonical(
    host: &str,
    port: Option<u16>,
    default_port: Option<u16>,
    path: &str,
) -> String {
    match port {
        Some(port) if Some(port) != default_port => format!("{host}:{port}/{path}"),
        _ => format!("{host}/{path}"),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut hex = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(raw: &str) -> String {
        canonicalize_remote_url(raw).expect("expected canonicalization to succeed")
    }

    #[test]
    fn equivalent_github_urls_share_canonical_identity_and_id() {
        let forms = [
            "git@github.com:CroCoder/shared-context-engineering.git",
            "ssh://git@github.com/CroCoder/shared-context-engineering.git",
            "ssh://git@github.com:22/CroCoder/shared-context-engineering.git",
            "https://github.com/CroCoder/shared-context-engineering.git",
            "https://github.com:443/CroCoder/shared-context-engineering",
            "https://GitHub.com/CroCoder/shared-context-engineering.git/",
            "https://token@github.com/CroCoder/shared-context-engineering.git?ref=main#readme",
        ];

        let expected = "github.com/CroCoder/shared-context-engineering";
        let expected_id = derive_repository_id(expected);
        for form in forms {
            let identity = repository_identity_from_remote_url(form)
                .expect("expected identity resolution to succeed");
            assert_eq!(identity.canonical_identity, expected, "input: {form}");
            assert_eq!(identity.repository_id, expected_id, "input: {form}");
        }
    }

    #[test]
    fn repository_id_uses_domain_separated_sha256() {
        let identity = repository_identity_from_explicit("acme/widgets")
            .expect("expected explicit identity to resolve");
        let mut hasher = Sha256::new();
        hasher.update(b"sce-repository-id-v1\0");
        hasher.update(b"acme/widgets");
        let expected = hex_encode(&hasher.finalize());
        assert_eq!(identity.repository_id, expected);
        assert_eq!(identity.repository_id.len(), 64);
    }

    #[test]
    fn distinct_identities_hash_differently() {
        let a = repository_identity_from_remote_url("git@github.com:acme/widgets.git")
            .expect("expected identity resolution to succeed");
        let b = repository_identity_from_remote_url("git@github.com:acme/gadgets.git")
            .expect("expected identity resolution to succeed");
        let c = repository_identity_from_remote_url("git@gitlab.com:acme/widgets.git")
            .expect("expected identity resolution to succeed");
        assert_ne!(a.repository_id, b.repository_id);
        assert_ne!(a.repository_id, c.repository_id);
        assert_ne!(b.repository_id, c.repository_id);
    }

    #[test]
    fn credentials_are_stripped_and_never_leak() {
        let secret_forms = [
            "https://alice:s3cr3t@github.com/acme/widgets.git",
            "ssh://alice:s3cr3t@github.com:22/acme/widgets.git",
            "alice@github.com:acme/widgets.git",
        ];
        for form in secret_forms {
            let identity = repository_identity_from_remote_url(form)
                .expect("expected identity resolution to succeed");
            assert_eq!(identity.canonical_identity, "github.com/acme/widgets");
            assert!(!identity.canonical_identity.contains("alice"));
            assert!(!identity.canonical_identity.contains("s3cr3t"));
            assert!(!identity.repository_id.contains("s3cr3t"));
        }
    }

    #[test]
    fn errors_do_not_echo_input() {
        let cases = [
            ("", RepositoryIdentityError::EmptyRemoteUrl),
            (
                "file:///alice:s3cr3t/repo.git",
                RepositoryIdentityError::UnsupportedRemoteUrl,
            ),
            (
                "/local/path/to/s3cr3t-repo",
                RepositoryIdentityError::UnsupportedRemoteUrl,
            ),
            (
                "https://alice:s3cr3t@github.com",
                RepositoryIdentityError::MissingPath,
            ),
            (
                "https://alice:s3cr3t@/acme/widgets.git",
                RepositoryIdentityError::MissingHost,
            ),
            (
                "https://github.com:port/acme/widgets.git",
                RepositoryIdentityError::InvalidPort,
            ),
        ];
        for (input, expected) in cases {
            let error =
                canonicalize_remote_url(input).expect_err("expected canonicalization error");
            assert_eq!(error, expected, "input: {input}");
            let rendered = error.to_string();
            assert!(
                !rendered.contains("s3cr3t"),
                "error leaked input: {rendered}"
            );
        }
    }

    #[test]
    fn non_default_ports_are_preserved() {
        assert_eq!(
            canonical("ssh://git@github.com:2222/acme/widgets.git"),
            "github.com:2222/acme/widgets"
        );
        assert_eq!(
            canonical("https://github.com:8443/acme/widgets.git"),
            "github.com:8443/acme/widgets"
        );
        assert_eq!(
            canonical("git://github.com:9418/acme/widgets.git"),
            "github.com/acme/widgets"
        );
        assert_eq!(
            canonical("http://github.com:80/acme/widgets.git"),
            "github.com/acme/widgets"
        );
    }

    #[test]
    fn hostnames_are_lowercased_but_paths_preserved() {
        assert_eq!(
            canonical("Git@GitHub.COM:Acme/Widgets.git"),
            "github.com/Acme/Widgets"
        );
    }

    #[test]
    fn query_fragment_and_trailing_cleanup() {
        assert_eq!(
            canonical("https://github.com/acme/widgets.git?depth=1"),
            "github.com/acme/widgets"
        );
        assert_eq!(
            canonical("https://github.com/acme/widgets#fragment"),
            "github.com/acme/widgets"
        );
        assert_eq!(
            canonical("https://github.com/acme/widgets///"),
            "github.com/acme/widgets"
        );
        assert_eq!(
            canonical("https://github.com/acme/widgets.git/"),
            "github.com/acme/widgets"
        );
    }

    #[test]
    fn scp_style_requires_colon_before_slash() {
        assert_eq!(
            canonicalize_remote_url("github.com/acme/widgets:tag"),
            Err(RepositoryIdentityError::UnsupportedRemoteUrl)
        );
        assert_eq!(
            canonical("git@github.com:acme/widgets"),
            "github.com/acme/widgets"
        );
    }

    #[test]
    fn ipv6_hosts_are_supported() {
        assert_eq!(
            canonical("ssh://git@[2001:DB8::1]:2222/acme/widgets.git"),
            "[2001:db8::1]:2222/acme/widgets"
        );
        assert_eq!(
            canonical("ssh://git@[2001:db8::1]/acme/widgets.git"),
            "[2001:db8::1]/acme/widgets"
        );
    }

    #[test]
    fn explicit_identity_is_trimmed_and_used_verbatim() {
        let identity = repository_identity_from_explicit("  my-monorepo  ")
            .expect("expected explicit identity to resolve");
        assert_eq!(identity.canonical_identity, "my-monorepo");
        assert_eq!(
            repository_identity_from_explicit("   "),
            Err(RepositoryIdentityError::EmptyExplicitIdentity)
        );
    }

    #[test]
    fn missing_path_variants_error() {
        assert_eq!(
            canonicalize_remote_url("https://github.com/"),
            Err(RepositoryIdentityError::MissingPath)
        );
        assert_eq!(
            canonicalize_remote_url("git@github.com:"),
            Err(RepositoryIdentityError::MissingPath)
        );
        assert_eq!(
            canonicalize_remote_url("https://github.com/.git"),
            Err(RepositoryIdentityError::MissingPath)
        );
    }
}
