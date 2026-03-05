use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::time::timeout;

use mashd::mcp::McpManager;
use mashd::protocol::{ClientMessage, DaemonMessage};

#[derive(Parser)]
#[command(name = "mash", version, about = "A minimal Claude agent")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// MCP server management
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// Session management
    Sessions {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Send a message to another session
    Msg {
        /// Target session ID
        session_id: String,
        /// Message text
        text: Vec<String>,
    },
    /// Run agent headlessly and print the result
    Agent {
        /// The prompt to send to the agent
        prompt: String,
        /// Extra instructions to append to the system prompt
        #[arg(short, long)]
        system: Option<String>,
    },
}

#[derive(Subcommand)]
enum McpAction {
    /// List all configured MCP servers and their status
    List,
    /// Show tools for a specific MCP server
    Tools {
        /// MCP server name
        name: String,
    },
    /// Call an MCP tool via the daemon
    Call {
        /// MCP server name
        server: String,
        /// Tool name
        tool: String,
        /// JSON arguments (optional)
        #[arg(default_value = "{}")]
        arguments: String,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// List all active sessions on the daemon
    List,
}

fn sock_path() -> PathBuf {
    dirs::home_dir()
        .expect("could not determine home directory")
        .join(".mash")
        .join("mash.sock")
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => run_client().await,
        Some(Commands::Mcp { action }) => match action {
            McpAction::List => cmd_mcp_list().await,
            McpAction::Tools { name } => cmd_mcp_tools(&name).await,
            McpAction::Call {
                server,
                tool,
                arguments,
            } => cmd_mcp_call(&server, &tool, &arguments).await,
        },
        Some(Commands::Sessions { action }) => match action {
            SessionAction::List => cmd_sessions_list().await,
        },
        Some(Commands::Msg { session_id, text }) => cmd_msg(&session_id, &text.join(" ")).await,
        Some(Commands::Agent { prompt, system }) => cmd_agent(&prompt, system.as_deref()).await,
    }
}

async fn run_client() -> Result<()> {
    let sock = sock_path();

    let stream = match tokio::net::UnixStream::connect(&sock).await {
        Ok(s) => s,
        Err(_) => {
            eprintln!("Starting mashd...");
            auto_start_daemon()?;

            let mut connected = None;
            for _ in 0..300 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if let Ok(s) = tokio::net::UnixStream::connect(&sock).await {
                    connected = Some(s);
                    break;
                }
            }

            connected.ok_or_else(|| anyhow::anyhow!("Failed to connect to mashd"))?
        }
    };

    mash_tui::tui::run(stream).await
}

fn auto_start_daemon() -> Result<()> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().unwrap();
    let mashd = dir.join("mashd");

    let cmd = if mashd.exists() {
        mashd
    } else {
        PathBuf::from("mashd")
    };

    let log_path = dirs::home_dir().unwrap().join(".mash").join("mashd.log");
    let log_file = std::fs::File::create(&log_path)?;

    std::process::Command::new(cmd)
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .process_group(0)
        .spawn()?;

    Ok(())
}

async fn cmd_mcp_list() -> Result<()> {
    let mut manager = McpManager::load()?;

    if manager.configs().is_empty() {
        println!("No MCP servers configured.");
        println!("Add servers to ~/.mash/mcp.json");
        return Ok(());
    }

    println!("MCP Servers:\n");

    let mut names: Vec<String> = manager.configs().keys().cloned().collect();
    names.sort();

    for name in &names {
        let config = manager.configs()[name].clone();
        let cmd_line = if config.args.is_empty() {
            config.command.clone()
        } else {
            format!("{} {}", config.command, config.args.join(" "))
        };

        print!("  {name}  [{cmd_line}]");

        if config.disabled {
            println!("  — disabled");
            continue;
        }

        match timeout(Duration::from_secs(30), manager.connect(name)).await {
            Ok(Ok(())) => {
                let count = manager
                    .get_client(name)
                    .map(|c| c.tool_count())
                    .unwrap_or(0);
                println!("  — ✓ connected ({count} tools)");
            }
            Ok(Err(e)) => println!("  — ✗ {e}"),
            Err(_) => println!("  — ✗ timeout"),
        }
    }

    Ok(())
}

