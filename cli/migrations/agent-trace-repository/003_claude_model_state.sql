CREATE TABLE IF NOT EXISTS claude_model_state (
    session_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    observation_kind TEXT NOT NULL CHECK (observation_kind IN ('session_start', 'post_model_switch')),
    source TEXT NOT NULL,
    observed_at_ms INTEGER NOT NULL CHECK (observed_at_ms >= 0),
    PRIMARY KEY (session_id, agent_id)
);
