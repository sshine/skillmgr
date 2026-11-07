//! SKILL.md frontmatter parsing and name validation.

use std::path::Path;

use crate::error::{Error, Result};

/// The subset of a skill's frontmatter that skillmgr needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillMeta {
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
}

/// Read and parse `SKILL.md` from a skill directory.
pub fn read_skill_meta(skill_dir: &Path) -> Result<SkillMeta> {
    let path = skill_dir.join("SKILL.md");
    let content = std::fs::read_to_string(&path)
        .map_err(|_| Error::NotASkill(skill_dir.display().to_string()))?;
    parse_meta(&content)
}

/// Read `SKILL.md` from a skill directory and require the file to exist.
pub fn read_skill_file(skill_dir: &Path) -> Result<String> {
    let path = skill_dir.join("SKILL.md");
    std::fs::read_to_string(&path).map_err(|_| Error::NotASkill(skill_dir.display().to_string()))
}

/// Parse the YAML frontmatter at the top of a SKILL.md.
pub fn parse_meta(content: &str) -> Result<SkillMeta> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Ok(SkillMeta::default());
    }

    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        return Err(Error::Yaml(
            "frontmatter is not closed with ---".to_string(),
        ));
    }

    let value: serde_yaml::Value =
        serde_yaml::from_str(&yaml).map_err(|e| Error::Yaml(e.to_string()))?;

    let name = string_field(&value, "name");
    let description = string_field(&value, "description");
    let version = value
        .get("metadata")
        .and_then(|m| m.get("version"))
        .and_then(|v| match v {
            serde_yaml::Value::String(s) => Some(s.clone()),
            serde_yaml::Value::Number(n) => Some(n.to_string()),
            _ => None,
        });

    Ok(SkillMeta {
        name,
        description,
        version,
    })
}

fn string_field(value: &serde_yaml::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Enforce the lowercase, hyphenated skill naming convention.
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(Error::Invalid(format!("invalid skill name: {name}")));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(Error::Invalid(format!("invalid skill name: {name}")));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(Error::Invalid(format!(
            "skill name must be lowercase letters, digits and hyphens: {name}"
        )));
    }
    Ok(())
}

/// A minimal but valid SKILL.md scaffold.
pub fn scaffold(name: &str, description: &str) -> String {
    let description = if description.trim().is_empty() {
        "TODO: describe what this skill does and when an agent should use it."
    } else {
        description.trim()
    };
    format!(
        "---\nname: {name}\ndescription: >-\n  {description}\n---\n\n# {name}\n\nTODO: write the skill instructions.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_folded_description_and_version() {
        let meta = parse_meta(
            "---\nname: blog-post\ndescription: >-\n  Write a post.\nmetadata:\n  version: \"1.0\"\n---\n# Body\n",
        )
        .expect("meta");
        assert_eq!(meta.name.as_deref(), Some("blog-post"));
        assert_eq!(meta.description.as_deref(), Some("Write a post."));
        assert_eq!(meta.version.as_deref(), Some("1.0"));
    }

    #[test]
    fn missing_frontmatter_is_empty() {
        assert_eq!(
            parse_meta("# No frontmatter\n").expect("meta"),
            SkillMeta::default()
        );
    }

    #[test]
    fn validates_names() {
        validate_name("blog-post").expect("valid");
        assert!(validate_name("Blog_Post").is_err());
        assert!(validate_name("-blog").is_err());
    }
}