async fn cmd_mcp_tools(name: &str) -> Result<()> {
    let mut manager = McpManager::load()?;

    if !manager.configs().contains_key(name) {
        println!("MCP server '{name}' not found in config.");
        let available: Vec<&String> = manager.configs().keys().collect();
        if !available.is_empty() {
            println!(
                "Available: {}",
                available
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return Ok(());
    }

    println!("Connecting to '{name}'...");
    manager.connect(name).await?;

    let client = manager.get_client(name).unwrap();
    let tools = client.tools();

    if tools.is_empty() {
        println!("No tools available.");
        return Ok(());
    }

    println!("\nTools for '{name}' ({} total):\n", tools.len());

    for tool in tools {
        println!("  {}", tool.name);
        if let Some(desc) = &tool.description {
            for line in desc.lines().take(3) {
                println!("    {line}");
            }
        }

        if let Some(props) = tool
            .input_schema
            .get("properties")
            .and_then(|p| p.as_object())
        {
            let required: Vec<&str> = tool
                .input_schema
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();

            println!("    params:");
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
                println!("      {pname}: {ptype}{req}");
            }
        }
        println!();
    }

    Ok(())
}

async fn cmd_msg(session_id: &str, text: &str) -> Result<()> {
    let sock = sock_path();
    let stream = tokio::net::UnixStream::connect(&sock).await?;
    let (_, write_half) = stream.into_split();
    let mut writer = tokio::io::BufWriter::new(write_half);

    let msg = ClientMessage::SendToSession {
        target: session_id.to_string(),
        text: text.to_string(),
    };
    let mut json = serde_json::to_string(&msg)?;
    json.push('\n');
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;

    println!("Message sent to session {session_id}.");
    Ok(())
}

async fn cmd_mcp_call(server: &str, tool: &str, arguments: &str) -> Result<()> {
    let args: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|e| anyhow::anyhow!("Invalid JSON arguments: {e}"))?;

    let sock = sock_path();
    let stream = tokio::net::UnixStream::connect(&sock).await?;
    let (read_half, write_half) = stream.into_split();
    let mut writer = tokio::io::BufWriter::new(write_half);
    let mut lines = tokio::io::BufReader::new(read_half).lines();

    let msg = ClientMessage::McpCall {
        server: server.to_string(),
        tool: tool.to_string(),
        arguments: args,
    };
    let mut json = serde_json::to_string(&msg)?;
    json.push('\n');
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;

    // Read lines until we get McpResult
    while let Some(line) = lines.next_line().await? {
        if let Ok(daemon_msg) = serde_json::from_str::<DaemonMessage>(&line) {
            match daemon_msg {
                DaemonMessage::McpResult { result } => {
                    print!("{result}");
                    return Ok(());
                }
                _ => continue,
            }
        }
    }

    anyhow::bail!("Connection closed without receiving MCP result")
}

async fn cmd_sessions_list() -> Result<()> {
    let sock = sock_path();
    let stream = tokio::net::UnixStream::connect(&sock).await?;
    let (read_half, write_half) = stream.into_split();
    let mut writer = tokio::io::BufWriter::new(write_half);
    let mut lines = tokio::io::BufReader::new(read_half).lines();

    let msg = ClientMessage::ListSessions;
    let mut json = serde_json::to_string(&msg)?;
    json.push('\n');
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;

    while let Some(line) = lines.next_line().await? {
        if let Ok(daemon_msg) = serde_json::from_str::<DaemonMessage>(&line) {
            match daemon_msg {
                DaemonMessage::SessionList { sessions } => {
                    if sessions.is_empty() {
                        println!("No active sessions.");
                    } else {
                        println!("Active sessions:\n");
                        for s in &sessions {
                            let status = if s.busy { "busy" } else { "idle" };
                            println!("  {}  [{}]", s.id, status);
                        }
                    }
                    return Ok(());
                }
                _ => continue,
            }
        }
    }

    anyhow::bail!("Connection closed without receiving session list")
}

async fn cmd_agent(prompt: &str, system_extra: Option<&str>) -> Result<()> {
    let sock = sock_path();

    let stream = match tokio::net::UnixStream::connect(&sock).await {
        Ok(s) => s,
        Err(_) => {
            eprintln!("Starting mashd...");
            auto_start_daemon()?;

            let mut connected = None;
            for _ in 0..300 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if let Ok(s) = tokio::net::UnixStream::connect(&sock).await {
                    connected = Some(s);
                    break;
                }
            }

            connected.ok_or_else(|| anyhow::anyhow!("Failed to connect to mashd"))?
        }
    };

    let (read_half, write_half) = stream.into_split();
    let mut writer = tokio::io::BufWriter::new(write_half);
    let mut lines = tokio::io::BufReader::new(read_half).lines();

    // Wait for Welcome
    while let Some(line) = lines.next_line().await? {
        if let Ok(DaemonMessage::Welcome { .. }) = serde_json::from_str(&line) {
            break;
        }
    }

    // Send the prompt
    let msg = ClientMessage::SendMessage {
        text: prompt.to_string(),
        system_extra: system_extra.map(|s| s.to_string()),
    };
    let mut json = serde_json::to_string(&msg)?;
    json.push('\n');
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;

    // Collect only the last round of text (after all tool calls finish)
    let mut output = Vec::new();
    while let Some(line) = lines.next_line().await? {
        let Ok(daemon_msg) = serde_json::from_str::<DaemonMessage>(&line) else {
            continue;
        };
        match daemon_msg {
            DaemonMessage::Text { line } => {
                output.push(line);
            }
            DaemonMessage::ToolCall { .. } => {
                output.clear();
            }
            DaemonMessage::AgentCompleted => break,
            DaemonMessage::AgentError { message } => {
                anyhow::bail!("Agent error: {message}");
            }
            _ => {}
        }
    }

    println!("{}", output.join("\n"));
    Ok(())
}
