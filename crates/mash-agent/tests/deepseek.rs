// crates/mash-agent/tests/deepseek.rs
use std::sync::Arc;

use mash_agent::{AgentContext, AgentTool, ToolResult, run, run_streaming};
use mash_ai::anthropic::{AnthropicBackend, AnthropicConfig};
use mash_ai::openai::{OpenAiBackend, OpenAiConfig};
use mash_ai::{Message, MessageContent, Role};
use serde_json::json;
use std::pin::Pin;
use tokio::sync::Mutex;
use tokio_stream::StreamExt;

fn api_key() -> String {
    std::env::var("DEEPSEEK_API_KEY").unwrap_or_else(|_| "sk-test".to_string())
}

fn openai_backend() -> OpenAiBackend {
    OpenAiBackend::new(OpenAiConfig {
        base_url: "https://api.deepseek.com".to_string(),
        api_key: api_key(),
    })
}

fn anthropic_backend() -> AnthropicBackend {
    AnthropicBackend::new(AnthropicConfig {
        base_url: "https://api.deepseek.com/anthropic".to_string(),
        api_key: api_key(),
    })
}

fn simple_context(model: &str) -> AgentContext {
    AgentContext {
        system: "You are a helpful assistant. Keep responses brief.".to_string(),
        model: model.to_string(),
        max_tokens: 256,
        messages: Arc::new(Mutex::new(vec![Message {
            role: Role::User,
            content: MessageContent::Text("Say 'Hello, DeepSeek!' in one sentence.".to_string()),
        }])),
        tools: vec![],
    }
}

/// A simple echo tool for testing
struct EchoTool;

impl AgentTool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn definition(&self) -> serde_json::Value {
        json!({
            "name": "echo",
            "description": "Echo back the input text",
            "input_schema": {
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The text to echo back"
                    }
                },
                "required": ["text"]
            }
        })
    }

    fn execute(
        &self,
        input: &serde_json::Value,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolResult> + Send + '_>> {
        let result = match input.get("text").and_then(|t| t.as_str()) {
            Some(text) => ToolResult {
                content: format!("Echo: {}", text),
                is_error: false,
            },
            None => ToolResult {
                content: "Error: missing 'text' field".to_string(),
                is_error: true,
            },
        };
        Box::pin(async move { result })
    }
}

#[tokio::test]
#[ignore]
async fn openai_basic_conversation() {
    let backend = openai_backend();
    let context = simple_context("deepseek-chat");
    let (_tx, rx) = tokio::sync::watch::channel(false);

    let mut events = run(&backend, context, rx);
    let mut got_text = false;
    let mut got_end = false;

    while let Some(event) = events.next().await {
        match event {
            mash_agent::AgentEvent::Text(_) => got_text = true,
            mash_agent::AgentEvent::AgentEnd => {
                got_end = true;
                break;
            }
            mash_agent::AgentEvent::Error(e) => panic!("Agent error: {}", e),
            _ => {}
        }
    }

    assert!(got_text, "Should have received text response");
    assert!(got_end, "Should have received AgentEnd");
}

#[tokio::test]
#[ignore]
async fn openai_tool_calling() {
    let backend = openai_backend();
    let context = AgentContext {
        system: "You are a helpful assistant. Use the echo tool when asked to echo.".to_string(),
        model: "deepseek-chat".to_string(),
        max_tokens: 256,
        messages: Arc::new(Mutex::new(vec![Message {
            role: Role::User,
            content: MessageContent::Text("Please echo the word 'test'".to_string()),
        }])),
        tools: vec![Box::new(EchoTool)],
    };
    let (_tx, rx) = tokio::sync::watch::channel(false);

    let mut events = run(&backend, context, rx);
    let mut got_tool_call = false;
    let mut got_response = false;

    while let Some(event) = events.next().await {
        match event {
            mash_agent::AgentEvent::ToolCallStart { .. } => got_tool_call = true,
            mash_agent::AgentEvent::Text(_) => got_response = true,
            mash_agent::AgentEvent::AgentEnd => break,
            mash_agent::AgentEvent::Error(e) => panic!("Agent error: {}", e),
            _ => {}
        }
    }

    assert!(got_tool_call, "Should have called echo tool");
    assert!(got_response, "Should have final response");
}

#[tokio::test]
#[ignore]
async fn openai_streaming() {
    let backend = openai_backend();
    let context = simple_context("deepseek-chat");
    let (_tx, rx) = tokio::sync::watch::channel(false);

    let mut events = run_streaming(&backend, context, rx);
    let mut got_delta = false;
    let mut got_end = false;

    while let Some(event) = events.next().await {
        match event {
            mash_agent::AgentEvent::TextDelta(_) => got_delta = true,
            mash_agent::AgentEvent::AgentEnd => {
                got_end = true;
                break;
            }
            mash_agent::AgentEvent::Error(e) => panic!("Agent error: {}", e),
            _ => {}
        }
    }

    assert!(got_delta, "Should have received text deltas");
    assert!(got_end, "Should have received AgentEnd");
}

#[tokio::test]
#[ignore]
async fn anthropic_basic_conversation() {
    let backend = anthropic_backend();
    let context = simple_context("deepseek-chat");
    let (_tx, rx) = tokio::sync::watch::channel(false);

    let mut events = run(&backend, context, rx);
    let mut got_text = false;
    let mut got_end = false;

    while let Some(event) = events.next().await {
        match event {
            mash_agent::AgentEvent::Text(_) => got_text = true,
            mash_agent::AgentEvent::AgentEnd => {
                got_end = true;
                break;
            }
            mash_agent::AgentEvent::Error(e) => panic!("Agent error: {}", e),
            _ => {}
        }
    }

    assert!(got_text, "Should have received text response");
    assert!(got_end, "Should have received AgentEnd");
}

#[tokio::test]
#[ignore]
async fn openai_reasoner_basic() {
    let backend = openai_backend();
    let context = simple_context("deepseek-reasoner");
    let (_tx, rx) = tokio::sync::watch::channel(false);

    let mut events = run(&backend, context, rx);
    let mut got_content = false;
    let mut got_end = false;

    while let Some(event) = events.next().await {
        match event {
            mash_agent::AgentEvent::Text(_) | mash_agent::AgentEvent::ThinkingDelta(_) => {
                got_content = true;
            }
            mash_agent::AgentEvent::AgentEnd => {
                got_end = true;
                break;
            }
            mash_agent::AgentEvent::Error(e) => panic!("Agent error: {}", e),
            _ => {}
        }
    }

    assert!(
        got_content,
        "Should have received text or thinking response"
    );
    assert!(got_end, "Should have received AgentEnd");
}
