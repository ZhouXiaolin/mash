use serde::{Deserialize, Serialize};

/// Skill metadata sent from daemon to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
}

/// Summary of an active session visible to other clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub busy: bool,
}

/// Messages from client to daemon (JSON Lines over Unix socket).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// Send a user message, starting the agent loop if idle.
    SendMessage {
        text: String,
        /// Optional extra instructions appended to the system prompt for this session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        system_extra: Option<String>,
    },
    /// Clear conversation history (start fresh in current session).
    ClearHistory,
    /// List all active sessions on the daemon.
    ListSessions,
    /// Send a text message to another session.
    SendToSession { target: String, text: String },
    /// Call an MCP tool via the daemon.
    McpCall {
        server: String,
        tool: String,
        #[serde(default)]
        arguments: serde_json::Value,
    },
}

/// Messages from daemon to client (JSON Lines over Unix socket).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonMessage {
    /// Sent immediately after client connects.
    Welcome {
        #[serde(default)]
        session_id: String,
        skills: Vec<SkillEntry>,
        busy: bool,
    },
    /// Agent loop has started processing.
    AgentStarted,
    /// Streaming text line from assistant.
    Text { line: String },
    /// A tool is being called.
    ToolCall { name: String, description: String },
    /// Tool execution result (first line preview).
    ToolResult { preview: String },
    /// Task progress updated.
    TasksUpdated { done: usize, total: usize },
    /// Full task file content for display.
    TaskContent { content: String },
    /// Agent loop completed successfully.
    AgentCompleted,
    /// Agent loop encountered an error.
    AgentError { message: String },
    /// Response to ListSessions.
    SessionList { sessions: Vec<SessionInfo> },
    /// A message received from another session.
    PeerMessage { from: String, text: String },
    /// Response to McpCall.
    McpResult { result: String },
}
