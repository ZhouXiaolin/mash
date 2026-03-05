use iocraft::prelude::*;

use crate::tui::pages::main_page::MainPage;
use crate::tui::{AppContext, AppMessage};

/// Replace backtick-wrapped segments with bright-blue ANSI output.
fn highlight_inline_code(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();

    while let Some(&(i, ch)) = chars.peek() {
        if ch == '`' {
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
        while let Ok(msg) = rx.recv().await {
            match msg {
                AppMessage::UserMessage(text) => {
                    stdout_msgs.println(format!("\x1b[36m▶ {}\x1b[0m", text));
                }
                AppMessage::AssistantLine(line) => {
                    stdout_msgs.println(highlight_inline_code(&line));
                }
                AppMessage::ToolCall { name, description } => {
                    let label = if name == "bash" { "Bash" } else { &name };
                    if description.is_empty() {
                        stdout_msgs.println(format!("\x1b[32m⏺ {}()\x1b[0m", label));
                    } else {
                        let term_width = crossterm::terminal::size()
                            .map(|(w, _)| w as usize)
                            .unwrap_or(80);
                        let lines: Vec<&str> = description.lines().collect();
                        let prefix = format!("⏺ {}(", label);
                        let indent_width = prefix.chars().count();
                        let padding = " ".repeat(indent_width);

                        let truncate = |s: &str, max: usize| -> String {
                            if s.chars().count() <= max {
                                s.to_string()
                            } else {
                                let truncated: String =
                                    s.chars().take(max.saturating_sub(1)).collect();
                                format!("{}…", truncated)
                            }
                        };

                        let first_avail = term_width.saturating_sub(indent_width + 1);
                        let rest_avail = term_width.saturating_sub(indent_width + 1);

                        let mut output =
                            format!("\x1b[32m{}{}", prefix, truncate(lines[0], first_avail));
                        for line in &lines[1..] {
                            output.push('\n');
                            output.push_str(&format!(
                                "\x1b[32m{}{}",
                                padding,
                                truncate(line, rest_avail)
                            ));
                        }
                        output.push_str(")\x1b[0m");
                        stdout_msgs.println(output);
                    }
                }
                AppMessage::ToolResult { preview } => {
                    stdout_msgs.println(format!("\x1b[33m✓ {}\x1b[0m", preview));
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
