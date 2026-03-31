use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow, bail};
use mash_agent::{AgentTool, ToolResult};
use once_cell::sync::Lazy;
use serde_json::{Value, json};
use similar::TextDiff;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;

const DEFAULT_MAX_LINES: usize = 200;
const DEFAULT_MAX_BYTES: usize = 100 * 1024;

static FILE_MUTATION_LOCKS: Lazy<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

async fn with_file_mutation_queue<F, T>(path: &Path, f: F) -> anyhow::Result<T>
where
    F: FnOnce() -> anyhow::Result<T>,
{
    let lock = {
        let mut map = FILE_MUTATION_LOCKS.lock().await;
        map.entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().await;
    f()
}

fn resolve_path(cwd: &Path, input_path: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(input_path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(cwd.join(path))
    }
}

fn required_string<'a>(input: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing required field `{key}`"))
}

fn optional_positive_usize(input: &Value, key: &str) -> anyhow::Result<Option<usize>> {
    match input.get(key) {
        None => Ok(None),
        Some(v) => {
            let n = v
                .as_u64()
                .ok_or_else(|| anyhow!("`{key}` must be a positive integer"))?;
            if n == 0 {
                bail!("`{key}` must be >= 1");
            }
            Ok(Some(usize::try_from(n).context("value out of range")?))
        }
    }
}

fn temp_log_path(prefix: &str) -> PathBuf {
    let mut tmp = std::env::temp_dir();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    tmp.push(format!("{prefix}-{}-{now}.log", std::process::id()));
    tmp
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TruncatedBy {
    Lines,
    Bytes,
}

#[derive(Debug, Clone)]
struct TruncationResult {
    content: String,
    truncated: bool,
    truncated_by: Option<TruncatedBy>,
    total_lines: usize,
    output_lines: usize,
    output_bytes: usize,
    first_line_exceeds_limit: bool,
    last_line_partial: bool,
}

fn truncate_head(text: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let lines: Vec<&str> = text.split('\n').collect();
    let total_lines = lines.len();
    if lines.first().map(|l| l.len()).unwrap_or(0) > max_bytes {
        return TruncationResult {
            content: String::new(),
            truncated: true,
            truncated_by: Some(TruncatedBy::Bytes),
            total_lines,
            output_lines: 0,
            output_bytes: 0,
            first_line_exceeds_limit: true,
            last_line_partial: false,
        };
    }

    let mut out = String::new();
    let mut output_lines = 0usize;
    let mut truncated_by = None;
    for (i, line) in lines.iter().enumerate() {
        if i >= max_lines {
            truncated_by = Some(TruncatedBy::Lines);
            break;
        }
        if i > 0 {
            if out.len() + 1 + line.len() > max_bytes {
                truncated_by = Some(TruncatedBy::Bytes);
                break;
            }
            out.push('\n');
        } else if line.len() > max_bytes {
            truncated_by = Some(TruncatedBy::Bytes);
            break;
        }
        out.push_str(line);
        output_lines += 1;
    }

    TruncationResult {
        content: out.clone(),
        truncated: output_lines < total_lines,
        truncated_by,
        total_lines,
        output_lines,
        output_bytes: out.len(),
        first_line_exceeds_limit: false,
        last_line_partial: false,
    }
}

fn truncate_tail(text: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let lines: Vec<&str> = text.split('\n').collect();
    let total_lines = lines.len();
    let start_line = total_lines.saturating_sub(max_lines);
    let mut selected = lines[start_line..].join("\n");
    let mut truncated_by = if start_line > 0 {
        Some(TruncatedBy::Lines)
    } else {
        None
    };
    let mut last_line_partial = false;

    if selected.len() > max_bytes {
        let start = selected.len().saturating_sub(max_bytes);
        selected = selected[start..].to_string();
        truncated_by = Some(TruncatedBy::Bytes);
        if !selected.starts_with('\n') && start > 0 {
            last_line_partial = true;
        }
    }

    let output_lines = if selected.is_empty() {
        0
    } else {
        selected.split('\n').count()
    };

    TruncationResult {
        content: selected.clone(),
        truncated: selected.len() < text.len(),
        truncated_by,
        total_lines,
        output_lines,
        output_bytes: selected.len(),
        first_line_exceeds_limit: false,
        last_line_partial,
    }
}

fn detect_line_ending(s: &str) -> &'static str {
    if s.contains("\r\n") { "\r\n" } else { "\n" }
}

