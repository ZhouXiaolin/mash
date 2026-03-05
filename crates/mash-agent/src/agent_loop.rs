// crates/mash-agent/src/agent_loop.rs
use std::pin::Pin;

use futures_core::Stream;
use mash_ai::{ContentBlock, LlmClient, LlmRequest, Message, MessageContent, StreamEvent};
use tokio_stream::StreamExt;

use crate::types::*;

/// Run the agent loop, returning a stream of events.
///
/// The loop continues until the LLM responds without tool calls,
/// or an error occurs, or the abort signal fires.
pub fn run(
    client: &dyn LlmClient,
    context: AgentContext,
    abort: tokio::sync::watch::Receiver<bool>,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send + '_>> {
    let stream = async_stream::stream! {
        yield AgentEvent::AgentStart;

        let mut messages = context.messages;
        let tool_defs: Vec<serde_json::Value> = context
            .tools
            .iter()
            .map(|t| t.definition())
            .collect();

        loop {
            if *abort.borrow() {
                yield AgentEvent::Error("aborted".to_string());
                break;
            }

            yield AgentEvent::TurnStart;

            let request = LlmRequest {
                model: context.model.clone(),
                system: context.system.clone(),
                max_tokens: context.max_tokens,
                messages: messages.clone(),
                tools: tool_defs.clone(),
            };

            // Call LLM (non-streaming for MVP simplicity)
            let response = match client.complete(&request).await {
                Ok(r) => r,
                Err(e) => {
                    yield AgentEvent::Error(e.to_string());
                    break;
                }
            };

            // Process response blocks
            let mut full_text = String::new();
            let mut tool_calls = Vec::new();

            for block in &response.content {
                match block {
                    ContentBlock::Text { text } => {
                        if !text.is_empty() {
                            full_text.push_str(text);
                            yield AgentEvent::Text(text.clone());
                        }
                    }
                    ContentBlock::Thinking { thinking } => {
                        if !thinking.is_empty() {
                            yield AgentEvent::ThinkingDelta(thinking.clone());
                        }
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }

            // Push assistant message
            messages.push(Message {
                role: mash_ai::Role::Assistant,
                content: MessageContent::Blocks(response.content),
            });

            if tool_calls.is_empty() {
                yield AgentEvent::TurnEnd;
                break;
            }

            // Execute tool calls
            let mut results = Vec::new();
            for (id, name, input) in &tool_calls {
                yield AgentEvent::ToolCallStart {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                };

                // Find tool
                let tool = context.tools.iter().find(|t| t.name() == name);
                let result = match tool {
                    Some(t) => t.execute(input).await,
                    None => ToolResult {
                        content: format!("Unknown tool: {name}"),
                        is_error: true,
                    },
                };

                yield AgentEvent::ToolCallEnd {
                    id: id.clone(),
                    name: name.clone(),
                    result: result
                        .content
                        .lines()
                        .next()
                        .unwrap_or("(empty)")
                        .to_string(),
                    is_error: result.is_error,
                };

                results.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: result.content,
                    is_error: if result.is_error { Some(true) } else { None },
                });
            }

            // Push tool results as user message
            messages.push(Message {
                role: mash_ai::Role::User,
                content: MessageContent::Blocks(results),
            });

            yield AgentEvent::TurnEnd;
        }

        yield AgentEvent::AgentEnd;
    };

    Box::pin(stream)
}

/// Stream variant: uses LlmClient::stream for real-time text deltas.
pub fn run_streaming(
    client: &dyn LlmClient,
    context: AgentContext,
    abort: tokio::sync::watch::Receiver<bool>,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send + '_>> {
    let stream = async_stream::stream! {
        yield AgentEvent::AgentStart;

        let mut messages = context.messages;
        let tool_defs: Vec<serde_json::Value> = context
            .tools
            .iter()
            .map(|t| t.definition())
            .collect();

        loop {
            if *abort.borrow() {
                yield AgentEvent::Error("aborted".to_string());
                break;
            }

            yield AgentEvent::TurnStart;

            let request = LlmRequest {
                model: context.model.clone(),
                system: context.system.clone(),
                max_tokens: context.max_tokens,
                messages: messages.clone(),
                tools: tool_defs.clone(),
            };

            // Get stream from LLM
            let event_stream = match client.stream(&request).await {
                Ok(s) => s,
                Err(e) => {
                    yield AgentEvent::Error(e.to_string());
                    break;
                }
            };

            // Collect response from stream events
            let mut full_text = String::new();
            let mut blocks: Vec<ContentBlock> = Vec::new();
            let mut tool_calls = Vec::new();

            {
                let mut pinned = std::pin::pin!(event_stream);
                while let Some(event) = pinned.next().await {
                    match event {
                        StreamEvent::TextDelta(text) => {
                            full_text.push_str(&text);
                            yield AgentEvent::TextDelta(text);
                        }
                        StreamEvent::BlockComplete(block) => {
                            match &block {
                                ContentBlock::ToolUse { id, name, input } => {
                                    tool_calls.push((id.clone(), name.clone(), input.clone()));
                                }
                                ContentBlock::Thinking { thinking } => {
                                    yield AgentEvent::ThinkingDelta(thinking.clone());
                                }
                                _ => {}
                            }
                            blocks.push(block);
                        }
                        StreamEvent::Done { stop_reason: _ } => {
                            if !full_text.is_empty() && !blocks.iter().any(|b| matches!(b, ContentBlock::Text { .. })) {
                                blocks.insert(0, ContentBlock::Text { text: full_text.clone() });
                            }
                            break;
                        }
                        StreamEvent::Error(e) => {
                            yield AgentEvent::Error(e);
                            break;
                        }
                    }
                }
            }

            if !full_text.is_empty() {
                yield AgentEvent::Text(full_text.clone());
            }

            // Push assistant message
            if blocks.is_empty() && !full_text.is_empty() {
                blocks.push(ContentBlock::Text { text: full_text });
            }
            messages.push(Message {
                role: mash_ai::Role::Assistant,
                content: MessageContent::Blocks(blocks),
            });

            if tool_calls.is_empty() {
                yield AgentEvent::TurnEnd;
                break;
            }

            // Execute tool calls (same as non-streaming)
            let mut results = Vec::new();
            for (id, name, input) in &tool_calls {
                yield AgentEvent::ToolCallStart {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                };

                let tool = context.tools.iter().find(|t| t.name() == name);
                let result = match tool {
                    Some(t) => t.execute(input).await,
                    None => ToolResult {
                        content: format!("Unknown tool: {name}"),
                        is_error: true,
                    },
                };

                yield AgentEvent::ToolCallEnd {
                    id: id.clone(),
                    name: name.clone(),
                    result: result
                        .content
                        .lines()
                        .next()
                        .unwrap_or("(empty)")
                        .to_string(),
                    is_error: result.is_error,
                };

                results.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: result.content,
                    is_error: if result.is_error { Some(true) } else { None },
                });
            }

            messages.push(Message {
                role: mash_ai::Role::User,
                content: MessageContent::Blocks(results),
            });

            yield AgentEvent::TurnEnd;
        }

        yield AgentEvent::AgentEnd;
    };

    Box::pin(stream)
}
