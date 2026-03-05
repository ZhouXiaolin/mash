use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{Mutex, broadcast};

use mash_ai::{Message, MessageContent, Role};
use mashd::agent::{self, AgentConfig, AgentEvent};
use mashd::mcp::McpManager;
use mashd::protocol::{ClientMessage, DaemonMessage, McpServerInfo, SessionInfo, SkillEntry};
use mashd::skills;
use mashd::tasks;

// ── Shared state (global, all sessions) ─────────────────────────

struct SharedState {
    base_agent_config: AgentConfig,
    #[allow(dead_code)] // kept for future per-session MCP access
    mcp: Arc<Mutex<McpManager>>,
    skills: Vec<SkillEntry>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

// ── Per-session state ───────────────────────────────────────────

struct Session {
    id: String,
    messages: Arc<Mutex<Vec<Message>>>,
    pending_user_messages: Arc<Mutex<Vec<String>>>,
    task_file: PathBuf,
    event_tx: broadcast::Sender<DaemonMessage>,
    busy: AtomicBool,
    system_extra: Mutex<Option<String>>,
}

impl Session {
    fn new(id: String, task_file: PathBuf) -> Self {
        let (event_tx, _) = broadcast::channel::<DaemonMessage>(256);
        Self {
            id,
            messages: Arc::new(Mutex::new(Vec::new())),
            pending_user_messages: Arc::new(Mutex::new(Vec::new())),
            task_file,
            event_tx,
            busy: AtomicBool::new(false),
            system_extra: Mutex::new(None),
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn sock_path() -> PathBuf {
    dirs::home_dir()
        .expect("could not determine home directory")
        .join(".mash")
        .join("mash.sock")
}

fn pid_path() -> PathBuf {
    dirs::home_dir()
        .expect("could not determine home directory")
        .join(".mash")
        .join("mashd.pid")
}

fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Short hex id: last 8 hex chars of timestamp + random suffix
    let rand_part: u16 = (ts & 0xFFFF) as u16 ^ std::process::id() as u16;
    format!("{:04x}{:04x}", (ts >> 16) as u16, rand_part)
}

// ── Main ────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let sock = sock_path();

    // Clean up stale socket file
    if sock.exists() {
        match tokio::net::UnixStream::connect(&sock).await {
            Ok(_) => bail!("mashd is already running"),
            Err(_) => std::fs::remove_file(&sock)?,
        }
    }

    // Ensure ~/.mash/ exists
    if let Some(parent) = sock.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write PID file
    std::fs::write(pid_path(), std::process::id().to_string())?;

    // ── Initialize core ─────────────────────────────────────────
    let mut mcp = McpManager::load()?;
    let configs = mcp.configs().clone();
    let mut names: Vec<String> = configs
        .iter()
        .filter(|(_, c)| !c.disabled)
        .map(|(n, _)| n.clone())
        .collect();
    names.sort();

    for name in &names {
        match mcp.connect(name).await {
            Ok(()) => {
                let count = mcp.get_client(name).map(|c| c.tool_count()).unwrap_or(0);
                eprintln!("  ✓ MCP: {name} ({count} tools)");
            }
            Err(e) => eprintln!("  ✗ MCP: {name} — {e}"),
        }
    }

    let mcp = Arc::new(Mutex::new(mcp));
    let raw_skills = skills::scan_skills();

    // System prompt template (task prompt will be per-session)
    let system_prompt_base = format!(
        "{}{}{}",
        agent::SYSTEM_PROMPT,
        mashd::mcp::format_mcp_tools_for_prompt(&*mcp.lock().await),
        skills::format_skills_for_prompt(&raw_skills),
    );

    let base_agent_config = AgentConfig {
        system: system_prompt_base,
        ..Default::default()
    };

    let skill_entries: Vec<SkillEntry> = raw_skills
        .iter()
        .map(|s| SkillEntry {
            name: s.name.clone(),
            description: s.description.clone(),
        })
        .collect();

    let shared = Arc::new(SharedState {
        base_agent_config,
        mcp,
        skills: skill_entries,
        sessions: Mutex::new(HashMap::new()),
    });

    // ── Listen for client connections ───────────────────────────
    let listener = UnixListener::bind(&sock)?;
    eprintln!("mashd listening on {}", sock.display());

    // Ctrl+C → graceful shutdown
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_ref = shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        shutdown_ref.store(true, Ordering::Relaxed);
    });

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
                let shared = Arc::clone(&shared);
                tokio::spawn(handle_client(stream, shared));
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
    }

    // Cleanup
    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(pid_path());
    eprintln!("mashd shutting down");

    Ok(())
}