fn restore_line_endings(s: &str, ending: &str) -> String {
    if ending == "\n" {
        s.to_string()
    } else {
        s.replace('\n', ending)
    }
}

fn line_of_offset(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

fn truncate_for_error(s: &str) -> String {
    const MAX: usize = 120;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(MAX).collect::<String>())
    }
}

pub struct ReadTool {
    cwd: PathBuf,
}

impl AgentTool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn definition(&self) -> Value {
        json!({
            "name": "read",
            "description": "Read file content. Supports text and images. Text is truncated to 200 lines or 100KB. Use offset/limit for large files. Images are base64 and auto-resized to 2000x2000.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to the file (relative or absolute)"},
                    "offset": {"type": "integer", "description": "Line number to start from (1-indexed)"},
                    "limit": {"type": "integer", "description": "Maximum number of lines to read"}
                },
                "required": ["path"]
            }
        })
    }

    fn execute(
        &self,
        input: &Value,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolResult> + Send + '_>> {
        let input = input.clone();
        let cwd = self.cwd.clone();
        Box::pin(async move {
            match exec_read(&input, &cwd).await {
                Ok(content) => ToolResult {
                    content,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    content: e.to_string(),
                    is_error: true,
                },
            }
        })
    }
}

async fn exec_read(input: &Value, cwd: &PathBuf) -> anyhow::Result<String> {
    let path = required_string(input, "path")?;
    if path.trim().is_empty() {
        bail!("`path` must not be empty");
    }
    let full_path = resolve_path(cwd, path)?;
    let data = std::fs::read(&full_path)
        .with_context(|| format!("failed to read file: {}", full_path.display()))?;

    if image::guess_format(&data).is_ok() {
        return Ok(format!(
            "Read image file: {} [image content omitted: current mash-ai backend does not support image inputs]",
            full_path.display()
        ));
    }

    let text = String::from_utf8_lossy(&data).to_string();
    let all_lines: Vec<&str> = text.split('\n').collect();
    let total_lines = all_lines.len();
    let start = optional_positive_usize(input, "offset")?
        .map(|v| v.saturating_sub(1))
        .unwrap_or(0);
    if start >= total_lines && total_lines > 0 {
        bail!(
            "Offset {} is beyond end of file ({} lines total)",
            start + 1,
            total_lines
        );
    }
    let limit = optional_positive_usize(input, "limit")?;

    let selected = if let Some(limit) = limit {
        let end = (start + limit).min(total_lines);
        all_lines[start..end].join("\n")
    } else {
        all_lines[start..].join("\n")
    };

    let trunc = truncate_head(&selected, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES);
    let mut output = trunc.content.clone();

    if trunc.first_line_exceeds_limit {
        let line_no = start + 1;
        let line_size = all_lines.get(start).map(|l| l.len()).unwrap_or(0);
        output = format!(
            "[Line {} is {} bytes, exceeds {} limit. Use bash: sed -n '{}p' {} | head -c {}]",
            line_no, line_size, DEFAULT_MAX_BYTES, line_no, path, DEFAULT_MAX_BYTES
        );
    } else if trunc.truncated {
        let end_line = start + trunc.output_lines;
        let next_offset = end_line + 1;
        if matches!(trunc.truncated_by, Some(TruncatedBy::Lines)) {
            output.push_str(&format!(
                "\n\n[Showing lines {}-{} of {}. Use offset={} to continue.]",
                start + 1,
                end_line,
                total_lines,
                next_offset
            ));
        } else {
            output.push_str(&format!(
                "\n\n[Showing lines {}-{} of {} ({} limit). Use offset={} to continue.]",
                start + 1,
                end_line,
                total_lines,
                DEFAULT_MAX_BYTES,
                next_offset
            ));
        }
    } else if let Some(limit) = limit {
        let consumed = start + limit;
        if consumed < total_lines {
            output.push_str(&format!(
                "\n\n[{} more lines in file. Use offset={} to continue.]",
                total_lines - consumed,
                consumed + 1
            ));
        }
    }

    Ok(output)
}

pub struct WriteTool {
    cwd: PathBuf,
}

impl AgentTool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn definition(&self) -> Value {
        json!({
            "name": "write",
            "description": "Write content to a file. Creates if missing, overwrites if exists, and creates parent directories.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to the file (relative or absolute)"},
                    "content": {"type": "string", "description": "Content to write"}
                },
                "required": ["path", "content"]
            }
        })
    }

    fn execute(
        &self,
        input: &Value,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolResult> + Send + '_>> {
        let input = input.clone();
        let cwd = self.cwd.clone();
        Box::pin(async move {
            match exec_write(&input, &cwd).await {
                Ok(content) => ToolResult {
                    content,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    content: e.to_string(),
                    is_error: true,
                },
            }
        })
    }
}

