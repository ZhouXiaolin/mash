// crates/mash-agent/tests/fib.rs
//! Fibonacci task test with bash tool
//!
//! Test matrix:
//! - Backends: OpenAI, Anthropic
//! - Modes: streaming, non-streaming
//! - Models: deepseek-chat, deepseek-reasoner
//!
//! Total: 8 combinations

use mash_agent::{AgentContext, AgentTool, ToolResult, run, run_streaming};
use mash_ai::anthropic::{AnthropicBackend, AnthropicConfig};
use mash_ai::openai::{OpenAiBackend, OpenAiConfig};
use mash_ai::{Message, MessageContent, Role};
use serde_json::json;
use std::pin::Pin;
use tokio_stream::StreamExt;

fn api_key() -> String {
    std::env::var("DEEPSEEK_API_KEY").unwrap_or_else(|_| "sk-test".to_string())
}

fn anthropic_backend() -> AnthropicBackend {
    AnthropicBackend::new(AnthropicConfig {
        base_url: "https://api.deepseek.com/anthropic".to_string(),
        api_key: api_key(),
    })
}

fn openai_backend() -> OpenAiBackend {
    OpenAiBackend::new(OpenAiConfig {
        base_url: "https://api.deepseek.com".to_string(),
        api_key: api_key(),
    })
}

/// Bash tool for executing shell commands
struct BashTool;

impl BashTool {
    fn execute_command(&self, input: &serde_json::Value) -> ToolResult {
        let cmd = input.get("command").and_then(|c| c.as_str()).unwrap_or("");

        if cmd.is_empty() {
            return ToolResult {
                content: "Error: missing 'command' field".to_string(),
                is_error: true,
            };
        }

        // Use sh -c for more complex commands
        let output = match std::process::Command::new("sh").arg("-c").arg(cmd).output() {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                if !stderr.is_empty() && !o.status.success() {
                    format!("exit={}\nstderr: {}", o.status.code().unwrap_or(-1), stderr)
                } else if !stdout.is_empty() {
                    stdout
                } else if !stderr.is_empty() {
                    stderr
                } else {
                    format!("exit={}", o.status.code().unwrap_or(-1))
                }
            }
            Err(e) => format!("Execution error: {}", e),
        };

        ToolResult {
            content: output,
            is_error: false,
        }
    }
}

impl AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn definition(&self) -> serde_json::Value {
        json!({
            "name": "bash",
            "description": "Execute a bash command and return its output.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The bash command to execute"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds. Default 120."
                    }
                },
                "required": ["command"]
            }
        })
    }

    fn execute(
        &self,
        input: &serde_json::Value,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolResult> + Send + '_>> {
        let result = self.execute_command(input);
        Box::pin(async move { result })
    }
}

/// Test backend type
enum BackendType {
    Anthropic,
    OpenAI,
}

/// Test streaming mode
enum StreamingMode {
    Streaming,
    NonStreaming,
}

/// Test model
enum TestModel {
    DeepSeekChat,
    DeepSeekReasoner,
}

impl TestModel {
    fn as_str(&self) -> &str {
        match self {
            TestModel::DeepSeekChat => "deepseek-chat",
            TestModel::DeepSeekReasoner => "deepseek-reasoner",
        }
    }
}

/// Common test verification state
struct TestVerification {
    got_tool_call: bool,
    got_final_answer: bool,
}

/// Verify result contains the correct Fibonacci number
fn contains_fib_result(text: &str) -> bool {
    text.contains("354224848179261915075")
        || text.contains("354,224,848,179,261,915,075")
        || text.contains("3.542e+20")
}

