use iocraft::prelude::*;
use std::path::Path;

use crate::tui::pages::main_page::MainPage;
use crate::tui::{AppContext, AppMessage};

fn file_name_from_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn extract_quoted_value(input: &str, start: usize) -> Option<(String, usize)> {
    let bytes = input.as_bytes();
    if start >= bytes.len() {
        return None;
    }
    let quote = bytes[start];
    if quote != b'"' && quote != b'\'' {
        return None;
    }

    let mut i = start + 1;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == quote {
            return Some((out, i + 1));
        }
        if b == b'\\' && i + 1 < bytes.len() {
            out.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        out.push(b as char);
        i += 1;
    }
    None
}

fn parse_mash_agent_command(cmd: &str) -> Option<(String, Option<String>)> {
    let marker = "mash agent";
    let idx = cmd.find(marker)?;
    let mut i = idx + marker.len();
    let bytes = cmd.as_bytes();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let (user, mut cursor) = extract_quoted_value(cmd, i)?;

    let mut system = None;
    if let Some(sys_idx_rel) = cmd[cursor..].find("--system") {
        cursor += sys_idx_rel + "--system".len();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if let Some((sys, _)) = extract_quoted_value(cmd, cursor) {
            system = Some(sys);
        }
    }
    Some((user, system))
}

/// Render inline markdown: `code` → bright blue, **bold** → egg-yolk yellow.
/// Both markers are hidden in the output.
fn highlight_inline_code(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();

    while let Some(&(i, ch)) = chars.peek() {
        // **bold**
        if ch == '*' && input[i..].starts_with("**") {
            chars.next(); // first *
            chars.next(); // second *
            let start = i + 2;
            let mut found_close = false;
            while let Some(&(j, c)) = chars.peek() {
                if c == '*' && input[j..].starts_with("**") {
                    let bold_text = &input[start..j];
                    if !bold_text.is_empty() {
                        result.push_str("\x1b[33m");
                        result.push_str(bold_text);
                        result.push_str("\x1b[0m");
                    }
                    chars.next(); // first *
                    chars.next(); // second *
                    found_close = true;
                    break;
                }
                chars.next();
            }
            if !found_close {
                result.push_str("**");
                result.push_str(&input[start..]);
                break;
            }
        // `inline code`
        } else if ch == '`' {
            chars.next();
            let start = i + 1;
            let mut found_close = false;
            while let Some(&(j, c)) = chars.peek() {
                if c == '`' {
                    let code_text = &input[start..j];
                    if !code_text.is_empty() {
                        result.push_str("\x1b[94m");
                        result.push_str(code_text);
                        result.push_str("\x1b[0m");
                    }
                    chars.next();
                    found_close = true;
                    break;
                }
                chars.next();
            }
            if !found_close {
                result.push('`');
                result.push_str(&input[start..]);
                break;
            }
        } else {
            result.push(ch);
            chars.next();
        }
    }

    result
}