async fn exec_write(input: &Value, cwd: &PathBuf) -> anyhow::Result<String> {
    let path = required_string(input, "path")?;
    if path.trim().is_empty() {
        bail!("`path` must not be empty");
    }
    let content = required_string(input, "content")?;
    let full_path = resolve_path(cwd, path)?;

    with_file_mutation_queue(&full_path, || {
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create parent directory: {}", parent.display())
            })?;
        }
        std::fs::write(&full_path, content.as_bytes())
            .with_context(|| format!("failed to write file: {}", full_path.display()))?;
        Ok(())
    })
    .await?;

    Ok(format!(
        "Successfully wrote {} bytes to {}",
        content.len(),
        path
    ))
}

pub struct EditTool {
    cwd: PathBuf,
}

impl AgentTool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn definition(&self) -> Value {
        json!({
            "name": "edit",
            "description": "Edit a single file using exact text replacement. Each edits[].oldText must match uniquely and not overlap.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "oldText": {"type": "string"},
                                "newText": {"type": "string"}
                            },
                            "required": ["oldText", "newText"],
                            "additionalProperties": false
                        }
                    },
                    "oldText": {"type": "string"},
                    "newText": {"type": "string"}
                },
                "required": ["path", "edits"],
                "additionalProperties": false
            }
        })
    }

    fn execute(
        &self,
        input: &Value,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolResult> + Send + '_>> {
        let input = input.clone();
        let cwd = self.cwd.clone();
        Box::pin(async move {
            match exec_edit(&input, &cwd).await {
                Ok(content) => ToolResult {
                    content,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    content: e.to_string(),
                    is_error: true,
                },
            }
        })
    }
}

#[derive(Debug)]
struct PendingEdit {
    start: usize,
    end: usize,
    new_text: String,
}

fn extract_edits(input: &Value) -> anyhow::Result<Vec<(String, String)>> {
    let mut edits: Vec<(String, String)> = Vec::new();
    if let Some(arr) = input.get("edits").and_then(Value::as_array) {
        for item in arr {
            let old = item
                .get("oldText")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("each edit requires `oldText`"))?;
            let new = item
                .get("newText")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("each edit requires `newText`"))?;
            edits.push((old.to_string(), new.to_string()));
        }
    }
    if let (Some(old), Some(new)) = (
        input.get("oldText").and_then(Value::as_str),
        input.get("newText").and_then(Value::as_str),
    ) {
        edits.push((old.to_string(), new.to_string()));
    }
    if edits.is_empty() {
        bail!("edits must contain at least one replacement");
    }
    Ok(edits)
}

async fn exec_edit(input: &Value, cwd: &PathBuf) -> anyhow::Result<String> {
    let path = required_string(input, "path")?;
    if path.trim().is_empty() {
        bail!("`path` must not be empty");
    }
    let edits = extract_edits(input)?;
    let full_path = resolve_path(cwd, path)?;

    with_file_mutation_queue(&full_path, || {
        let raw = std::fs::read_to_string(&full_path)
            .with_context(|| format!("failed to read file: {}", full_path.display()))?;

        let (bom, content) = if let Some(rest) = raw.strip_prefix('\u{feff}') {
            ("\u{feff}", rest.to_string())
        } else {
            ("", raw.clone())
        };
        let original_line_ending = detect_line_ending(&content);
        let normalized = content.replace("\r\n", "\n");

        let mut pending = Vec::new();
        for (old_text, new_text) in edits {
            let old_norm = old_text.replace("\r\n", "\n");
            let new_norm = new_text.replace("\r\n", "\n");
            let matches: Vec<usize> = normalized
                .match_indices(&old_norm)
                .map(|(idx, _)| idx)
                .collect();
            if matches.len() != 1 {
                bail!(
                    "`oldText` must match exactly once, found {} matches for snippet: {:?}",
                    matches.len(),
                    truncate_for_error(&old_text)
                );
            }
            let start = matches[0];
            let end = start + old_norm.len();
            pending.push(PendingEdit {
                start,
                end,
                new_text: new_norm,
            });
        }

        pending.sort_by_key(|e| e.start);
        for w in pending.windows(2) {
            if w[0].end > w[1].start {
                bail!("edit ranges overlap, which is not allowed");
            }
        }

        let mut rebuilt = String::with_capacity(normalized.len());
        let mut cursor = 0usize;
        for e in &pending {
            rebuilt.push_str(&normalized[cursor..e.start]);
            rebuilt.push_str(&e.new_text);
            cursor = e.end;
        }
        rebuilt.push_str(&normalized[cursor..]);

        let final_content = format!(
            "{}{}",
            bom,
            restore_line_endings(&rebuilt, original_line_ending)
        );
        std::fs::write(&full_path, final_content.as_bytes())
            .with_context(|| format!("failed to write file: {}", full_path.display()))?;

        let diff = TextDiff::from_lines(&normalized, &rebuilt)
            .unified_diff()
            .header("a/file", "b/file")
            .to_string();
        let first_changed_line = pending
            .first()
            .map(|e| line_of_offset(&normalized, e.start))
            .unwrap_or(1);

        Ok(json!({
            "message": format!("Successfully replaced {} block(s) in {}.", pending.len(), path),
            "diff": diff,
            "firstChangedLine": first_changed_line
        })
        .to_string())
    })
    .await
}

