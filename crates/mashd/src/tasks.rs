use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use serde_json::Value;

/// Build the task-list section of the system prompt.
pub fn format_task_prompt(task_dir: &Path) -> String {
    let task_dir_str = task_dir.display().to_string();
    include_str!("prompt/task_protocol.md").replace("{task_dir}", &task_dir_str)
}

/// Parse task lines from assistant text.
pub fn parse_tasks(text: &str) -> Option<Vec<String>> {
    let start = text.find("<!-- TASKS")?;
    let end = text.find("TASKS -->")?;
    if end <= start {
        return None;
    }

    let block = &text[start + "<!-- TASKS".len()..end];
    let lines: Vec<String> = block
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("- ["))
        .map(|l| l.to_string())
        .collect();

    if lines.is_empty() { None } else { Some(lines) }
}

fn project_name() -> String {
    project_name_from(
        &std::env::current_dir()
            .ok()
            .unwrap_or_else(|| PathBuf::from(".")),
    )
}

fn project_name_from(cwd: &Path) -> String {
    cwd.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn tasks_root_dir() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    let dir = home.join(".mash").join("tasks");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Resolve per-project task directory: ~/.mash/tasks/<project>
pub fn project_tasks_dir(cwd: &Path) -> Result<PathBuf> {
    let dir = tasks_root_dir()?.join(project_name_from(cwd));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Write the current task list to the task file.
pub fn write_tasks(path: &Path, lines: &[String]) -> Result<()> {
    let mut content = format!("# Tasks — {}\n\n", project_name());
    for line in lines {
        content.push_str(line);
        content.push('\n');
    }
    fs::write(path, content)?;
    Ok(())
}

/// Read full task file content for display.
pub fn read_task_content(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// Return the latest modified regular file in a task directory.
pub fn latest_task_file(task_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(task_dir).ok()?;
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((path, modified))
        })
        .max_by_key(|(_, modified)| *modified)
        .map(|(path, _)| path)
}

/// Read latest task file content from task directory.
pub fn read_latest_task_content(task_dir: &Path) -> Option<(PathBuf, String)> {
    let file = latest_task_file(task_dir)?;
    let content = fs::read_to_string(&file).ok()?;
    Some((file, content))
}

/// Read task summary from a task file: (completed, total).
pub fn read_task_summary(path: &Path) -> Option<(usize, usize)> {
    let content = fs::read_to_string(path).ok()?;
    if let Some(summary) = read_taskgraph_summary_from_content(&content) {
        return Some(summary);
    }

    // Fallback: markdown checklist format.
    let total = content
        .lines()
        .filter(|l| l.trim().starts_with("- ["))
        .count();
    if total == 0 {
        return None;
    }
    let done = content
        .lines()
        .filter(|l| l.trim().starts_with("- [x]"))
        .count();
    Some((done, total))
}

fn read_taskgraph_summary_from_content(content: &str) -> Option<(usize, usize)> {
    let mut total = 0usize;
    let mut done = 0usize;

    for line in content.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if !v.is_object() {
            continue;
        }
        let has_required_fields = v.get("id").and_then(Value::as_str).is_some()
            && v.get("desc").and_then(Value::as_str).is_some()
            && v.get("type").and_then(Value::as_str).is_some();
        if !has_required_fields {
            continue;
        }

        total += 1;
        let is_done = v.get("done").and_then(Value::as_bool).unwrap_or(false)
            || matches!(
                v.get("status").and_then(Value::as_str),
                Some("done" | "completed" | "success")
            );
        if is_done {
            done += 1;
        }
    }

    if total > 0 { Some((done, total)) } else { None }
}

/// Summary for a task directory based on its latest file.
pub fn read_task_summary_from_dir(task_dir: &Path) -> Option<(usize, usize)> {
    let file = latest_task_file(task_dir)?;
    read_task_summary(&file)
}

/// Metadata signature for change detection.
pub fn task_file_signature(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let len = meta.len();
    Some((modified, len))
}
