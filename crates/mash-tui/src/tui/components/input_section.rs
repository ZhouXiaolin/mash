use std::sync::Arc;

use iocraft::prelude::*;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use mashd::protocol::{ClientMessage, SkillEntry};

use crate::tui::{AppContext, AppMessage};

/// A command entry shown in the slash menu.
#[derive(Clone)]
struct SlashCommand {
    name: String,
    description: String,
    builtin: bool,
}

fn build_commands(skills: &[SkillEntry]) -> Vec<SlashCommand> {
    let mut cmds = vec![
        SlashCommand {
            name: "new".to_string(),
            description: "清空上下文，开始新对话".to_string(),
            builtin: true,
        },
        SlashCommand {
            name: "sessions".to_string(),
            description: "列出所有活跃会话".to_string(),
            builtin: true,
        },
        SlashCommand {
            name: "msg".to_string(),
            description: "向其他会话发送消息 /msg <id> <text>".to_string(),
            builtin: true,
        },
    ];
    for skill in skills {
        cmds.push(SlashCommand {
            name: skill.name.clone(),
            description: skill.description.clone(),
            builtin: false,
        });
    }
    cmds
}

fn filter_commands<'a>(commands: &'a [SlashCommand], query: &str) -> Vec<&'a SlashCommand> {
    if query.is_empty() {
        return commands.iter().collect();
    }
    let q = query.to_lowercase();
    commands
        .iter()
        .filter(|c| c.name.to_lowercase().contains(&q))
        .collect()
}

/// Send a ClientMessage to the daemon over the socket.
fn send_to_daemon(writer: Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>, msg: ClientMessage) {
    tokio::spawn(async move {
        let mut json = serde_json::to_string(&msg).unwrap();
        json.push('\n');
        let mut w = writer.lock().await;
        let _ = w.write_all(json.as_bytes()).await;
        let _ = w.flush().await;
    });
}

/// Try to handle input as a builtin command. Returns true if handled.
fn try_builtin(
    text: &str,
    ui_sender: &tokio::sync::broadcast::Sender<AppMessage>,
    socket_writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
) -> bool {
    let trimmed = text.trim();

    if trimmed == "/new" {
        let _ = ui_sender.send(AppMessage::UserMessage("/new".to_string()));
        send_to_daemon(socket_writer.clone(), ClientMessage::ClearHistory);
        return true;
    }

    if trimmed == "/sessions" {
        let _ = ui_sender.send(AppMessage::UserMessage("/sessions".to_string()));
        send_to_daemon(socket_writer.clone(), ClientMessage::ListSessions);
        return true;
    }

    // /msg <session_id> <text...>
    if let Some(rest) = trimmed.strip_prefix("/msg ") {
        let rest = rest.trim();
        if let Some(space) = rest.find(' ') {
            let target = rest[..space].to_string();
            let msg_text = rest[space + 1..].trim().to_string();
            if !target.is_empty() && !msg_text.is_empty() {
                let _ =
                    ui_sender.send(AppMessage::UserMessage(format!("/msg {target} {msg_text}")));
                send_to_daemon(
                    socket_writer.clone(),
                    ClientMessage::SendToSession {
                        target,
                        text: msg_text,
                    },
                );
                return true;
            }
        }
    }

    false
}

