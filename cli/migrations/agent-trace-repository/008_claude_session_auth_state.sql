CREATE TABLE IF NOT EXISTS claude_session_auth_state (
    session_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL CHECK (provider IN ('anthropic', 'aws_bedrock', 'google_vertex', 'microsoft_foundry', 'custom_gateway', 'unknown')),
    billing_mode TEXT NOT NULL CHECK (billing_mode IN ('api_key', 'subscription', 'cloud_provider', 'unknown')),
    auth_env_key TEXT CHECK (auth_env_key IS NULL OR auth_env_key IN ('claude_code_use_bedrock', 'claude_code_use_vertex', 'claude_code_use_foundry', 'anthropic_auth_token', 'anthropic_api_key', 'claude_code_oauth_token')),
    confidence TEXT NOT NULL CHECK (confidence IN ('inferred', 'unknown')),
    observed_at_ms INTEGER NOT NULL CHECK (observed_at_ms >= 0)
);