#[component]
pub fn App(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let (stdout, _stderr) = hooks.use_output();
    let header_rendered = hooks.use_state(|| false);

    let app_ctx = hooks.use_context::<AppContext>();
    let ui_sender = app_ctx.ui_sender.clone();
    let session_id = app_ctx.session_id.clone();

    // Output welcome header once (with session id).
    let mut header_rendered_clone = header_rendered;
    let stdout_header = stdout.clone();
    hooks.use_future(async move {
        if !*header_rendered_clone.read() {
            stdout_header.println(format!(
                "\x1b[1;34mmash\x1b[0m  \x1b[2msession {}\x1b[0m",
                session_id
            ));
            stdout_header
                .println("Enter 发送 · Shift+Enter 换行 · /sessions 列表 · /msg <id> 发消息");
            stdout_header.println("");
            header_rendered_clone.set(true);
        }
    });

    // Subscribe to messages and print them to stdout.
    let stdout_msgs = stdout.clone();
    hooks.use_future(async move {
        let mut rx = ui_sender.subscribe();
        let mut last_tool_name: Option<String> = None;
        while let Ok(msg) = rx.recv().await {
            match msg {
                AppMessage::UserMessage(text) => {
                    stdout_msgs.println(format!("\x1b[36m▶ {}\x1b[0m", text));
                }
                AppMessage::AssistantLine(line) => {
                    stdout_msgs.println(highlight_inline_code(&line));
                }
                AppMessage::ToolCall { name, description } => {
                    last_tool_name = Some(name.clone());
                    match name.as_str() {
                        "write" => {
                            let file = file_name_from_path(&description);
                            stdout_msgs.println(format!("\x1b[32m⏺ Write({})\x1b[0m", file));
                        }
                        "read" => {
                            let file = file_name_from_path(&description);
                            stdout_msgs.println(format!("\x1b[32m⏺ Read({})\x1b[0m", file));
                        }
                        "edit" => {
                            let file = file_name_from_path(&description);
                            stdout_msgs.println(format!("\x1b[32m⏺ Edit({})\x1b[0m", file));
                        }
                        "bash" => {
                            if let Some((user, system)) = parse_mash_agent_command(&description) {
                                match system {
                                    Some(system) => stdout_msgs.println(format!(
                                        "\x1b[32m⏺ Agent(user={}, system={})\x1b[0m",
                                        user, system
                                    )),
                                    None => stdout_msgs
                                        .println(format!("\x1b[32m⏺ Agent(user={})\x1b[0m", user)),
                                }
                            } else if description.is_empty() {
                                stdout_msgs.println("\x1b[32m⏺ Bash()\x1b[0m");
                            } else {
                                stdout_msgs
                                    .println(format!("\x1b[32m⏺ Bash({})\x1b[0m", description));
                            }
                        }
                        _ => {
                            if description.is_empty() {
                                stdout_msgs.println(format!("\x1b[32m⏺ {}()\x1b[0m", name));
                            } else {
                                stdout_msgs
                                    .println(format!("\x1b[32m⏺ {}({})\x1b[0m", name, description));
                            }
                        }
                    }
                }
                AppMessage::ToolResult { preview } => {
                    let skip = matches!(last_tool_name.as_deref(), Some("read" | "write" | "edit"));
                    if !skip {
                        stdout_msgs.println(format!("\x1b[33m✓ {}\x1b[0m", preview));
                    }
                }
                AppMessage::AgentError(e) => {
                    stdout_msgs.println(format!("\x1b[31mError: {}\x1b[0m", e));
                }
                AppMessage::AgentCompleted => {
                    stdout_msgs.println("");
                }
                AppMessage::SessionList(sessions) => {
                    stdout_msgs.println("\x1b[1mActive sessions:\x1b[0m");
                    for s in &sessions {
                        let status = if s.busy { "busy" } else { "idle" };
                        stdout_msgs.println(format!("  \x1b[33m{}\x1b[0m  {}", s.id, status));
                    }
                    stdout_msgs.println("");
                }
                AppMessage::McpList(servers) => {
                    if servers.is_empty() {
                        stdout_msgs.println("\x1b[1mNo MCP servers connected.\x1b[0m");
                    } else {
                        stdout_msgs.println("\x1b[1mMCP servers:\x1b[0m");
                        for s in &servers {
                            stdout_msgs.println(format!(
                                "  \x1b[33m{}\x1b[0m ({} tools)",
                                s.name,
                                s.tools.len()
                            ));
                            for tool in &s.tools {
                                stdout_msgs.println(format!("    - {tool}"));
                            }
                        }
                    }
                    stdout_msgs.println("");
                }
                AppMessage::PeerMessage { from, text } => {
                    stdout_msgs.println(format!("\x1b[35m◀ [{}] {}\x1b[0m", from, text));
                }
                AppMessage::AgentTaskStarted
                | AppMessage::TasksUpdated { .. }
                | AppMessage::TaskContent(_) => {}
            }
        }
    });

    element! {
        MainPage
    }
}
