// crates/mash-core/src/tool_adapter.rs
use serde_json::json;
use serde_json::Value;
use std::pin::Pin;
use std::process::Command;

use mash_agent::{AgentTool, ToolResult};

/// Wrapper for bash tool implementing AgentTool trait.
pub struct BashTool;

impl AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn definition(&self) -> Value {
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
        input: &Value,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolResult> + Send + '_>> {
        let input = input.clone();
        Box::pin(async move {
            match exec_bash(&input) {
                Ok(output) => ToolResult {
                    content: output,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    content: e.to_string(),
                    is_error: true,
                },
            }
        })
    }
}

fn exec_bash(input: &Value) -> anyhow::Result<String> {
    let command = input["command"].as_str().unwrap_or_default();
    let output = Command::new("bash").arg("-c").arg(command).output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut result = String::new();
    if !stdout.is_empty() {
        result.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str("[stderr]\n");
        result.push_str(&stderr);
    }
    if !output.status.success() {
        result.push_str(&format!(
            "\n[exit code: {}]",
            output.status.code().unwrap_or(-1)
        ));
    }
    Ok(result)
}

/// Create all tool instances.
pub fn create_tools() -> Vec<Box<dyn AgentTool>> {
    vec![Box::new(BashTool)]
}
