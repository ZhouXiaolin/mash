// crates/mash-ai/src/openai.rs
use std::future::Future;
use std::pin::Pin;

use anyhow::{bail, Result};
use bytes::Bytes;
use futures_core::Stream;
use reqwest::Client;
use serde_json::{json, Value};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::client::LlmClient;
use crate::types::*;

/// Configuration for the OpenAI-compatible backend.
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub api_key: String,
}

pub struct OpenAiBackend {
    client: Client,
    config: OpenAiConfig,
}

impl OpenAiBackend {
    pub fn new(config: OpenAiConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    fn url(&self) -> String {
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }
}

// ── Request/Response conversion ─────────────────────────────────

/// Convert our unified messages to OpenAI format.
fn to_openai_messages(system: &str, messages: &[Message]) -> Vec<Value> {
    let mut out = vec![json!({"role": "system", "content": system})];

    for msg in messages {
        match &msg.content {
            MessageContent::Text(t) => {
                let role = match msg.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                out.push(json!({"role": role, "content": t}));
            }
            MessageContent::Blocks(blocks) => {
                // Collect text, thinking, and tool_use/tool_result blocks
                let mut text_parts = Vec::new();
                let mut reasoning_content = String::new();
                let mut tool_calls_out = Vec::new();

                for block in blocks {
                    match block {
                        ContentBlock::Text { text } => text_parts.push(text.clone()),
                        ContentBlock::Thinking { thinking } => {
                            // Preserve thinking as reasoning_content for DeepSeek Reasoner
                            reasoning_content.push_str(thinking);
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_calls_out.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(input).unwrap_or_default(),
                                }
                            }));
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error: _,
                        } => {
                            out.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content,
                            }));
                            continue;
                        }
                    }
                }

                if !tool_calls_out.is_empty() {
                    let mut msg_obj = json!({
                        "role": "assistant",
                    });
                    if !text_parts.is_empty() {
                        msg_obj["content"] = json!(text_parts.join("\n"));
                    }
                    // Include reasoning_content for DeepSeek Reasoner compatibility
                    if !reasoning_content.is_empty() {
                        msg_obj["reasoning_content"] = json!(reasoning_content);
                    }
                    msg_obj["tool_calls"] = json!(tool_calls_out);
                    out.push(msg_obj);
                } else if !text_parts.is_empty() || !reasoning_content.is_empty() {
                    let role = match msg.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                    };
                    let mut msg_obj = json!({"role": role});
                    if !text_parts.is_empty() {
                        msg_obj["content"] = json!(text_parts.join("\n"));
                    }
                    if !reasoning_content.is_empty() {
                        msg_obj["reasoning_content"] = json!(reasoning_content);
                    }
                    out.push(msg_obj);
                }
            }
        }
    }
    out
}

/// Convert OpenAI tools format from our unified tool definitions.
fn to_openai_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t["name"],
                    "description": t.get("description").cloned().unwrap_or(json!("")),
                    "parameters": t.get("input_schema").cloned().unwrap_or(json!({})),
                }
            })
        })
        .collect()
}

/// Parse an OpenAI response into our unified format.
fn parse_openai_response(body: &str) -> Result<LlmResponse> {
    let v: Value = serde_json::from_str(body)?;
    let choice = v
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| anyhow::anyhow!("No choices in OpenAI response"))?;

    let message = &choice["message"];
    let mut content = Vec::new();

    // Handle reasoning_content from DeepSeek Reasoner
    if let Some(reasoning) = message.get("reasoning_content").and_then(|c| c.as_str()) {
        if !reasoning.is_empty() {
            content.push(ContentBlock::Thinking {
                thinking: reasoning.to_string(),
            });
        }
    }

    if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            content.push(ContentBlock::Text {
                text: text.to_string(),
            });
        }
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(|tc| tc.as_array()) {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
            content.push(ContentBlock::ToolUse { id, name, input });
        }
    }

    let stop_reason = choice
        .get("finish_reason")
        .and_then(|r| r.as_str())
        .map(|r| match r {
            "stop" => "end_turn".to_string(),
            "tool_calls" => "tool_use".to_string(),
            other => other.to_string(),
        });

    Ok(LlmResponse {
        content,
        stop_reason,
    })
}

