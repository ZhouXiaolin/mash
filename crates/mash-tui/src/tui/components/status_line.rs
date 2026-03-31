use iocraft::prelude::*;
use serde_json::Value;
use std::time::Duration;

use crate::tui::{AppContext, AppMessage};

struct TaskLine {
    text: String,
    done: bool,
}

fn parse_taskgraph_lines(content: &str) -> Option<Vec<TaskLine>> {
    let mut lines = Vec::new();
    let mut idx = 0usize;

    for raw in content.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        let Some(id) = v.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(desc) = v.get("desc").and_then(Value::as_str) else {
            continue;
        };
        let Some(typ) = v.get("type").and_then(Value::as_str) else {
            continue;
        };
        if typ != "inline" && typ != "subagent" {
            continue;
        }
        idx += 1;
        let done = v.get("done").and_then(Value::as_bool).unwrap_or(false)
            || matches!(
                v.get("status").and_then(Value::as_str),
                Some("done" | "completed" | "success")
            );
        let kind = if typ == "inline" {
            "inline"
        } else {
            "subagent"
        };
        lines.push(TaskLine {
            text: format!("{idx} [{kind}] {desc} ({id})"),
            done,
        });
    }

    if lines.is_empty() { None } else { Some(lines) }
}

/// Animated status line: "思考中" + task progress bar + task file content.
/// All data comes from daemon via broadcast messages (no direct file access).
#[component]
pub fn StatusLine(mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let is_processing = hooks.use_state(|| false);
    let elapsed = hooks.use_state(|| 0u64);
    let tick = hooks.use_state(|| 0u64);
    let task_summary = hooks.use_state(|| Option::<(usize, usize)>::None);
    let task_content = hooks.use_state(|| Option::<String>::None);

    let app_ctx = hooks.use_context::<AppContext>();
    let ui_sender = app_ctx.ui_sender.clone();

    // Subscribe to events from daemon (via internal broadcast).
    let mut is_proc = is_processing;
    let mut task_sum = task_summary;
    let mut task_content_ref = task_content;
    hooks.use_future(async move {
        let mut rx = ui_sender.subscribe();
        while let Ok(msg) = rx.recv().await {
            match msg {
                AppMessage::AgentTaskStarted => {
                    is_proc.set(true);
                }
                AppMessage::AgentCompleted | AppMessage::AgentError(_) => {
                    is_proc.set(false);
                }
                AppMessage::TasksUpdated { done, total } => {
                    task_sum.set(Some((done, total)));
                }
                AppMessage::TaskContent(content) => {
                    task_content_ref.set(Some(content));
                }
                _ => {}
            }
        }
    });

    // Tick timer for elapsed seconds and spinner animation.
    let mut tick_clone = tick;
    let mut elapsed_clone = elapsed;
    let is_proc_timer = is_processing;
    hooks.use_future(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            tick_clone += 1;
            if *is_proc_timer.read() {
                elapsed_clone += 1;
            } else {
                elapsed_clone.set(0);
            }
        }
    });

    let has_tasks = task_summary.read().is_some();
    let is_proc = *is_processing.read();

    if !is_proc && !has_tasks {
        return element! { View {} };
    }

    let spinners = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let idx = (*tick.read() % spinners.len() as u64) as usize;

    // Build task progress text.
    let task_text = match *task_summary.read() {
        Some((done, total)) => {
            let bar_len = 10usize;
            let filled = (done * bar_len).checked_div(total).unwrap_or(0);
            let bar: String = "█".repeat(filled) + &"░".repeat(bar_len - filled);
            format!(" ┃ 📋 {done}/{total} [{bar}]")
        }
        None => String::new(),
    };

    let task_body = task_content.read().clone();
    let body_trimmed = task_body
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(body) = body_trimmed {
        let parsed_lines = parse_taskgraph_lines(&body);
        let task_line_views: Vec<AnyElement<'static>> = if let Some(lines) = parsed_lines {
            lines
                .into_iter()
                .map(|line| {
                    let color = if line.done { Color::Grey } else { Color::Red };
                    element! {
                        View(padding_left: 1) {
                            Text(content: line.text, color: color, align: TextAlign::Left)
                        }
                    }
                    .into_any()
                })
                .collect()
        } else {
            vec![
                element! {
                    View(padding_left: 1) {
                        Text(content: body.clone(), color: Color::Grey, align: TextAlign::Left)
                    }
                }
                .into_any(),
            ]
        };

        if is_proc {
            let secs = *elapsed.read();
            let text = format!(
                "{} 思考中… ({}s · esc 中断){}",
                spinners[idx], secs, task_text
            );
            element! {
                View(margin_bottom: 1, flex_direction: FlexDirection::Column, align_items: AlignItems::Start) {
                    View(padding_left: 1) {
                        Text(content: text, color: Color::Yellow, weight: Weight::Bold)
                    }
                    View(margin_top: 1, flex_direction: FlexDirection::Column, align_items: AlignItems::Start) {
                        #(task_line_views)
                    }
                }
            }
        } else {
            let text = format!("📋 任务进度{}", task_text);
            element! {
                View(margin_bottom: 1, flex_direction: FlexDirection::Column, align_items: AlignItems::Start) {
                    View(padding_left: 1) {
                        Text(content: text, color: Color::Cyan)
                    }
                    View(margin_top: 1, flex_direction: FlexDirection::Column, align_items: AlignItems::Start) {
                        #(task_line_views)
                    }
                }
            }
        }
    } else if is_proc {
        let secs = *elapsed.read();
        let text = format!(
            "{} 思考中… ({}s · esc 中断){}",
            spinners[idx], secs, task_text
        );
        element! {
            View(margin_bottom: 1, flex_direction: FlexDirection::Column, align_items: AlignItems::Start) {
                View(padding_left: 1) {
                    Text(content: text, color: Color::Yellow, weight: Weight::Bold)
                }
            }
        }
    } else {
        let text = format!("📋 任务进度{}", task_text);
        element! {
            View(margin_bottom: 1, flex_direction: FlexDirection::Column, align_items: AlignItems::Start) {
                View(padding_left: 1) {
                    Text(content: text, color: Color::Cyan)
                }
            }
        }
    }
}
