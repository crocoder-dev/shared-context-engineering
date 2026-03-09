use anyhow::{Context, Result};
use serde_json::json;

use crate::services::output_format::OutputFormat;

pub const NAME: &str = "mcp";

pub type McpFormat = OutputFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpRequest {
    pub format: McpFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpTransport {
    Stdio,
    LocalSocket,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpToolContract {
    pub tool_name: &'static str,
    pub purpose: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpCapabilitySnapshot {
    pub transport: McpTransport,
    pub supported_transports: Vec<McpTransport>,
    pub contracts: Vec<McpToolContract>,
    pub runnable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachePolicy {
    pub max_entries: usize,
    pub content_hashing: bool,
}

pub trait McpService {
    fn capability_snapshot(&self) -> McpCapabilitySnapshot;
    fn cache_policy(&self) -> CachePolicy;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlaceholderMcpService;

impl McpService for PlaceholderMcpService {
    fn capability_snapshot(&self) -> McpCapabilitySnapshot {
        McpCapabilitySnapshot {
            transport: McpTransport::Stdio,
            supported_transports: vec![McpTransport::Stdio, McpTransport::LocalSocket],
            contracts: vec![
                McpToolContract {
                    tool_name: "cache-put",
                    purpose: "Store file content snapshots for later task execution reuse",
                },
                McpToolContract {
                    tool_name: "cache-get",
                    purpose: "Resolve cached file content by deterministic cache keys",
                },
            ],
            runnable: false,
        }
    }

    fn cache_policy(&self) -> CachePolicy {
        CachePolicy {
            max_entries: 1024,
            content_hashing: true,
        }
    }
}

pub fn run_placeholder_mcp(request: McpRequest) -> Result<String> {
    let service = PlaceholderMcpService;
    let snapshot = service.capability_snapshot();
    let policy = service.cache_policy();

    match request.format {
        McpFormat::Text => Ok(format!(
            "TODO: '{NAME}' is planned and not implemented yet. MCP file-cache surface defines {} placeholder tool contract(s) with max {} entries. Next step: run 'sce mcp --help' for current placeholder usage while runtime execution remains disabled.",
            snapshot.contracts.len(),
            policy.max_entries
        )),
        McpFormat::Json => {
            let payload = json!({
                "status": "ok",
                "command": NAME,
                "placeholder_state": "planned",
                "runnable": snapshot.runnable,
                "transport": transport_name(snapshot.transport),
                "supported_transports": snapshot
                    .supported_transports
                    .iter()
                    .map(|transport| transport_name(*transport))
                    .collect::<Vec<_>>(),
                "capabilities": snapshot
                    .contracts
                    .iter()
                    .map(|contract| json!({
                        "tool_name": contract.tool_name,
                        "purpose": contract.purpose,
                    }))
                    .collect::<Vec<_>>(),
                "cache_policy": {
                    "max_entries": policy.max_entries,
                    "content_hashing": policy.content_hashing,
                },
                "next_step": "Run 'sce mcp --help' for current placeholder usage while runtime execution remains disabled.",
            });

            serde_json::to_string_pretty(&payload)
                .context("failed to serialize mcp placeholder report to JSON")
        }
    }
}

fn transport_name(transport: McpTransport) -> &'static str {
    match transport {
        McpTransport::Stdio => "stdio",
        McpTransport::LocalSocket => "local_socket",
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde_json::Value;

    use super::{
        run_placeholder_mcp, McpFormat, McpRequest, McpService, PlaceholderMcpService, NAME,
    };

    #[test]
    fn mcp_placeholder_snapshot_is_non_runnable() {
        let service = PlaceholderMcpService;
        let snapshot = service.capability_snapshot();
        assert!(!snapshot.runnable);
        assert!(!snapshot.contracts.is_empty());
        assert_eq!(snapshot.supported_transports.len(), 2);
        assert_eq!(snapshot.contracts[0].tool_name, "cache-put");
        let policy = service.cache_policy();
        assert_eq!(policy.max_entries, 1024);
        assert!(policy.content_hashing);
    }

    #[test]
    fn mcp_placeholder_message_mentions_contracts() -> Result<()> {
        let message = run_placeholder_mcp(McpRequest {
            format: McpFormat::Text,
        })?;
        assert!(message.contains("file-cache surface"));
        Ok(())
    }

    #[test]
    fn mcp_json_output_includes_stable_fields() -> Result<()> {
        let output = run_placeholder_mcp(McpRequest {
            format: McpFormat::Json,
        })?;
        let parsed: Value = serde_json::from_str(&output)?;
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["command"], NAME);
        assert_eq!(parsed["placeholder_state"], "planned");
        assert!(parsed["runnable"].is_boolean());
        assert!(parsed["supported_transports"].is_array());
        assert!(parsed["capabilities"].is_array());
        assert!(parsed["cache_policy"].is_object());
        assert!(parsed["next_step"].as_str().is_some());
        Ok(())
    }

    #[test]
    fn mcp_json_output_is_deterministic_for_same_request() -> Result<()> {
        let first = run_placeholder_mcp(McpRequest {
            format: McpFormat::Json,
        })?;
        let second = run_placeholder_mcp(McpRequest {
            format: McpFormat::Json,
        })?;

        assert_eq!(first, second);
        Ok(())
    }
}
