use iocraft::prelude::*;
use std::path::Path;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, FontStyle, Theme};
use syntect::parsing::SyntaxSet;

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

fn strip_markdown_heading_prefix(input: &str) -> &str {
    let s = input.trim_start();
    let hash_count = s.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hash_count) {
        let rest = &s[hash_count..];
        if rest.starts_with(' ') {
            return rest.trim_start();
        }
    }
    input
}

fn color_numbered_prefix_blue(input: &str) -> String {
    let s = input;
    let mut digit_end = 0usize;
    for (i, ch) in s.char_indices() {
        if ch.is_ascii_digit() {
            digit_end = i + ch.len_utf8();
            continue;
        }
        break;
    }
    if digit_end > 0 && s[digit_end..].starts_with('.') {
        let marker = &s[..digit_end + 1];
        let rest = &s[digit_end + 1..];
        return format!("\x1b[94m{}\x1b[0m{}", marker, rest);
    }
    s.to_string()
}

fn format_assistant_line(line: &str) -> String {
    let no_heading = strip_markdown_heading_prefix(line);
    let numbered = color_numbered_prefix_blue(no_heading);
    highlight_inline_code(&numbered)
}

fn scope_entry(scope: &str, fg: Color, font_style: Option<syntect::highlighting::FontStyle>) -> syntect::highlighting::ThemeItem {
    syntect::highlighting::ThemeItem {
        scope: scope.parse().unwrap(),
        style: syntect::highlighting::StyleModifier {
            foreground: Some(fg),
            background: None,
            font_style,
        },
    }
}