pub struct BashTool {
    cwd: PathBuf,
}

impl AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn definition(&self) -> Value {
        json!({
            "name": "bash",
            "description": "Execute a bash command in the current working directory. Output is truncated to last 200 lines or 100KB; full output is saved to a temp file when truncated.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Bash command to execute"},
                    "timeout": {"type": "integer", "description": "Timeout in seconds (optional, no default timeout)"}
                },
                "required": ["command"]
            }
        })
    }

    fn execute(
        &self,
        input: &Value,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolResult> + Send + '_>> {
        let input = input.clone();
        let cwd = self.cwd.clone();
        Box::pin(async move {
            match exec_bash(&input, &cwd).await {
                Ok(content) => ToolResult {
                    content,
                    is_error: false,
                },
                Err(e) => ToolResult {
                    content: e.to_string(),
                    is_error: true,
                },
            }
        })
    }
}

async fn exec_bash(input: &Value, cwd: &PathBuf) -> anyhow::Result<String> {
    let command = required_string(input, "command")?;
    if command.trim().is_empty() {
        bail!("`command` must not be empty");
    }
    let timeout_secs = input.get("timeout").and_then(Value::as_u64);

    let child = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .output();

    let output = if let Some(secs) = timeout_secs {
        timeout(Duration::from_secs(secs), child)
            .await
            .map_err(|_| anyhow!("Command timed out after {} seconds", secs))?
            .context("failed to execute bash command")?
    } else {
        child.await.context("failed to execute bash command")?
    };

    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    let trunc = truncate_tail(&combined, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES);
    let mut payload = if trunc.content.is_empty() {
        "(no output)".to_string()
    } else {
        trunc.content.clone()
    };

    if trunc.truncated {
        let log_path = temp_log_path("mashd-bash-output");
        std::fs::write(&log_path, combined.as_bytes())
            .with_context(|| format!("failed to write temp log: {}", log_path.display()))?;
        let start_line = trunc
            .total_lines
            .saturating_sub(trunc.output_lines)
            .saturating_add(1);
        let end_line = trunc.total_lines;
        if trunc.last_line_partial {
            payload.push_str(&format!(
                "\n\n[Showing last {} bytes of line {}. Full output: {}]",
                trunc.output_bytes,
                end_line,
                log_path.display()
            ));
        } else if matches!(trunc.truncated_by, Some(TruncatedBy::Lines)) {
            payload.push_str(&format!(
                "\n\n[Showing lines {}-{} of {}. Full output: {}]",
                start_line,
                end_line,
                trunc.total_lines,
                log_path.display()
            ));
        } else {
            payload.push_str(&format!(
                "\n\n[Showing lines {}-{} of {} ({} limit). Full output: {}]",
                start_line,
                end_line,
                trunc.total_lines,
                DEFAULT_MAX_BYTES,
                log_path.display()
            ));
        }
    }

    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        bail!("{}\n\nCommand exited with code {}", payload, code);
    }

    Ok(payload)
}

pub fn create_tools(cwd: PathBuf) -> Vec<Box<dyn AgentTool>> {
    vec![
        Box::new(ReadTool { cwd: cwd.clone() }),
        Box::new(BashTool { cwd: cwd.clone() }),
        Box::new(EditTool { cwd: cwd.clone() }),
        Box::new(WriteTool { cwd }),
    ]
}