/// Run a single fibonacci test combination
async fn run_fibonacci_test(
    backend_type: BackendType,
    streaming_mode: StreamingMode,
    model: TestModel,
) -> TestVerification {
    let backend = match backend_type {
        BackendType::Anthropic => Box::new(anthropic_backend()) as Box<dyn mash_ai::LlmClient>,
        BackendType::OpenAI => Box::new(openai_backend()) as Box<dyn mash_ai::LlmClient>,
    };

    let system = "You are a helpful assistant with access to a bash tool. You can execute shell commands to complete tasks. When the user asks you to write code, run commands, or perform calculations, use the bash tool.".to_string();

    let temp_dir = std::env::temp_dir();
    let work_dir = temp_dir.join("mash-agent-test");
    let _ = std::fs::create_dir_all(&work_dir);

    let user_prompt = format!(
        "Write a Python script to calculate the 100th Fibonacci number, save it to {}/fib.py, run it, and tell me the result.",
        work_dir.display()
    );

    let context = AgentContext {
        system,
        model: model.as_str().to_string(),
        max_tokens: 4096,
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text(user_prompt),
        }],
        tools: vec![Box::new(BashTool)],
    };
    let (_tx, rx) = tokio::sync::watch::channel(false);

    let mut verification = TestVerification {
        got_tool_call: false,
        got_final_answer: false,
    };

    let backend_name = match backend_type {
        BackendType::Anthropic => "Anthropic",
        BackendType::OpenAI => "OpenAI",
    };
    let mode_name = match streaming_mode {
        StreamingMode::Streaming => "Streaming",
        StreamingMode::NonStreaming => "NonStreaming",
    };

    println!("=== {} + {} + {} ===\n", backend_name, mode_name, model.as_str());
    println!("Work directory: {}\n", work_dir.display());

    let mut events = match streaming_mode {
        StreamingMode::Streaming => run_streaming(&*backend, context, rx),
        StreamingMode::NonStreaming => run(&*backend, context, rx),
    };

    while let Some(event) = events.next().await {
        match event {
            mash_agent::AgentEvent::AgentStart => {
                println!("[Agent] Starting task...");
            }
            mash_agent::AgentEvent::TurnStart => {
                println!("[Agent] New turn");
            }
            mash_agent::AgentEvent::TextDelta(t) => {
                print!("{}", t);
                std::io::Write::flush(&mut std::io::stdout()).ok();
                if contains_fib_result(&t) {
                    verification.got_final_answer = true;
                }
            }
            mash_agent::AgentEvent::Text(t) => {
                println!();
                if contains_fib_result(&t) {
                    verification.got_final_answer = true;
                }
            }
            mash_agent::AgentEvent::ToolCallStart { name, input, .. } => {
                let cmd = input.get("command").and_then(|c| c.as_str()).unwrap_or("");
                println!("[Tool: {}] {}", name, cmd);
                verification.got_tool_call = true;
            }
            mash_agent::AgentEvent::ToolCallEnd { result, .. } => {
                let lines: Vec<&str> = result.lines().collect();
                if !lines.is_empty() {
                    for line in lines.iter().take(5) {
                        println!("        {}", line);
                        if contains_fib_result(line) {
                            verification.got_final_answer = true;
                        }
                    }
                    if lines.len() > 5 {
                        println!("        ... ({} more lines)", lines.len() - 5);
                    }
                }
            }
            mash_agent::AgentEvent::TurnEnd => {
                println!("[Agent] Turn end\n");
            }
            mash_agent::AgentEvent::AgentEnd => {
                println!("[Agent] Task complete\n");
                break;
            }
            mash_agent::AgentEvent::Error(e) => {
                panic!("Agent error: {}", e);
            }
            _ => {}
        }
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&work_dir);

    verification
}

// ============================================================================
// Anthropic + deepseek-chat
// ============================================================================

#[tokio::test]
#[ignore]
async fn anthropic_deepseek_chat_streaming() {
    let result = run_fibonacci_test(
        BackendType::Anthropic,
        StreamingMode::Streaming,
        TestModel::DeepSeekChat,
    )
    .await;

    assert!(result.got_tool_call, "Should have called bash tool");
    assert!(
        result.got_final_answer,
        "Should have computed fib(100) = 354224848179261915075"
    );
    println!("=== Test Passed ===");
}

#[tokio::test]
#[ignore]
async fn anthropic_deepseek_chat_non_streaming() {
    let result = run_fibonacci_test(
        BackendType::Anthropic,
        StreamingMode::NonStreaming,
        TestModel::DeepSeekChat,
    )
    .await;

    assert!(result.got_tool_call, "Should have called bash tool");
    assert!(
        result.got_final_answer,
        "Should have computed fib(100) = 354224848179261915075"
    );
    println!("=== Test Passed ===");
}

// ============================================================================
// Anthropic + deepseek-reasoner
// ============================================================================

#[tokio::test]
#[ignore]
async fn anthropic_deepseek_reasoner_streaming() {
    let result = run_fibonacci_test(
        BackendType::Anthropic,
        StreamingMode::Streaming,
        TestModel::DeepSeekReasoner,
    )
    .await;

    assert!(result.got_tool_call, "Should have called bash tool");
    assert!(
        result.got_final_answer,
        "Should have computed fib(100) = 354224848179261915075"
    );
    println!("=== Test Passed ===");
}

