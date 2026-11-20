use anyhow::Result;

const CLAUDE_CODE_USE_BEDROCK: &str = "CLAUDE_CODE_USE_BEDROCK";
const CLAUDE_CODE_USE_VERTEX: &str = "CLAUDE_CODE_USE_VERTEX";
const CLAUDE_CODE_USE_FOUNDRY: &str = "CLAUDE_CODE_USE_FOUNDRY";
const ANTHROPIC_AUTH_TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";
const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
const ANTHROPIC_BASE_URL: &str = "ANTHROPIC_BASE_URL";
const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthProvider {
    Anthropic,
    AwsBedrock,
    GoogleVertex,
    MicrosoftFoundry,
    CustomGateway,
    Unknown,
}

impl AuthProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::AwsBedrock => "aws_bedrock",
            Self::GoogleVertex => "google_vertex",
            Self::MicrosoftFoundry => "microsoft_foundry",
            Self::CustomGateway => "custom_gateway",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_str(value: &str) -> Result<Self> {
        match value {
            "anthropic" => Ok(Self::Anthropic),
            "aws_bedrock" => Ok(Self::AwsBedrock),
            "google_vertex" => Ok(Self::GoogleVertex),
            "microsoft_foundry" => Ok(Self::MicrosoftFoundry),
            "custom_gateway" => Ok(Self::CustomGateway),
            "unknown" => Ok(Self::Unknown),
            other => anyhow::bail!("invalid Claude auth provider: {other}"),
        }
    }
}

/// `Subscription` is a valid stored value, but no environment-only signal can
/// establish it, so the detector never returns it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BillingMode {
    ApiKey,
    Subscription,
    CloudProvider,
    Unknown,
}

impl BillingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Subscription => "subscription",
            Self::CloudProvider => "cloud_provider",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_str(value: &str) -> Result<Self> {
        match value {
            "api_key" => Ok(Self::ApiKey),
            "subscription" => Ok(Self::Subscription),
            "cloud_provider" => Ok(Self::CloudProvider),
            "unknown" => Ok(Self::Unknown),
            other => anyhow::bail!("invalid Claude billing mode: {other}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthEnvKey {
    ClaudeCodeUseBedrock,
    ClaudeCodeUseVertex,
    ClaudeCodeUseFoundry,
    AnthropicAuthToken,
    AnthropicApiKey,
    ClaudeCodeOauthToken,
}

impl AuthEnvKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCodeUseBedrock => "claude_code_use_bedrock",
            Self::ClaudeCodeUseVertex => "claude_code_use_vertex",
            Self::ClaudeCodeUseFoundry => "claude_code_use_foundry",
            Self::AnthropicAuthToken => "anthropic_auth_token",
            Self::AnthropicApiKey => "anthropic_api_key",
            Self::ClaudeCodeOauthToken => "claude_code_oauth_token",
        }
    }

    pub fn from_str(value: &str) -> Result<Self> {
        match value {
            "claude_code_use_bedrock" => Ok(Self::ClaudeCodeUseBedrock),
            "claude_code_use_vertex" => Ok(Self::ClaudeCodeUseVertex),
            "claude_code_use_foundry" => Ok(Self::ClaudeCodeUseFoundry),
            "anthropic_auth_token" => Ok(Self::AnthropicAuthToken),
            "anthropic_api_key" => Ok(Self::AnthropicApiKey),
            "claude_code_oauth_token" => Ok(Self::ClaudeCodeOauthToken),
            other => anyhow::bail!("invalid Claude auth env key: {other}"),
        }
    }
}

/// Environment variables describe configuration, not proof of authentication,
/// so there is no verified level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetectionConfidence {
    Inferred,
    Unknown,
}

impl DetectionConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inferred => "inferred",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_str(value: &str) -> Result<Self> {
        match value {
            "inferred" => Ok(Self::Inferred),
            "unknown" => Ok(Self::Unknown),
            other => anyhow::bail!("invalid Claude auth detection confidence: {other}"),
        }
    }
}

/// Presence of each documented authentication signal; never holds values.
#[allow(
    clippy::struct_excessive_bools,
    reason = "each field is an independent signal-presence flag"
)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClaudeAuthSignals {
    pub use_bedrock: bool,
    pub use_vertex: bool,
    pub use_foundry: bool,
    pub anthropic_auth_token: bool,
    pub anthropic_api_key: bool,
    pub claude_code_oauth_token: bool,
    pub custom_base_url: bool,
}

