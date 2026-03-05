use std::fs;
use std::path::{Path, PathBuf};

/// A skill scanned from `~/.claude/skills/`.
#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    /// Absolute path to the skill.md file.
    pub path: String,
}

/// Scan `~/.claude/skills/` for skill.md files and extract name + description from YAML frontmatter.
pub fn scan_skills() -> Vec<SkillInfo> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let skills_dir = home.join(".claude").join("skills");
    let entries = match fs::read_dir(&skills_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }

        let skill_md = find_skill_md(&path);
        let Some(skill_md) = skill_md else {
            continue;
        };
        let Ok(content) = fs::read_to_string(&skill_md) else {
            continue;
        };
        if let Some(mut info) = parse_frontmatter(&content) {
            info.path = skill_md.to_string_lossy().to_string();
            skills.push(info);
        }
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn find_skill_md(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        for name in ["skill.md", "SKILL.md"] {
            let candidate = path.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn parse_frontmatter(content: &str) -> Option<SkillInfo> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let rest = &content[3..];
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];

    let mut name = None;
    let mut description = None;

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("description:") {
            description = Some(val.trim().to_string());
        }
    }

    Some(SkillInfo {
        name: name?,
        description: description.unwrap_or_default(),
        path: String::new(),
    })
}

/// Format skills for inclusion in the system prompt.
pub fn format_skills_for_prompt(skills: &[SkillInfo]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\n## Available Skills\n\n");
    out.push_str(
        "The user can invoke skills by typing `/skill-name` in the input. \
         When a skill is invoked, you MUST first read the corresponding skill.md file \
         to understand the skill's full instructions before executing it.\n\n\
         If a skill produces output files, save them to the current working directory \
         by default, or to the directory specified by the user.\n\n",
    );
    for skill in skills {
        out.push_str(&format!(
            "- **{}**: {} (path: `{}`)\n",
            skill.name, skill.description, skill.path
        ));
    }
    out
}
