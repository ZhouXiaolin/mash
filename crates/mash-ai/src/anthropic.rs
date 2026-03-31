// crates/mash-ai/src/anthropic.rs
use std::future::Future;
use std::pin::Pin;

use anyhow::{Result, bail};
use bytes::Bytes;
use futures_core::Stream;
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::client::LlmClient;
use crate::types::*;

const API_VERSION: &str = "2023-06-01";

/// Configuration for the Anthropic backend.
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub base_url: String,
    pub api_key: String,
}

pub struct AnthropicBackend {
    client: Client,
    config: AnthropicConfig,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    system: String,
    max_tokens: u32,
    messages: Vec<Message>,
    tools: Vec<Value>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

impl AnthropicBackend {
    pub fn new(config: AnthropicConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    fn url(&self) -> String {
        format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'))
    }

    fn build_request(&self, request: &LlmRequest, stream: bool) -> AnthropicRequest {
        AnthropicRequest {
            model: request.model.clone(),
            system: request.system.clone(),
            max_tokens: request.max_tokens,
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            stream,
        }
    }
}

impl LlmClient for AnthropicBackend {
    fn complete(
        &self,
        request: &LlmRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse>> + Send + '_>> {
        let req = self.build_request(request, false);
        Box::pin(async move {
            let resp = self
                .client
                .post(&self.url())
                .header("x-api-key", &self.config.api_key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .json(&req)
                .send()
                .await?;

            let status = resp.status();
            let body = resp.text().await?;

            if !status.is_success() {
                bail!("Anthropic API error ({}): {}", status, body);
            }

            let response: LlmResponse = serde_json::from_str(&body)?;
            Ok(response)
        })
    }

    fn stream(
        &self,
        request: &LlmRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Pin<Box<dyn Stream<Item = StreamEvent> + Send>>>>
                + Send
                + '_,
        >,
    > {
        let req = self.build_request(request, true);
        Box::pin(async move {
            let resp = self
                .client
                .post(&self.url())
                .header("x-api-key", &self.config.api_key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .json(&req)
                .send()
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await?;
                bail!("Anthropic API error ({}): {}", status, body);
            }

            let event_stream = parse_anthropic_sse(resp.bytes_stream());
            Ok(Box::pin(event_stream) as Pin<Box<dyn Stream<Item = StreamEvent> + Send>>)
        })
    }
}

/// Parse Anthropic SSE stream into StreamEvents.
fn parse_anthropic_sse(
    byte_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = StreamEvent> + Send {
    // Use a bounded channel to bridge between the async task and the stream
    let (tx, rx) = tokio::sync::mpsc::channel(64);

    tokio::spawn(async move {
        let mut lines_buf = String::new();
        let mut pending_tool: Option<(String, String, String)> = None; // (id, name, accumulated_json)
        let mut thinking_content = String::new();
        tokio::pin!(byte_stream);

        while let Some(result) = byte_stream.next().await {
            match result {
                Ok(bytes) => {
                    lines_buf.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(pos) = lines_buf.find("\n\n") {
                        let event_block = lines_buf[..pos].to_string();
                        lines_buf = lines_buf[pos + 2..].to_string();

                        // Parse and handle multiple events from one block
                        let events = parse_sse_block_with_state(
                            &event_block,
                            &mut pending_tool,
                            &mut thinking_content,
                        );
                        for event in events {
                            let is_done = matches!(event, StreamEvent::Done { .. });
                            if tx.send(event).await.is_err() {
                                return;
                            }
                            if is_done {
                                return;
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
    });

    ReceiverStream::new(rx)
}

/// Parse with state for accumulating tool input and thinking deltas.
/// Returns a Vec because one SSE block can produce multiple events (e.g., flush thinking then start tool).
fn parse_sse_block_with_state(
    block: &str,
    pending_tool: &mut Option<(String, String, String)>,
    thinking_content: &mut String,
) -> Vec<StreamEvent> {
    let mut event_type = None;
    let mut data = String::new();

    for line in block.lines() {
        if let Some(t) = line.strip_prefix("event: ") {
            event_type = Some(t.to_string());
        } else if let Some(d) = line.strip_prefix("data: ") {
            data.push_str(d);
        }
    }

    let event_type = match event_type {
        Some(t) => t,
        None => return vec![],
    };

    let mut result = Vec::new();

    match event_type.as_str() {
        "content_block_start" => {
            // Flush thinking before new content block
            if !thinking_content.is_empty() {
                result.push(StreamEvent::BlockComplete(ContentBlock::Thinking {
                    thinking: thinking_content.clone(),
                }));
                thinking_content.clear();
            }
            if let Ok(v) = serde_json::from_str::<Value>(&data) {
                if let Some(content_block) = v.get("content_block") {
                    if let Some(block_type) = content_block.get("type").and_then(|t| t.as_str()) {
                        if block_type == "tool_use" {
                            if let (Some(id), Some(name)) = (
                                content_block.get("id").and_then(|i| i.as_str()),
                                content_block.get("name").and_then(|n| n.as_str()),
                            ) {
                                *pending_tool =
                                    Some((id.to_string(), name.to_string(), String::new()));
                            }
                        }
                    }
                }
            }
        }
        "content_block_delta" => {
            if let Ok(v) = serde_json::from_str::<Value>(&data) {
                if let Some(delta) = v.get("delta") {
                    if let Some(delta_type) = delta.get("type").and_then(|t| t.as_str()) {
                        match delta_type {
                            "text_delta" => {
                                // Flush thinking before text
                                if !thinking_content.is_empty() {
                                    result.push(StreamEvent::BlockComplete(
                                        ContentBlock::Thinking {
                                            thinking: thinking_content.clone(),
                                        },
                                    ));
                                    thinking_content.clear();
                                }
                                if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                    result.push(StreamEvent::TextDelta(text.to_string()));
                                }
                            }
                            "input_json_delta" => {
                                // Accumulate JSON string fragments
                                if let Some((_id, _name, json_str)) = pending_tool.as_mut() {
                                    if let Some(partial) =
                                        delta.get("partial_json").and_then(|p| p.as_str())
                                    {
                                        json_str.push_str(partial);
                                    }
                                }
                            }
                            "thinking_delta" => {
                                if let Some(thinking) =
                                    delta.get("thinking").and_then(|t| t.as_str())
                                {
                                    thinking_content.push_str(thinking);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        "content_block_stop" => {
            // Emit the complete tool call
            if let Some((id, name, json_str)) = pending_tool.take() {
                let input = if json_str.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str::<serde_json::Value>(&json_str)
                        .unwrap_or_else(|_| serde_json::json!({}))
                };
                result.push(StreamEvent::BlockComplete(ContentBlock::ToolUse {
                    id,
                    name,
                    input,
                }));
            }
        }
        "message_stop" => {
            // Flush thinking before message stop
            if !thinking_content.is_empty() {
                result.push(StreamEvent::BlockComplete(ContentBlock::Thinking {
                    thinking: thinking_content.clone(),
                }));
                thinking_content.clear();
            }
            result.push(StreamEvent::Done { stop_reason: None });
        }
        "message_delta" => {
            if let Ok(v) = serde_json::from_str::<Value>(&data) {
                let stop = v
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|s| s.as_str())
                    .map(String::from);
                result.push(StreamEvent::Done { stop_reason: stop });
            }
        }
        "error" => {
            result.push(StreamEvent::Error(data));
        }
        _ => {}
    }

    result
}