fn build_theme() -> Theme {
    use syntect::highlighting::FontStyle as FS;
    Theme {
        name: Some("tokyo-night".into()),
        author: Some("mash".into()),
        scopes: vec![
            scope_entry("comment", Color { r: 92, g: 111, b: 139, a: 255 }, Some(FS::ITALIC)),
            scope_entry("comment.block", Color { r: 92, g: 111, b: 139, a: 255 }, Some(FS::ITALIC)),
            scope_entry("comment.line", Color { r: 92, g: 111, b: 139, a: 255 }, Some(FS::ITALIC)),
            scope_entry("string", Color { r: 158, g: 206, b: 106, a: 255 }, None),
            scope_entry("string.unquoted", Color { r: 158, g: 206, b: 106, a: 255 }, None),
            scope_entry("string.regexp", Color { r: 158, g: 206, b: 106, a: 255 }, None),
            scope_entry("string.special", Color { r: 158, g: 206, b: 106, a: 255 }, None),
            scope_entry("constant", Color { r: 255, g: 158, b: 100, a: 255 }, None),
            scope_entry("constant.numeric", Color { r: 255, g: 158, b: 100, a: 255 }, None),
            scope_entry("constant.character", Color { r: 255, g: 158, b: 100, a: 255 }, None),
            scope_entry("constant.character.escape", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("constant.other", Color { r: 255, g: 158, b: 100, a: 255 }, None),
            scope_entry("keyword", Color { r: 187, g: 154, b: 247, a: 255 }, None),
            scope_entry("keyword.control", Color { r: 187, g: 154, b: 247, a: 255 }, None),
            scope_entry("keyword.operator", Color { r: 187, g: 154, b: 247, a: 255 }, None),
            scope_entry("keyword.declaration", Color { r: 187, g: 154, b: 247, a: 255 }, None),
            scope_entry("entity.name.function", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("entity.name.function.member", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("entity.name.function.macro", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("entity.name.type", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("entity.name.class", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("entity.name.struct", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("entity.name.enum", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("entity.name.interface", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("entity.name.tag", Color { r: 187, g: 154, b: 247, a: 255 }, None),
            scope_entry("entity.other.attribute-name", Color { r: 158, g: 206, b: 106, a: 255 }, None),
            scope_entry("entity.other.inherited-class", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("variable", Color { r: 192, g: 202, b: 245, a: 255 }, None),
            scope_entry("variable.parameter", Color { r: 192, g: 202, b: 245, a: 255 }, None),
            scope_entry("variable.function", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("variable.other.member", Color { r: 192, g: 202, b: 245, a: 255 }, None),
            scope_entry("support.function", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("support.class", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("support.type", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("support.constant", Color { r: 255, g: 158, b: 100, a: 255 }, None),
            scope_entry("support.variable", Color { r: 192, g: 202, b: 245, a: 255 }, None),
            scope_entry("storage", Color { r: 187, g: 154, b: 247, a: 255 }, None),
            scope_entry("storage.type", Color { r: 187, g: 154, b: 247, a: 255 }, None),
            scope_entry("storage.modifier", Color { r: 187, g: 154, b: 247, a: 255 }, None),
            scope_entry("punctuation", Color { r: 169, g: 177, b: 214, a: 255 }, None),
            scope_entry("punctuation.bracket", Color { r: 169, g: 177, b: 214, a: 255 }, None),
            scope_entry("punctuation.separator", Color { r: 169, g: 177, b: 214, a: 255 }, None),
            scope_entry("punctuation.terminator", Color { r: 169, g: 177, b: 214, a: 255 }, None),
            scope_entry("markup.heading", Color { r: 130, g: 170, b: 255, a: 255 }, Some(FS::BOLD)),
            scope_entry("markup.list", Color { r: 187, g: 154, b: 247, a: 255 }, None),
            scope_entry("markup.bold", Color { r: 192, g: 202, b: 245, a: 255 }, Some(FS::BOLD)),
            scope_entry("markup.italic", Color { r: 192, g: 202, b: 245, a: 255 }, Some(FS::ITALIC)),
            scope_entry("markup.inserted", Color { r: 158, g: 206, b: 106, a: 255 }, None),
            scope_entry("markup.deleted", Color { r: 247, g: 118, b: 142, a: 255 }, None),
            scope_entry("meta.function-call", Color { r: 130, g: 170, b: 255, a: 255 }, None),
            scope_entry("meta.preprocessor", Color { r: 187, g: 154, b: 247, a: 255 }, None),
        ],
        settings: syntect::highlighting::ThemeSettings {
            foreground: Some(Color {
                r: 169,
                g: 177,
                b: 214,
                a: 255,
            }),
            background: Some(Color {
                r: 26,
                g: 27,
                b: 38,
                a: 255,
            }),
            caret: Some(Color {
                r: 192,
                g: 202,
                b: 245,
                a: 255,
            }),
            selection: Some(Color {
                r: 52,
                g: 54,
                b: 74,
                a: 255,
            }),
            line_highlight: Some(Color {
                r: 36,
                g: 38,
                b: 52,
                a: 255,
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn highlight_code_line(line: &str, hl: &mut HighlightLines, ss: &SyntaxSet) -> String {
    let ranges = hl.highlight_line(line, ss).unwrap_or_default();
    let mut out = String::new();
    for (style, text) in ranges {
        let mut parts = String::new();
        if style.font_style.contains(FontStyle::BOLD) {
            parts.push_str("\x1b[1m");
        }
        if style.font_style.contains(FontStyle::ITALIC) {
            parts.push_str("\x1b[3m");
        }
        if style.font_style.contains(FontStyle::UNDERLINE) {
            parts.push_str("\x1b[4m");
        }
        parts.push_str(&format!(
            "\x1b[38;2;{};{};{}m",
            style.foreground.r, style.foreground.g, style.foreground.b
        ));
        out.push_str(&parts);
        out.push_str(text);
        out.push_str("\x1b[0m");
    }
    out
}

struct CodeBlockState {
    in_block: bool,
    lang: Option<String>,
}

fn detect_fence(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with("```") {
        Some(&trimmed[3..])
    } else {
        None
    }
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

        let ss = SyntaxSet::load_defaults_newlines();
        let theme = build_theme();
        let mut code_state = CodeBlockState {
            in_block: false,
            lang: None,
        };
        let mut hl_lines: Option<HighlightLines> = None;

        while let Ok(msg) = rx.recv().await {
            match msg {
                AppMessage::UserMessage(text) => {
                    stdout_msgs.println(format!("\x1b[36m▶ {}\x1b[0m", text));
                }
                AppMessage::AssistantLine(line) => {
                    if let Some(lang_suffix) = detect_fence(&line) {
                        if code_state.in_block {
                            code_state.in_block = false;
                            hl_lines = None;
                            stdout_msgs.println("\x1b[2m    ╰───\x1b[0m");
                        } else {
                            code_state.in_block = true;
                            code_state.lang = if lang_suffix.is_empty() {
                                None
                            } else {
                                Some(lang_suffix.to_string())
                            };
                            let syntax = code_state
                                .lang
                                .as_deref()
                                .and_then(|l| ss.find_syntax_by_token(l))
                                .or_else(|| ss.find_syntax_by_extension("txt"))
                                .unwrap_or_else(|| ss.find_syntax_plain_text());
                            hl_lines = Some(HighlightLines::new(syntax, &theme));
                            let label = code_state.lang.as_deref().unwrap_or("");
                            stdout_msgs.println(format!(
                                "\x1b[2m    ╭───{}\x1b[0m",
                                if label.is_empty() {
                                    String::new()
                                } else {
                                    format!(" {}", label)
                                }
                            ));
                        }
                    } else if code_state.in_block {
                        let highlighted = match &mut hl_lines {
                            Some(hl) => highlight_code_line(&line, hl, &ss),
                            None => line.clone(),
                        };
                        stdout_msgs.println(format!("    {}", highlighted));
                    } else {
                        stdout_msgs.println(format_assistant_line(&line));
                    }
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
                        // 在toolresult输出后添加空行
                        stdout_msgs.println("");
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
