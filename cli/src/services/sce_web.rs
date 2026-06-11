pub const SCE_WEB_BASE_URL: &str = "https://sce.crocoder.dev";

pub(crate) fn agent_trace_conversation_url(agent_trace_id: &str) -> String {
    format!("{SCE_WEB_BASE_URL}/conversations/{agent_trace_id}")
}

pub(crate) fn agent_trace_persisted_url(agent_trace_id: &str) -> String {
    format!("{}/trace/{agent_trace_id}", sce_web_host())
}

pub(crate) fn config_schema_url() -> String {
    format!("{SCE_WEB_BASE_URL}/config.json")
}

fn sce_web_host() -> &'static str {
    SCE_WEB_BASE_URL
        .strip_prefix("https://")
        .unwrap_or(SCE_WEB_BASE_URL)
}