// ── Client handler ──────────────────────────────────────────────

async fn handle_client(stream: tokio::net::UnixStream, shared: Arc<SharedState>) {
    let (reader, writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let writer = Arc::new(Mutex::new(writer));

    // Create a new session for this client
    let session_id = generate_session_id();
    let task_file = match tasks::init_task_file() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("failed to init task file: {e}");
            return;
        }
    };

    let session = Arc::new(Session::new(session_id.clone(), task_file.clone()));
    shared
        .sessions
        .lock()
        .await
        .insert(session_id.clone(), Arc::clone(&session));

    eprintln!("session {session_id} connected");

    // Start task-file polling for this session
    let poll_tx = session.event_tx.clone();
    let poll_task_file = task_file.clone();
    let poll_handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if let Some(content) = tasks::read_task_content(&poll_task_file)
                && content.contains("- [")
            {
                if let Some((done, total)) = tasks::read_task_summary(&poll_task_file) {
                    let _ = poll_tx.send(DaemonMessage::TasksUpdated { done, total });
                }
                let _ = poll_tx.send(DaemonMessage::TaskContent { content });
            }
        }
    });

    // Send Welcome
    let welcome = DaemonMessage::Welcome {
        session_id: session_id.clone(),
        skills: shared.skills.clone(),
        busy: false,
    };
    if send_msg(&writer, &welcome).await.is_err() {
        cleanup_session(&shared, &session_id, poll_handle).await;
        return;
    }

    // Subscribe to this session's events
    let mut rx = session.event_tx.subscribe();

    loop {
        tokio::select! {
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        let Ok(msg) = serde_json::from_str::<ClientMessage>(&line) else {
                            continue;
                        };
                        handle_message(msg, &shared, &session, &writer).await;
                    }
                    Ok(None) | Err(_) => break,
                }
            }
            event = rx.recv() => {
                if let Ok(msg) = event
                    && send_msg(&writer, &msg).await.is_err()
                {
                    break;
                }
            }
        }
    }

    cleanup_session(&shared, &session_id, poll_handle).await;
}

async fn cleanup_session(
    shared: &SharedState,
    session_id: &str,
    poll_handle: tokio::task::JoinHandle<()>,
) {
    poll_handle.abort();
    shared.sessions.lock().await.remove(session_id);
    eprintln!("session {session_id} disconnected");
}

