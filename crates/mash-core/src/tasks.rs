use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

/// Build the task-list section of the system prompt.
pub fn format_task_prompt(task_file_path: &Path) -> String {
    let path_str = task_file_path.display().to_string();
    let perl_cmd = format!(
        r#"perl -i -pe 's/^- \\[ \\] N\\./- [x] N./' "{}""#,
        path_str
    );
    format!(
        include_str!("prompt/task_protocol.md"),
        path_str = path_str,
        perl_cmd = perl_cmd
    )
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
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

fn tasks_dir() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    let dir = home.join(".mash").join("tasks");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Initialize a new task file for this session, returning its path.
pub fn init_task_file() -> Result<PathBuf> {
    let dir = tasks_dir()?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = format!("{}_{}.md", project_name(), ts);
    let path = dir.join(name);
    fs::write(&path, format!("# Tasks — {}\n\n", project_name()))?;
    Ok(path)
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

/// Read task summary from a task file: (completed, total).
pub fn read_task_summary(path: &Path) -> Option<(usize, usize)> {
    let content = fs::read_to_string(path).ok()?;
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
