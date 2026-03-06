use std::path::PathBuf;use std::sync::Arc;

use anyhow::Result;
use mash_agent::{run, AgentContext, AgentEvent as CoreAgentEvent};
use mash_ai::anthropic::{AnthropicBackend, AnthropicConfig};
use mash_ai::Message;
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::StreamExt;

use crate::config::ApiConfig;
use crate::tool_adapter;

/// System prompt 由多模块在编译期拼接而成，见 `src/prompt/*.md`。
pub const SYSTEM_PROMPT: &str = concat!(
    include_str!("prompt/identity.md"),
    "\n\n",
    include_str!("prompt/tools_general.md"),
    "\n\n",
    include_str!("prompt/tools_specialized.md"),
    "\n\n",
    include_str!("prompt/work_style.md"),
    "\n\n",
    include_str!("prompt/response_format.md"),
);

/// Events emitted by the agent loop in real time.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    Text(String),
    ToolCall { name: String, description: String },
    ToolResult { preview: String },
    TasksUpdated { done: usize, total: usize },
}

/// Adapt core AgentEvent to application AgentEvent.
fn adapt_event(core_event: &CoreAgentEvent, tx: &mpsc::UnboundedSender<AgentEvent>) -> bool {
    match core_event {
        CoreAgentEvent::AgentStart => false,
        CoreAgentEvent::TurnStart => false,
        CoreAgentEvent::TextDelta(text) => {
            let _ = tx.send(AgentEvent::Text(text.clone()));
            true
        }
        CoreAgentEvent::Text(text) => {
            for line in text.lines() {
                let _ = tx.send(AgentEvent::Text(line.to_string()));
            }
            true
        }
        CoreAgentEvent::ToolCallStart { id: _, name, input } => {
            let desc = tool_call_description(name, input);
            let _ = tx.send(AgentEvent::ToolCall {
                name: name.clone(),
                description: desc,
            });
            true
        }
        CoreAgentEvent::ToolCallEnd {
            id: _,
            name: _,
            result,
            is_error: _,
        } => {
            let preview = result.lines().next().unwrap_or("(empty)").to_string();
            let _ = tx.send(AgentEvent::ToolResult { preview });
            true
        }
        CoreAgentEvent::TurnEnd => false,
        CoreAgentEvent::AgentEnd => false,
        CoreAgentEvent::ThinkingDelta(_) => false,
        CoreAgentEvent::Error(e) => {
            let _ = tx.send(AgentEvent::Text(format!("[Error] {}", e)));
            true
        }
    }
}

/// Build a short description for the tool call event.
fn tool_call_description(name: &str, input: &Value) -> String {
    match name {
        "bash" => input["command"].as_str().unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// Configuration for the agent loop.
#[derive(Clone)]
pub struct AgentConfig {
    pub system: String,
    pub model: String,
    pub max_tokens: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        let config = ApiConfig::load();
        Self {
            system: SYSTEM_PROMPT.to_string(),
            model: config.model,
            max_tokens: config.max_tokens,
        }
    }
}

pub async fn run_agent_loop(
    config: AgentConfig,
    messages: &Arc<Mutex<Vec<Message>>>,
    tx: mpsc::UnboundedSender<AgentEvent>,
    cwd: PathBuf,
) -> Result<()> {
    // Create backend from ApiConfig
    let api_config = ApiConfig::load();
    let backend = AnthropicBackend::new(AnthropicConfig {
        base_url: api_config.base_url,
        api_key: api_config.api_key,
    });

    // Create abort channel
    let (_abort_tx, abort_rx) = tokio::sync::watch::channel(false);

    // Build context, sharing the messages Arc directly so the agent loop
    // writes assistant replies and tool results back in-place.
    let tools = tool_adapter::create_tools(cwd);

    let context = AgentContext {
        system: config.system,
        model: config.model,
        max_tokens: config.max_tokens,
        messages: Arc::clone(messages),
        tools,
    };

    // Run the agent loop
    let event_stream = run(&backend, context, abort_rx);

    // Process events
    tokio::pin!(event_stream);

    while let Some(event) = event_stream.next().await {
        adapt_event(&event, &tx);

        if matches!(event, CoreAgentEvent::AgentEnd | CoreAgentEvent::Error(_)) {
            break;
        }
    }

    Ok(())
}