impl LlmClient for OpenAiBackend {
    fn complete(
        &self,
        request: &LlmRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse>> + Send + '_>> {
        let messages = to_openai_messages(&request.system, &request.messages);
        let tools = to_openai_tools(&request.tools);
        let model = request.model.clone();
        let max_tokens = request.max_tokens;

        Box::pin(async move {
            let mut body = json!({
                "model": model,
                "messages": messages,
                "max_tokens": max_tokens,
            });
            if !tools.is_empty() {
                body["tools"] = json!(tools);
            }

            let resp = self
                .client
                .post(&self.url())
                .header("Authorization", format!("Bearer {}", self.config.api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status();
            let text = resp.text().await?;

            if !status.is_success() {
                bail!("OpenAI API error ({}): {}", status, text);
            }

            parse_openai_response(&text)
        })
    }

    fn stream(
        &self,
        request: &LlmRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>>> + Send + '_>> {
        let messages = to_openai_messages(&request.system, &request.messages);
        let tools = to_openai_tools(&request.tools);
        let model = request.model.clone();
        let max_tokens = request.max_tokens;

        Box::pin(async move {
            let mut body = json!({
                "model": model,
                "messages": messages,
                "max_tokens": max_tokens,
                "stream": true,
            });
            if !tools.is_empty() {
                body["tools"] = json!(tools);
            }

            let resp = self
                .client
                .post(&self.url())
                .header("Authorization", format!("Bearer {}", self.config.api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await?;
                bail!("OpenAI API error ({}): {}", status, text);
            }

            let event_stream = parse_openai_sse(resp.bytes_stream());
            Ok(Box::pin(event_stream) as Pin<Box<dyn Stream<Item = StreamEvent> + Send>>)
        })
    }
}

/// Parse OpenAI SSE stream into StreamEvents.
fn parse_openai_sse(
    byte_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = StreamEvent> + Send {
    let (tx, rx) = tokio::sync::mpsc::channel(64);

    tokio::spawn(async move {
        let mut buf = String::new();
        tokio::pin!(byte_stream);
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_args = String::new();
        let mut reasoning_content = String::new();

        while let Some(result) = byte_stream.next().await {
            match result {
                Ok(bytes) => {
                    buf.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(pos) = buf.find('\n') {
                        let line = buf[..pos].trim().to_string();
                        buf = buf[pos + 1..].to_string();

                        if line.is_empty() || line.starts_with(':') {
                            continue;
                        }

                        let data = match line.strip_prefix("data: ") {
                            Some(d) => d,
                            None => continue,
                        };

                        if data == "[DONE]" {
                            // Flush remaining tool call
                            if !current_tool_id.is_empty() {
                                let input: Value =
                                    serde_json::from_str(&current_tool_args).unwrap_or(json!({}));
                                let _ = tx
                                    .send(StreamEvent::BlockComplete(ContentBlock::ToolUse {
                                        id: current_tool_id.clone(),
                                        name: current_tool_name.clone(),
                                        input,
                                    }))
                                    .await;
                                current_tool_id.clear();
                            }
                            // Flush reasoning content as Thinking block
                            if !reasoning_content.is_empty() {
                                let _ = tx.send(StreamEvent::BlockComplete(ContentBlock::Thinking {
                                    thinking: reasoning_content.clone(),
                                })).await;
                                reasoning_content.clear();
                            }
                            let _ = tx.send(StreamEvent::Done { stop_reason: None }).await;
                            return;
                        }

                        let Ok(v) = serde_json::from_str::<Value>(data) else {
                            continue;
                        };

                        // First check for tool_calls in delta
                        if let Some(tool_calls) = v
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                            .and_then(|d| d.get("tool_calls"))
                            .and_then(|tc| tc.as_array())
                        {
                            // Flush reasoning content before tool calls
                            if !reasoning_content.is_empty() {
                                let _ = tx.send(StreamEvent::BlockComplete(ContentBlock::Thinking {
                                    thinking: reasoning_content.clone(),
                                })).await;
                                reasoning_content.clear();
                            }
                            for tc in tool_calls {
                                // Check for new tool call with id
                                if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                    // Flush previous tool call
                                    if !current_tool_id.is_empty() {
                                        let input: Value =
                                            serde_json::from_str(&current_tool_args).unwrap_or(json!({}));
                                        let _ = tx
                                            .send(StreamEvent::BlockComplete(ContentBlock::ToolUse {
                                                id: current_tool_id.clone(),
                                                name: current_tool_name.clone(),
                                                input,
                                            }))
                                            .await;
                                    }
                                    current_tool_id = id.to_string();
                                    current_tool_name = tc
                                        .get("function")
                                        .and_then(|f| f.get("name"))
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    current_tool_args = tc
                                        .get("function")
                                        .and_then(|f| f.get("arguments"))
                                        .and_then(|a| a.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                } else if let Some(args) = tc
                                    .get("function")
                                    .and_then(|f| f.get("arguments"))
                                    .and_then(|a| a.as_str())
                                {
                                    current_tool_args.push_str(args);
                                }
                            }
                        }

                        // Then check for finish_reason
                        if let Some(reason) = v
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("finish_reason"))
                            .and_then(|r| r.as_str())
                        {
                            // Flush remaining tool call
                            if !current_tool_id.is_empty() {
                                let input: Value =
                                    serde_json::from_str(&current_tool_args).unwrap_or(json!({}));
                                let _ = tx
                                    .send(StreamEvent::BlockComplete(ContentBlock::ToolUse {
                                        id: current_tool_id.clone(),
                                        name: current_tool_name.clone(),
                                        input,
                                    }))
                                    .await;
                                current_tool_id.clear();
                            }
                            // Flush reasoning content
                            if !reasoning_content.is_empty() {
                                let _ = tx.send(StreamEvent::BlockComplete(ContentBlock::Thinking {
                                    thinking: reasoning_content.clone(),
                                })).await;
                                reasoning_content.clear();
                            }
                            let _ = tx
                                .send(StreamEvent::Done {
                                    stop_reason: Some(reason.to_string()),
                                })
                                .await;
                            return;
                        }

                        // Then check for content delta
                        if let Some(delta) = v
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                        {
                            // Reasoning content delta (DeepSeek Reasoner)
                            if let Some(r) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
                                if !r.is_empty() {
                                    reasoning_content.push_str(r);
                                }
                            }

                            // Text content delta
                            if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
                                if !text.is_empty() {
                                    let _ = tx.send(StreamEvent::TextDelta(text.to_string())).await;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                    return;
                }
            }
        }

        // Flush remaining tool call
        if !current_tool_id.is_empty() {
            let input: Value = serde_json::from_str(&current_tool_args).unwrap_or(json!({}));
            let _ = tx
                .send(StreamEvent::BlockComplete(ContentBlock::ToolUse {
                    id: current_tool_id,
                    name: current_tool_name,
                    input,
                }))
                .await;
        }
        // Flush remaining reasoning content
        if !reasoning_content.is_empty() {
            let _ = tx.send(StreamEvent::BlockComplete(ContentBlock::Thinking {
                thinking: reasoning_content,
            })).await;
        }
    });

    ReceiverStream::new(rx)
}
