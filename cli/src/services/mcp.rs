use anyhow::Result;

pub const NAME: &str = "mcp";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
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

pub fn run_placeholder_mcp() -> Result<String> {
    let service = PlaceholderMcpService;
    let snapshot = service.capability_snapshot();
    let policy = service.cache_policy();

    Ok(format!(
        "TODO: '{NAME}' is planned and not implemented yet. MCP file-cache surface defines {} placeholder tool contract(s) with max {} entries.",
        snapshot.contracts.len(),
        policy.max_entries
    ))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{run_placeholder_mcp, McpService, PlaceholderMcpService};

    #[test]
    fn mcp_placeholder_snapshot_is_non_runnable() {
        let service = PlaceholderMcpService;
        let snapshot = service.capability_snapshot();
        assert!(!snapshot.runnable);
        assert!(!snapshot.contracts.is_empty());
        assert_eq!(snapshot.contracts[0].tool_name, "cache-put");
        let policy = service.cache_policy();
        assert_eq!(policy.max_entries, 1024);
        assert!(policy.content_hashing);
    }

    #[test]
    fn mcp_placeholder_message_mentions_contracts() -> Result<()> {
        let message = run_placeholder_mcp()?;
        assert!(message.contains("file-cache surface"));
        Ok(())
    }
}