async fn handle_message(
    msg: ClientMessage,
    shared: &Arc<SharedState>,
    session: &Arc<Session>,
    writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
) {
    match msg {
        ClientMessage::SendMessage { text, system_extra } => {
            // Store system_extra on first message if provided
            if let Some(extra) = system_extra {
                *session.system_extra.lock().await = Some(extra);
            }
            if session.busy.load(Ordering::Relaxed) {
                session.pending_user_messages.lock().await.push(text);
            } else {
                session.busy.store(true, Ordering::Relaxed);
                let _ = session.event_tx.send(DaemonMessage::AgentStarted);
                spawn_agent_loop(text, shared, session);
            }
        }
        ClientMessage::ClearHistory => {
            session.messages.lock().await.clear();
        }
        ClientMessage::ListSessions => {
            let sessions = shared.sessions.lock().await;
            let list: Vec<SessionInfo> = sessions
                .values()
                .map(|s| SessionInfo {
                    id: s.id.clone(),
                    busy: s.busy.load(Ordering::Relaxed),
                })
                .collect();
            let _ = send_msg(writer, &DaemonMessage::SessionList { sessions: list }).await;
        }
        ClientMessage::ListMcp => {
            let mcp = shared.mcp.lock().await;
            let servers: Vec<McpServerInfo> = mcp
                .clients_iter()
                .map(|(name, client)| McpServerInfo {
                    name: name.clone(),
                    tools: client.tools().iter().map(|t| t.name.clone()).collect(),
                })
                .collect();
            let _ = send_msg(writer, &DaemonMessage::McpList { servers }).await;
        }
        ClientMessage::SendToSession { target, text } => {
            let sessions = shared.sessions.lock().await;
            if let Some(target_session) = sessions.get(&target) {
                // Notify target TUI
                let _ = target_session.event_tx.send(DaemonMessage::PeerMessage {
                    from: session.id.clone(),
                    text: text.clone(),
                });
                // Inject into target session's context so its agent sees it
                let context_msg = format!(
                    "[来自 session {} 的消息]\n{}\n\n（使用 bash 执行 `mash msg {} \"你的回复\"` 回复该 session）",
                    session.id, text, session.id
                );
                target_session
                    .pending_user_messages
                    .lock()
                    .await
                    .push(context_msg);
            } else {
                let _ = send_msg(
                    writer,
                    &DaemonMessage::AgentError {
                        message: format!("session '{target}' not found"),
                    },
                )
                .await;
            }
        }
        ClientMessage::McpCall {
            server,
            tool,
            arguments,
        } => {
            let full_name = format!("mcp__{server}__{tool}");
            let result = shared
                .mcp
                .lock()
                .await
                .call_tool(&full_name, &arguments)
                .await;
            let msg = match result {
                Ok(output) => DaemonMessage::McpResult { result: output },
                Err(e) => DaemonMessage::McpResult {
                    result: format!("Error: {e}"),
                },
            };
            let _ = send_msg(writer, &msg).await;
        }
    }
}

// ── Send helper ─────────────────────────────────────────────────

async fn send_msg(
    writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    msg: &DaemonMessage,
) -> Result<()> {
    let mut json = serde_json::to_string(msg)?;
    json.push('\n');
    let mut w = writer.lock().await;
    w.write_all(json.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

// ── Agent loop ──────────────────────────────────────────────────

fn spawn_agent_loop(input: String, shared: &Arc<SharedState>, session: &Arc<Session>) {
    let base_config = shared.base_agent_config.clone();
    let session = Arc::clone(session);

    tokio::spawn(async move {
        // Build config with optional system_extra for this session
        let config = match session.system_extra.lock().await.as_deref() {
            Some(extra) => AgentConfig {
                system: format!("{}\n\n{}", base_config.system, extra),
                ..base_config.clone()
            },
            None => base_config.clone(),
        };

        // Push user message into history
        session.messages.lock().await.push(Message {
            role: Role::User,
            content: MessageContent::Text(input),
        });

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

        // Forward agent events → session broadcast
        let event_tx = session.event_tx.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let msg = match event {
                    AgentEvent::Text(line) => DaemonMessage::Text { line },
                    AgentEvent::ToolCall { name, description } => {
                        DaemonMessage::ToolCall { name, description }
                    }
                    AgentEvent::ToolResult { preview } => DaemonMessage::ToolResult { preview },
                    AgentEvent::TasksUpdated { done, total } => {
                        DaemonMessage::TasksUpdated { done, total }
                    }
                };
                let _ = event_tx.send(msg);
            }
        });

        let result = agent::run_agent_loop(
            config,
            &session.messages,
            tx,
            &session.pending_user_messages,
        )
        .await;

        let _ = forwarder.await;

        match result {
            Ok(()) => {
                let _ = session.event_tx.send(DaemonMessage::AgentCompleted);
            }
            Err(e) => {
                let _ = session.event_tx.send(DaemonMessage::AgentError {
                    message: e.to_string(),
                });
            }
        }

        session.busy.store(false, Ordering::Relaxed);
    });
}
