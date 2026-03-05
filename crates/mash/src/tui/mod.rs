pub mod app;
pub mod components;
pub mod pages;

use std::sync::Arc;

use anyhow::Result;
use iocraft::prelude::*;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, broadcast};

use mash_core::protocol::{DaemonMessage, SessionInfo, SkillEntry};

/// Messages broadcast between TUI components (internal to client).
#[derive(Debug, Clone)]
pub enum AppMessage {
    UserMessage(String),
    AssistantLine(String),
    ToolCall { name: String, description: String },
    ToolResult { preview: String },
    AgentTaskStarted,
    AgentCompleted,
    AgentError(String),
    TasksUpdated { done: usize, total: usize },
    TaskContent(String),
    SessionList(Vec<SessionInfo>),
    PeerMessage { from: String, text: String },
}

/// Shared application context passed via ContextProvider.
#[derive(Clone)]
pub struct AppContext {
    pub ui_sender: broadcast::Sender<AppMessage>,
    pub socket_writer: Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    pub skills: Arc<Vec<SkillEntry>>,
    pub session_id: Arc<String>,
}

pub async fn run(stream: UnixStream) -> Result<()> {
    let (reader, writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let writer = Arc::new(Mutex::new(writer));

    let (ui_sender, _) = broadcast::channel::<AppMessage>(256);

    // Read Welcome message from daemon
    let welcome_line = lines
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("daemon disconnected before Welcome"))?;
    let welcome: DaemonMessage = serde_json::from_str(&welcome_line)?;

    let (session_id, skills, initial_busy) = match welcome {
        DaemonMessage::Welcome {
            session_id,
            skills,
            busy,
        } => (session_id, skills, busy),
        _ => (String::new(), Vec::new(), false),
    };

    // Fallback: generate a local session id if daemon didn't provide one
    let session_id = if session_id.is_empty() {
        format!("{:08x}", std::process::id())
    } else {
        session_id
    };

    if initial_busy {
        let _ = ui_sender.send(AppMessage::AgentTaskStarted);
    }

    let ctx = AppContext {
        ui_sender: ui_sender.clone(),
        socket_writer: writer,
        skills: Arc::new(skills),
        session_id: Arc::new(session_id),
    };

    // Spawn socket reader: DaemonMessage → AppMessage
    let ui_sender_fwd = ui_sender.clone();
    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(msg) = serde_json::from_str::<DaemonMessage>(&line) else {
                continue;
            };
            let app_msg = match msg {
                DaemonMessage::AgentStarted => AppMessage::AgentTaskStarted,
                DaemonMessage::Text { line } => AppMessage::AssistantLine(line),
                DaemonMessage::ToolCall { name, description } => {
                    AppMessage::ToolCall { name, description }
                }
                DaemonMessage::ToolResult { preview } => AppMessage::ToolResult { preview },
                DaemonMessage::TasksUpdated { done, total } => {
                    AppMessage::TasksUpdated { done, total }
                }
                DaemonMessage::TaskContent { content } => AppMessage::TaskContent(content),
                DaemonMessage::AgentCompleted => AppMessage::AgentCompleted,
                DaemonMessage::AgentError { message } => AppMessage::AgentError(message),
                DaemonMessage::SessionList { sessions } => AppMessage::SessionList(sessions),
                DaemonMessage::PeerMessage { from, text } => AppMessage::PeerMessage { from, text },
                DaemonMessage::Welcome { .. } | DaemonMessage::McpResult { .. } => continue,
            };
            let _ = ui_sender_fwd.send(app_msg);
        }
    });

    element! {
        ContextProvider(value: Context::owned(ctx)) {
            app::App
        }
    }
    .render_loop()
    .await?;

    Ok(())
}
