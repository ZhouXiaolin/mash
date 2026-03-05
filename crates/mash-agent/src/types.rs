// crates/mash-agent/src/types.rs
use mash_ai::Message;
use serde_json::Value;

/// Events emitted by the agent loop.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Agent loop started.
    AgentStart,
    /// A new turn (LLM call) is starting.
    TurnStart,
    /// Streaming text delta from assistant.
    TextDelta(String),
    /// Streaming thinking delta (reasoning output).
    ThinkingDelta(String),
    /// Assistant produced a full text block.
    Text(String),
    /// Agent is calling a tool.
    ToolCallStart {
        id: String,
        name: String,
        input: Value,
    },
    /// Tool execution finished.
    ToolCallEnd {
        id: String,
        name: String,
        result: String,
        is_error: bool,
    },
    /// A turn has completed (one LLM response fully processed).
    TurnEnd,
    /// Agent loop finished.
    AgentEnd,
    /// Agent loop encountered an error.
    Error(String),
}

/// A tool that the agent can call.
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn definition(&self) -> Value;

    fn execute(
        &self,
        input: &Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolResult> + Send + '_>>;
}

/// Result of a tool execution.
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

/// Context for the agent loop.
pub struct AgentContext {
    pub system: String,
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<Message>,
    pub tools: Vec<Box<dyn AgentTool>>,
}
