//! Facade — **agent access** (routes/agent.mjs, upstream c22767f).
//!
//! | JS route                          | method |
//! |-----------------------------------|--------|
//! | GET  /api/agent-access            | [`Engine::agent_access`] |
//! | PUT  /api/agent-access            | [`Engine::set_agent_access`] |
//! | POST /api/agent-access/rotate     | [`Engine::rotate_agent_token`] |
//!
//! The UI routes are engine methods like every other route. The token itself
//! is never returned — only its path.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::agent::{self, AgentAccessPatch};
use crate::engine::Engine;
use crate::error::EngineResult;

/// Argument that switches the app binary into the stdio MCP server.
pub const MCP_ARG: &str = "--mcp";

/// Ready-to-paste MCP server entry (JS `mcpEntry()`): the absolute app binary
/// in `--mcp` mode, plus the data dir + port it must talk to. Absolute paths
/// because MCP clients resolve relative ones against the agent's project dir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpEntry {
    pub command: String,
    pub args: Vec<String>,
    pub env: IndexMap<String, String>,
}

/// `{ enabled, allowed_servers, token_path, mcp }` (JS `accessStatus()`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentAccessStatus {
    pub enabled: bool,
    pub allowed_servers: Vec<String>,
    pub token_path: String,
    pub mcp: McpEntry,
}

impl Engine {
    /// The MCP server entry for this engine (current executable + `--mcp`).
    pub fn mcp_entry(&self) -> McpEntry {
        let command = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let mut env = IndexMap::new();
        env.insert("ESPECTRAL_DATA_DIR".to_string(), self.paths().data_dir().to_string_lossy().into_owned());
        env.insert("ESPECTRAL_PORT".to_string(), crate::loopback::configured_port().to_string());
        McpEntry { command, args: vec![MCP_ARG.to_string()], env }
    }

    fn agent_access_status(&self) -> AgentAccessStatus {
        let cfg = agent::agent_config(self.config());
        AgentAccessStatus {
            enabled: cfg.enabled,
            allowed_servers: cfg.allowed_servers,
            token_path: agent::token_path(self.paths().data_dir()).to_string_lossy().into_owned(),
            mcp: self.mcp_entry(),
        }
    }

    /// GET /api/agent-access.
    pub async fn agent_access(&self) -> EngineResult<AgentAccessStatus> {
        Ok(self.agent_access_status())
    }

    /// PUT /api/agent-access. Enabling creates the token (the UI copies the
    /// MCP config right after, so it must exist by then).
    pub async fn set_agent_access(&self, patch: AgentAccessPatch) -> EngineResult<AgentAccessStatus> {
        let cfg = agent::save_agent_config(self.config(), &patch);
        if cfg.enabled {
            agent::ensure_token(self.paths().data_dir())?;
        }
        Ok(self.agent_access_status())
    }

    /// POST /api/agent-access/rotate: replace the token (revokes every client).
    pub async fn rotate_agent_token(&self) -> EngineResult<AgentAccessStatus> {
        agent::write_token(self.paths().data_dir())?;
        Ok(self.agent_access_status())
    }
}