/// Fixed-bottom input section with keyboard handling and slash command menu.
#[component]
pub fn InputSection(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let input_buf = hooks.use_state(String::new);
    let busy = hooks.use_state(|| false);
    let menu_index = hooks.use_state(|| 0usize);
    let (width, _) = hooks.use_terminal_size();

    let app_ctx = hooks.use_context::<AppContext>();
    let ui_sender = app_ctx.ui_sender.clone();
    let socket_writer = app_ctx.socket_writer.clone();
    let skills = app_ctx.skills.clone();

    let all_commands = build_commands(&skills);

    // Track busy state from broadcast messages.
    let mut busy_track = busy;
    let ui_sender_track = ui_sender.clone();
    hooks.use_future(async move {
        let mut rx = ui_sender_track.subscribe();
        while let Ok(msg) = rx.recv().await {
            match msg {
                AppMessage::AgentTaskStarted => busy_track.set(true),
                AppMessage::AgentCompleted | AppMessage::AgentError(_) => busy_track.set(false),
                _ => {}
            }
        }
    });

    // Handle keyboard events.
    hooks.use_terminal_events({
        let mut input_buf = input_buf;
        let mut menu_index = menu_index;
        let ui_sender = ui_sender.clone();
        let socket_writer = socket_writer.clone();
        let all_commands = all_commands.clone();
        move |event| {
            if let TerminalEvent::Key(key) = event {
                if key.kind != KeyEventKind::Press {
                    return;
                }

                let buf_snapshot = input_buf.read().clone();
                let in_menu = buf_snapshot.starts_with('/') && !buf_snapshot[1..].contains(' ');

                match key.code {
                    KeyCode::Up if in_menu => {
                        let idx = *menu_index.read();
                        if idx > 0 {
                            menu_index.set(idx - 1);
                        }
                    }
                    KeyCode::Down if in_menu => {
                        let query = &buf_snapshot[1..];
                        let filtered = filter_commands(&all_commands, query);
                        let idx = *menu_index.read();
                        if idx + 1 < filtered.len() {
                            menu_index.set(idx + 1);
                        }
                    }
                    KeyCode::Tab if in_menu => {
                        let query = &buf_snapshot[1..];
                        let filtered = filter_commands(&all_commands, query);
                        let idx = *menu_index.read();
                        if let Some(cmd) = filtered.get(idx) {
                            input_buf.set(format!("/{}", cmd.name));
                        }
                    }
                    KeyCode::Esc if in_menu => {
                        input_buf.set(String::new());
                        menu_index.set(0);
                    }
                    KeyCode::Enter => {
                        if in_menu {
                            let query = &buf_snapshot[1..];
                            let filtered = filter_commands(&all_commands, query);
                            let idx = *menu_index.read();
                            let selected = filtered
                                .get(idx)
                                .copied()
                                .or_else(|| filtered.iter().find(|c| c.name == query).copied());

                            if let Some(cmd) = selected {
                                if cmd.builtin {
                                    // For /msg, auto-complete to "/msg " and let user type the rest
                                    if cmd.name == "msg" {
                                        input_buf.set("/msg ".to_string());
                                        menu_index.set(0);
                                    } else {
                                        let text = format!("/{}", cmd.name);
                                        input_buf.set(String::new());
                                        menu_index.set(0);
                                        try_builtin(&text, &ui_sender, &socket_writer);
                                    }
                                } else {
                                    let text = format!("/{}", cmd.name);
                                    input_buf.set(String::new());
                                    menu_index.set(0);
                                    let _ = ui_sender.send(AppMessage::UserMessage(text.clone()));
                                    send_to_daemon(
                                        socket_writer.clone(),
                                        ClientMessage::SendMessage {
                                            text,
                                            system_extra: None,
                                        },
                                    );
                                }
                            }
                        } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                            let mut buf = buf_snapshot;
                            buf.push('\n');
                            input_buf.set(buf);
                        } else {
                            let text = buf_snapshot.trim().to_string();
                            if !text.is_empty() {
                                input_buf.set(String::new());
                                // Try builtin commands first
                                if !try_builtin(&text, &ui_sender, &socket_writer) {
                                    let _ = ui_sender.send(AppMessage::UserMessage(text.clone()));
                                    send_to_daemon(
                                        socket_writer.clone(),
                                        ClientMessage::SendMessage {
                                            text,
                                            system_extra: None,
                                        },
                                    );
                                }
                            }
                        }
                    }
                    KeyCode::Char(c) => {
                        let mut buf = buf_snapshot;
                        buf.push(c);
                        if buf.starts_with('/') {
                            menu_index.set(0);
                        }
                        input_buf.set(buf);
                    }
                    KeyCode::Backspace => {
                        let mut buf = buf_snapshot;
                        buf.pop();
                        if buf.starts_with('/') {
                            menu_index.set(0);
                        }
                        input_buf.set(buf);
                    }
                    _ => {}
                }
            }
        }
    });

    let buf = input_buf.read().clone();
    let selected_idx = *menu_index.read();
    let show_menu = buf.starts_with('/') && !buf[1..].contains(' ');

    // Build menu elements
    let menu_items: Vec<AnyElement<'static>> = if show_menu {
        let query = &buf[1..];
        let filtered = filter_commands(&all_commands, query);
        filtered
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let is_selected = i == selected_idx;
                let label = format!(" /{}  {}", cmd.name, cmd.description);
                let max_len = (width as usize).saturating_sub(8);
                let label = if label.len() > max_len {
                    let end = label
                        .char_indices()
                        .map(|(i, _)| i)
                        .take_while(|&i| i <= max_len)
                        .last()
                        .unwrap_or(0);
                    format!("{}…", &label[..end])
                } else {
                    label
                };
                let (fg, bg) = if is_selected {
                    (Color::Black, Color::Yellow)
                } else {
                    (Color::White, Color::DarkGrey)
                };
                element! {
                    View(background_color: bg) {
                        Text(content: label, color: fg)
                    }
                }
                .into_any()
            })
            .collect()
    } else {
        Vec::new()
    };

    let display = if buf.is_empty() {
        "│".to_string()
    } else {
        format!("{}│", buf)
    };

    let input_width = if width > 6 { width - 4 } else { 76 };

    element! {
        View(
            flex_direction: FlexDirection::Column,
            width: input_width,
        ) {
            #(menu_items)
            View(
                border_style: BorderStyle::Round,
                border_color: Color::Yellow,
                padding_left: 1,
                padding_right: 1,
            ) {
                Text(content: display)
            }
        }
    }
}