#[tokio::test]
#[ignore]
async fn anthropic_deepseek_reasoner_non_streaming() {
    let result = run_fibonacci_test(
        BackendType::Anthropic,
        StreamingMode::NonStreaming,
        TestModel::DeepSeekReasoner,
    )
    .await;

    assert!(result.got_tool_call, "Should have called bash tool");
    assert!(
        result.got_final_answer,
        "Should have computed fib(100) = 354224848179261915075"
    );
    println!("=== Test Passed ===");
}

// ============================================================================
// OpenAI + deepseek-chat
// ============================================================================

#[tokio::test]
#[ignore]
async fn openai_deepseek_chat_streaming() {
    let result = run_fibonacci_test(
        BackendType::OpenAI,
        StreamingMode::Streaming,
        TestModel::DeepSeekChat,
    )
    .await;

    assert!(result.got_tool_call, "Should have called bash tool");
    assert!(
        result.got_final_answer,
        "Should have computed fib(100) = 354224848179261915075"
    );
    println!("=== Test Passed ===");
}

#[tokio::test]
#[ignore]
async fn openai_deepseek_chat_non_streaming() {
    let result = run_fibonacci_test(
        BackendType::OpenAI,
        StreamingMode::NonStreaming,
        TestModel::DeepSeekChat,
    )
    .await;

    assert!(result.got_tool_call, "Should have called bash tool");
    assert!(
        result.got_final_answer,
        "Should have computed fib(100) = 354224848179261915075"
    );
    println!("=== Test Passed ===");
}

// ============================================================================
// OpenAI + deepseek-reasoner
// ============================================================================

#[tokio::test]
#[ignore]
async fn openai_deepseek_reasoner_streaming() {
    let result = run_fibonacci_test(
        BackendType::OpenAI,
        StreamingMode::Streaming,
        TestModel::DeepSeekReasoner,
    )
    .await;

    assert!(result.got_tool_call, "Should have called bash tool");
    assert!(
        result.got_final_answer,
        "Should have computed fib(100) = 354224848179261915075"
    );
    println!("=== Test Passed ===");
}

#[tokio::test]
#[ignore]
async fn openai_deepseek_reasoner_non_streaming() {
    let result = run_fibonacci_test(
        BackendType::OpenAI,
        StreamingMode::NonStreaming,
        TestModel::DeepSeekReasoner,
    )
    .await;

    assert!(result.got_tool_call, "Should have called bash tool");
    assert!(
        result.got_final_answer,
        "Should have computed fib(100) = 354224848179261915075"
    );
    println!("=== Test Passed ===");
}

// ============================================================================
// Legacy test (kept for compatibility)
// ============================================================================

#[tokio::test]
#[ignore]
async fn simple_bash_tool_test() {
    let backend = anthropic_backend();
    let context = AgentContext {
        system: "You are a helpful assistant with access to a bash tool. When the user asks you to execute commands or check system information, use the bash tool.".to_string(),
        model: "deepseek-chat".to_string(),
        max_tokens: 1024,
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("Use bash to echo 'hello world'".to_string()),
        }],
        tools: vec![Box::new(BashTool)],
    };
    let (_tx, rx) = tokio::sync::watch::channel(false);

    let mut events = run_streaming(&backend, context, rx);
    let mut got_tool_call = false;
    let mut tool_result = String::new();

    println!("=== Simple Bash Tool Test ===\n");

    while let Some(event) = events.next().await {
        match event {
            mash_agent::AgentEvent::AgentStart => {
                println!("[Agent] Starting...");
            }
            mash_agent::AgentEvent::TextDelta(t) => {
                print!("{}", t);
                std::io::Write::flush(&mut std::io::stdout()).ok();
            }
            mash_agent::AgentEvent::Text(t) => {
                println!("\n[Agent] {}", t);
            }
            mash_agent::AgentEvent::ToolCallStart { name, input, .. } => {
                println!("[Tool Call] {} {:?}", name, input);
                got_tool_call = true;
            }
            mash_agent::AgentEvent::ToolCallEnd { result, .. } => {
                println!("[Tool Result] {}", result);
                tool_result = result;
            }
            mash_agent::AgentEvent::AgentEnd => {
                println!("[Agent] Done");
                break;
            }
            mash_agent::AgentEvent::Error(e) => {
                panic!("Error: {}", e);
            }
            _ => {}
        }
    }

    assert!(got_tool_call, "Should have called bash tool");
    assert!(
        tool_result.contains("hello world"),
        "Should have echoed hello world"
    );
    println!("\n=== Test Passed ===");
}
