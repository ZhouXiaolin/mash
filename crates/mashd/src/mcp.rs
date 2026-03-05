use std::collections::HashMap;
use std::process::Stdio;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

// ── Config ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct McpConfigFile {
    #[serde(rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub disabled: bool,
}

// ── MCP Tool ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

// ── MCP Client (single server) ─────────────────────────────────

pub struct McpClient {
    name: String,
    _child: Child,
    stdin: ChildStdin,
    reader: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    tools: Vec<McpTool>,
}

impl McpClient {
    pub async fn connect(name: &str, config: &McpServerConfig) -> Result<Self> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("stdin not captured");
        let stdout = child.stdout.take().expect("stdout not captured");
        let reader = BufReader::new(stdout).lines();

        let mut client = Self {
            name: name.to_string(),
            _child: child,
            stdin,
            reader,
            next_id: 1,
            tools: Vec::new(),
        };

        client.initialize().await?;
        client.tools = client.fetch_tools().await?;
        Ok(client)
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        loop {
            match self.reader.next_line().await? {
                Some(line) if !line.is_empty() => {
                    let msg: Value = serde_json::from_str(&line)?;
                    if msg.get("id").and_then(|v| v.as_u64()) == Some(id) {
                        if let Some(error) = msg.get("error") {
                            bail!("MCP error: {}", error);
                        }
                        return Ok(msg["result"].clone());
                    }
                }
                Some(_) => continue,
                None => bail!("MCP server '{}' closed connection", self.name),
            }
        }
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let mut line = serde_json::to_string(&notification)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn initialize(&mut self) -> Result<()> {
        self.send_request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "mash", "version": "0.1.0" }
            }),
        )
        .await?;

        self.send_notification("notifications/initialized", json!({}))
            .await?;
        Ok(())
    }

    async fn fetch_tools(&mut self) -> Result<Vec<McpTool>> {
        let result = self.send_request("tools/list", json!({})).await?;
        let tools: Vec<McpTool> =
            serde_json::from_value(result.get("tools").cloned().unwrap_or(json!([])))?;
        Ok(tools)
    }

    pub async fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<String> {
        let result = self
            .send_request(
                "tools/call",
                json!({ "name": tool_name, "arguments": arguments }),
            )
            .await?;

        if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
            let texts: Vec<&str> = content
                .iter()
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .collect();
            if !texts.is_empty() {
                return Ok(texts.join("\n"));
            }
        }

        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }
}

// ── MCP Manager (all servers) ──────────────────────────────────

pub struct McpManager {
    configs: HashMap<String, McpServerConfig>,
    clients: HashMap<String, McpClient>,
}

impl McpManager {
    pub fn load() -> Result<Self> {
        let config_path = crate::config::mash_config_path("mcp.json")?;

        let configs = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let file: McpConfigFile = serde_json::from_str(&content)?;
            file.mcp_servers
        } else {
            HashMap::new()
        };

        Ok(Self {
            configs,
            clients: HashMap::new(),
        })
    }

    pub fn configs(&self) -> &HashMap<String, McpServerConfig> {
        &self.configs
    }

    pub fn is_connected(&self, name: &str) -> bool {
        self.clients.contains_key(name)
    }

    pub async fn connect(&mut self, name: &str) -> Result<()> {
        let config = self
            .configs
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not found in config", name))?;

        if config.disabled {
            bail!("MCP server '{}' is disabled", name);
        }

        let client = McpClient::connect(name, &config).await?;
        self.clients.insert(name.to_string(), client);
        Ok(())
    }

    pub async fn connect_all(&mut self) {
        let names: Vec<String> = self
            .configs
            .iter()
            .filter(|(_, c)| !c.disabled)
            .map(|(n, _)| n.clone())
            .collect();

        for name in names {
            let _ = self.connect(&name).await;
        }
    }

    pub fn tool_definitions(&self) -> Vec<Value> {
        let mut defs = Vec::new();
        for (server_name, client) in &self.clients {
            for tool in client.tools() {
                defs.push(json!({
                    "name": format!("mcp__{}__{}", server_name, tool.name),
                    "description": tool.description.clone().unwrap_or_default(),
                    "input_schema": tool.input_schema,
                }));
            }
        }
        defs
    }

    pub async fn call_tool(&mut self, full_name: &str, arguments: &Value) -> Result<String> {
        let rest = full_name
            .strip_prefix("mcp__")
            .ok_or_else(|| anyhow::anyhow!("Invalid MCP tool name: {}", full_name))?;

        let sep = rest
            .find("__")
            .ok_or_else(|| anyhow::anyhow!("Invalid MCP tool name: {}", full_name))?;

        let server_name = &rest[..sep];
        let tool_name = &rest[sep + 2..];

        let client = self
            .clients
            .get_mut(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not connected", server_name))?;

        client.call_tool(tool_name, arguments.clone()).await
    }

    pub fn get_client(&self, name: &str) -> Option<&McpClient> {
        self.clients.get(name)
    }

    /// 按 server 遍历所有工具，用于生成 system prompt。
    pub fn iter_servers_and_tools(&self) -> impl Iterator<Item = (String, &[McpTool])> + '_ {
        self.clients.iter().map(|(n, c)| (n.clone(), c.tools()))
    }

    pub fn clients_iter(&self) -> impl Iterator<Item = (&String, &McpClient)> {
        self.clients.iter()
    }
}

// ── Prompt formatting ─────────────────────────────────────────────

/// 将 MCP 工具列表格式化为 system prompt 片段。
pub fn format_mcp_tools_for_prompt(mcp: &McpManager) -> String {
    let mut lines = Vec::new();
    for (server_name, tools) in mcp.iter_servers_and_tools() {
        for tool in tools {
            let full_name = format!("mcp__{}__{}", server_name, tool.name);
            let desc_short: String = tool
                .description
                .as_deref()
                .unwrap_or("(无描述)")
                .lines()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ");
            let params_block = format_params_block(tool.input_schema.as_object());
            let call_example = format!("mash mcp call {} {} '{{...}}'", server_name, tool.name);
            let block = if params_block.is_empty() {
                format!(
                    "- **{}**\n  描述: {}\n  调用: {}",
                    full_name, desc_short, call_example
                )
            } else {
                format!(
                    "- **{}**\n  描述: {}\n  params:\n{}\n  调用: {}",
                    full_name, desc_short, params_block, call_example
                )
            };
            lines.push(block);
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    let header = "\n\n## MCP 工具（通过 bash 使用 mash mcp call 调用）\n\
        需要调用以下工具时，请使用 bash 执行 `mash mcp call <server> <tool> '<json_arguments>'`。\n\n";
    format!("{header}{}", lines.join("\n\n"))
}

fn format_params_block(input_schema: Option<&serde_json::Map<String, Value>>) -> String {
    let Some(schema) = input_schema else {
        return String::new();
    };
    let props = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return String::new(),
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let mut out = Vec::with_capacity(props.len());
    for (pname, pschema) in props {
        let ptype = pschema
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("any");
        let req = if required.contains(&pname.as_str()) {
            " *"
        } else {
            ""
        };
        out.push(format!("    {pname}: {ptype}{req}"));
    }
    out.join("\n")
}