impl ClaudeAuthSignals {
    /// Each looked-up value is dropped immediately after its presence check.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let truthy = |name: &str| lookup(name).is_some_and(|value| is_truthy_flag(&value));
        let non_empty = |name: &str| lookup(name).is_some_and(|value| !value.trim().is_empty());

        Self {
            use_bedrock: truthy(CLAUDE_CODE_USE_BEDROCK),
            use_vertex: truthy(CLAUDE_CODE_USE_VERTEX),
            use_foundry: truthy(CLAUDE_CODE_USE_FOUNDRY),
            anthropic_auth_token: non_empty(ANTHROPIC_AUTH_TOKEN),
            anthropic_api_key: non_empty(ANTHROPIC_API_KEY),
            claude_code_oauth_token: non_empty(CLAUDE_CODE_OAUTH_TOKEN),
            custom_base_url: lookup(ANTHROPIC_BASE_URL)
                .is_some_and(|value| is_custom_base_url(&value)),
        }
    }
}

fn is_truthy_flag(value: &str) -> bool {
    let value = value.trim();
    value == "1" || value.eq_ignore_ascii_case("true")
}

fn is_custom_base_url(value: &str) -> bool {
    let value = value.trim().trim_end_matches('/');
    !value.is_empty() && !value.eq_ignore_ascii_case(DEFAULT_ANTHROPIC_BASE_URL)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClaudeAuthDetection {
    pub provider: AuthProvider,
    pub billing_mode: BillingMode,
    pub auth_env_key: Option<AuthEnvKey>,
    pub confidence: DetectionConfidence,
}

impl ClaudeAuthDetection {
    const UNKNOWN: Self = Self {
        provider: AuthProvider::Unknown,
        billing_mode: BillingMode::Unknown,
        auth_env_key: None,
        confidence: DetectionConfidence::Unknown,
    };

    fn inferred(
        provider: AuthProvider,
        billing_mode: BillingMode,
        auth_env_key: AuthEnvKey,
    ) -> Self {
        Self {
            provider,
            billing_mode,
            auth_env_key: Some(auth_env_key),
            confidence: DetectionConfidence::Inferred,
        }
    }
}

/// Applies Claude Code's documented precedence: cloud-provider flags, then
/// `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY`, and `CLAUDE_CODE_OAUTH_TOKEN`.
/// More than one cloud flag is a provider conflict and classifies as unknown.
pub fn detect_claude_auth(signals: ClaudeAuthSignals) -> ClaudeAuthDetection {
    let cloud_flags = [
        (
            signals.use_bedrock,
            AuthProvider::AwsBedrock,
            AuthEnvKey::ClaudeCodeUseBedrock,
        ),
        (
            signals.use_vertex,
            AuthProvider::GoogleVertex,
            AuthEnvKey::ClaudeCodeUseVertex,
        ),
        (
            signals.use_foundry,
            AuthProvider::MicrosoftFoundry,
            AuthEnvKey::ClaudeCodeUseFoundry,
        ),
    ];
    let mut set_cloud_flags = cloud_flags.iter().filter(|(set, _, _)| *set);
    match (set_cloud_flags.next(), set_cloud_flags.next()) {
        (Some(_), Some(_)) => return ClaudeAuthDetection::UNKNOWN,
        (Some(&(_, provider, key)), None) => {
            return ClaudeAuthDetection::inferred(provider, BillingMode::CloudProvider, key);
        }
        (None, _) => {}
    }

    let anthropic_or_gateway = if signals.custom_base_url {
        AuthProvider::CustomGateway
    } else {
        AuthProvider::Anthropic
    };

    if signals.anthropic_auth_token {
        return ClaudeAuthDetection::inferred(
            anthropic_or_gateway,
            BillingMode::Unknown,
            AuthEnvKey::AnthropicAuthToken,
        );
    }

    if signals.anthropic_api_key {
        let billing_mode = if signals.custom_base_url {
            BillingMode::Unknown
        } else {
            BillingMode::ApiKey
        };
        return ClaudeAuthDetection::inferred(
            anthropic_or_gateway,
            billing_mode,
            AuthEnvKey::AnthropicApiKey,
        );
    }

    if signals.claude_code_oauth_token {
        return ClaudeAuthDetection::inferred(
            AuthProvider::Anthropic,
            BillingMode::Unknown,
            AuthEnvKey::ClaudeCodeOauthToken,
        );
    }

    ClaudeAuthDetection::UNKNOWN
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        detect_claude_auth, AuthEnvKey, AuthProvider, BillingMode, ClaudeAuthDetection,
        ClaudeAuthSignals, DetectionConfidence,
    };

    fn detect(vars: &[(&str, &str)]) -> ClaudeAuthDetection {
        let env: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        detect_claude_auth(ClaudeAuthSignals::from_env_lookup(|name| {
            env.get(name).cloned()
        }))
    }

    fn inferred(
        provider: AuthProvider,
        billing_mode: BillingMode,
        key: AuthEnvKey,
    ) -> ClaudeAuthDetection {
        ClaudeAuthDetection {
            provider,
            billing_mode,
            auth_env_key: Some(key),
            confidence: DetectionConfidence::Inferred,
        }
    }

    const ALL_ANTHROPIC_CREDENTIALS: [(&str, &str); 4] = [
        ("ANTHROPIC_AUTH_TOKEN", "token"),
        ("ANTHROPIC_API_KEY", "key"),
        ("CLAUDE_CODE_OAUTH_TOKEN", "oauth"),
        ("ANTHROPIC_BASE_URL", "https://gateway.example.com"),
    ];

    #[test]
    fn each_cloud_flag_yields_its_provider_with_cloud_billing() {
        let cases = [
            (
                "CLAUDE_CODE_USE_BEDROCK",
                AuthProvider::AwsBedrock,
                AuthEnvKey::ClaudeCodeUseBedrock,
            ),
            (
                "CLAUDE_CODE_USE_VERTEX",
                AuthProvider::GoogleVertex,
                AuthEnvKey::ClaudeCodeUseVertex,
            ),
            (
                "CLAUDE_CODE_USE_FOUNDRY",
                AuthProvider::MicrosoftFoundry,
                AuthEnvKey::ClaudeCodeUseFoundry,
            ),
        ];
        for (flag, provider, key) in cases {
            let expected = inferred(provider, BillingMode::CloudProvider, key);
            assert_eq!(detect(&[(flag, "1")]), expected, "{flag}=1");
            assert_eq!(detect(&[(flag, "TRUE")]), expected, "{flag}=TRUE");

            let mut vars = ALL_ANTHROPIC_CREDENTIALS.to_vec();
            vars.push((flag, "true"));
            assert_eq!(detect(&vars), expected, "{flag} over Anthropic credentials");
        }
    }

    #[test]
    fn falsy_cloud_flag_is_not_a_signal() {
        for value in ["0", "false", "", "yes"] {
            assert_eq!(
                detect(&[("CLAUDE_CODE_USE_BEDROCK", value)]),
                ClaudeAuthDetection::UNKNOWN,
                "CLAUDE_CODE_USE_BEDROCK={value:?}"
            );
        }
    }

    #[test]
    fn multiple_cloud_flags_are_a_provider_conflict() {
        assert_eq!(
            detect(&[
                ("CLAUDE_CODE_USE_BEDROCK", "1"),
                ("CLAUDE_CODE_USE_VERTEX", "1"),
                ("ANTHROPIC_API_KEY", "key"),
            ]),
            ClaudeAuthDetection::UNKNOWN
        );
    }

    #[test]
    fn auth_token_takes_precedence_over_api_key_with_unknown_billing() {
        assert_eq!(
            detect(&[
                ("ANTHROPIC_AUTH_TOKEN", "token"),
                ("ANTHROPIC_API_KEY", "key"),
                ("CLAUDE_CODE_OAUTH_TOKEN", "oauth"),
            ]),
            inferred(
                AuthProvider::Anthropic,
                BillingMode::Unknown,
                AuthEnvKey::AnthropicAuthToken
            )
        );
    }

    #[test]
    fn api_key_without_custom_base_url_is_inferred_api_key_billing() {
        let expected = inferred(
            AuthProvider::Anthropic,
            BillingMode::ApiKey,
            AuthEnvKey::AnthropicApiKey,
        );
        assert_eq!(detect(&[("ANTHROPIC_API_KEY", "key")]), expected);
        assert_eq!(
            detect(&[
                ("ANTHROPIC_API_KEY", "key"),
                ("CLAUDE_CODE_OAUTH_TOKEN", "oauth"),
                ("ANTHROPIC_BASE_URL", " https://API.anthropic.com/ "),
            ]),
            expected
        );
    }

    #[test]
    fn custom_base_url_classifies_either_credential_as_custom_gateway() {
        let base_url = ("ANTHROPIC_BASE_URL", "https://gateway.example.com");
        assert_eq!(
            detect(&[("ANTHROPIC_AUTH_TOKEN", "token"), base_url]),
            inferred(
                AuthProvider::CustomGateway,
                BillingMode::Unknown,
                AuthEnvKey::AnthropicAuthToken
            )
        );
        assert_eq!(
            detect(&[("ANTHROPIC_API_KEY", "key"), base_url]),
            inferred(
                AuthProvider::CustomGateway,
                BillingMode::Unknown,
                AuthEnvKey::AnthropicApiKey
            )
        );
    }

    #[test]
    fn oauth_token_alone_is_anthropic_with_unknown_billing() {
        let expected = inferred(
            AuthProvider::Anthropic,
            BillingMode::Unknown,
            AuthEnvKey::ClaudeCodeOauthToken,
        );
        assert_eq!(detect(&[("CLAUDE_CODE_OAUTH_TOKEN", "oauth")]), expected);
        assert_eq!(
            detect(&[
                ("CLAUDE_CODE_OAUTH_TOKEN", "oauth"),
                ("ANTHROPIC_BASE_URL", "https://gateway.example.com"),
            ]),
            expected
        );
    }

    #[test]
    fn missing_or_empty_configuration_is_unknown() {
        assert_eq!(detect(&[]), ClaudeAuthDetection::UNKNOWN);
        assert_eq!(
            detect(&[
                ("ANTHROPIC_AUTH_TOKEN", ""),
                ("ANTHROPIC_API_KEY", "  "),
                ("CLAUDE_CODE_OAUTH_TOKEN", ""),
                ("ANTHROPIC_BASE_URL", "https://gateway.example.com"),
            ]),
            ClaudeAuthDetection::UNKNOWN
        );
    }

    #[test]
    fn detector_never_returns_subscription_billing_for_any_signal_combination() {
        for bits in 0u8..128 {
            let signals = ClaudeAuthSignals {
                use_bedrock: bits & 1 != 0,
                use_vertex: bits & 2 != 0,
                use_foundry: bits & 4 != 0,
                anthropic_auth_token: bits & 8 != 0,
                anthropic_api_key: bits & 16 != 0,
                claude_code_oauth_token: bits & 32 != 0,
                custom_base_url: bits & 64 != 0,
            };
            let detection = detect_claude_auth(signals);
            assert_ne!(detection.billing_mode, BillingMode::Subscription);
            assert_eq!(
                detection.auth_env_key.is_some(),
                detection.confidence == DetectionConfidence::Inferred
            );
        }
    }

    #[test]
    fn string_mapping_round_trips_and_rejects_unknown_values() {
        for provider in [
            AuthProvider::Anthropic,
            AuthProvider::AwsBedrock,
            AuthProvider::GoogleVertex,
            AuthProvider::MicrosoftFoundry,
            AuthProvider::CustomGateway,
            AuthProvider::Unknown,
        ] {
            assert_eq!(AuthProvider::from_str(provider.as_str()).unwrap(), provider);
        }
        for mode in [
            BillingMode::ApiKey,
            BillingMode::Subscription,
            BillingMode::CloudProvider,
            BillingMode::Unknown,
        ] {
            assert_eq!(BillingMode::from_str(mode.as_str()).unwrap(), mode);
        }
        for key in [
            AuthEnvKey::ClaudeCodeUseBedrock,
            AuthEnvKey::ClaudeCodeUseVertex,
            AuthEnvKey::ClaudeCodeUseFoundry,
            AuthEnvKey::AnthropicAuthToken,
            AuthEnvKey::AnthropicApiKey,
            AuthEnvKey::ClaudeCodeOauthToken,
        ] {
            assert_eq!(AuthEnvKey::from_str(key.as_str()).unwrap(), key);
        }
        for confidence in [DetectionConfidence::Inferred, DetectionConfidence::Unknown] {
            assert_eq!(
                DetectionConfidence::from_str(confidence.as_str()).unwrap(),
                confidence
            );
        }
        assert!(AuthProvider::from_str("verified").is_err());
        assert!(BillingMode::from_str("API_KEY").is_err());
        assert!(AuthEnvKey::from_str("anthropic_base_url").is_err());
        assert!(DetectionConfidence::from_str("verified").is_err());
    }
}
