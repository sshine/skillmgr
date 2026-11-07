//! Inventory and upstream status for the CLI and TUI.

use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::git::{Git, Upstream};
use crate::install::{LinkState, inspect};
use crate::paths::Paths;
use crate::registry::{Entry, Origin, Registry};
use crate::repo::Repo;
use crate::skill::read_skill_meta;

/// A skill as it exists locally, with optional upstream information.
#[derive(Debug, Clone)]
pub struct SkillStatus {
    pub name: String,
    pub source: String,
    pub origin: Origin,
    pub registered: bool,
    pub link: LinkState,
    pub managed_path: PathBuf,
    pub description: Option<String>,
    pub version: Option<String>,
    pub upstream: Option<Upstream>,
    pub note: Option<String>,
}

impl SkillStatus {
    pub fn is_vendored(&self) -> bool {
        self.origin.is_vendored()
    }

    pub fn is_outdated(&self) -> bool {
        match &self.upstream {
            Some(Upstream::Behind { repo, path }) => *repo > 0 || *path > 0,
            Some(Upstream::Diverged) | Some(Upstream::PathRemoved { .. }) => true,
            _ => false,
        }
    }

    pub fn behind_label(&self) -> String {
        match &self.upstream {
            Some(Upstream::Current) => "0/0".to_string(),
            Some(Upstream::Behind { repo, path }) => format!("{repo}/{path}"),
            Some(Upstream::Diverged) => "diverged".to_string(),
            Some(Upstream::PathRemoved { repo }) => format!("{repo}/gone"),
            Some(Upstream::Unreachable) => "unreachable".to_string(),
            Some(Upstream::Unknown(_)) => "?".to_string(),
            None => "-".to_string(),
        }
    }

    pub fn repo_field(&self) -> Option<(&str, &str, &str, &str)> {
        match &self.origin {
            Origin::Vendored {
                repo,
                path,
                branch,
                rev,
            } => Some((repo, path, branch, rev)),
            Origin::Authored => None,
        }
    }
}

/// Build the local inventory, including unregistered and missing directories.
pub fn local(paths: &Paths, registry: &Registry) -> Result<Vec<SkillStatus>> {
    let mut statuses = Vec::new();

    for entry in registry.entries() {
        statuses.push(entry_status(paths, entry)?);
    }

    for (source, name, path) in scan_sources(paths)? {
        if let Some(entry) = registry.get(&name)
            && entry.source == source
        {
            continue;
        }
        let meta = read_skill_meta(&path).ok();
        let link = inspect(
            paths,
            &paths.dest().join(&name),
            &paths.skill_dir(&source, &name),
        )?;
        statuses.push(SkillStatus {
            name,
            source,
            origin: Origin::Authored,
            registered: false,
            link,
            managed_path: path,
            description: meta.as_ref().and_then(|m| m.description.clone()),
            version: meta.as_ref().and_then(|m| m.version.clone()),
            upstream: None,
            note: Some("unregistered".to_string()),
        });
    }

    statuses.sort_by(|a, b| a.name.cmp(&b.name).then(a.source.cmp(&b.source)));
    Ok(statuses)
}

fn entry_status(paths: &Paths, entry: &Entry) -> Result<SkillStatus> {
    let managed_path = paths.skill_dir(&entry.source, &entry.name);
    let link = inspect(paths, &paths.dest().join(&entry.name), &managed_path)?;
    let meta = read_skill_meta(&managed_path).ok();
    let note = if !managed_path.is_dir() {
        Some("missing managed directory".to_string())
    } else {
        None
    };

    Ok(SkillStatus {
        name: entry.name.clone(),
        source: entry.source.clone(),
        origin: entry.origin.clone(),
        registered: true,
        link,
        managed_path,
        description: meta.as_ref().and_then(|m| m.description.clone()),
        version: meta.as_ref().and_then(|m| m.version.clone()),
        upstream: None,
        note,
    })
}

/// Every `<source>/<name>/SKILL.md` directory below `skills/`.
pub fn scan_sources(paths: &Paths) -> Result<Vec<(String, String, PathBuf)>> {
    let mut found = Vec::new();
    let skills = paths.skills_dir();
    if !skills.exists() {
        return Ok(found);
    }

    for source_entry in std::fs::read_dir(&skills)? {
        let source_entry = source_entry?;
        if !source_entry.file_type()?.is_dir() {
            continue;
        }
        let source = source_entry.file_name().to_string_lossy().into_owned();
        for skill_entry in std::fs::read_dir(source_entry.path())? {
            let skill_entry = skill_entry?;
            if !skill_entry.file_type()?.is_dir() {
                continue;
            }
            let name = skill_entry.file_name().to_string_lossy().into_owned();
            let path = skill_entry.path();
            if path.join("SKILL.md").is_file() {
                found.push((source.clone(), name, path));
            }
        }
    }
    Ok(found)
}

/// Fill in upstream state for every vendored status.
pub fn check_upstream(git: &Git, statuses: &mut [SkillStatus]) {
    for status in statuses.iter_mut() {
        let Some((repo_text, path, branch, rev)) =
            status.repo_field().map(|(repo, path, branch, rev)| {
                (
                    repo.to_string(),
                    path.to_string(),
                    branch.to_string(),
                    rev.to_string(),
                )
            })
        else {
            continue;
        };
        let repo = match Repo::parse(&repo_text) {
            Ok(repo) => repo,
            Err(e) => {
                status.upstream = Some(Upstream::Unknown(e.to_string()));
                continue;
            }
        };
        status.upstream = Some(
            git.upstream(&repo, &rev, &branch, &path)
                .unwrap_or_else(|e| Upstream::Unknown(e.to_string())),
        );
    }
}

/// Filter statuses by name, preserving order.
pub fn filter(statuses: Vec<SkillStatus>, names: &[String]) -> Vec<SkillStatus> {
    if names.is_empty() {
        return statuses;
    }
    statuses
        .into_iter()
        .filter(|s| names.iter().any(|n| n == &s.name))
        .collect()
}

/// Fail when a requested name is neither registered nor present as a directory.
pub fn ensure_names_exist(paths: &Paths, registry: &Registry, names: &[String]) -> Result<()> {
    let known: Vec<String> = registry
        .names()
        .into_iter()
        .chain(scan_sources(paths)?.into_iter().map(|(_, name, _)| name))
        .collect();
    for name in names {
        if !known.iter().any(|k| k == name) {
            return Err(Error::NotFound(format!("skill '{name}'")));
        }
    }
    Ok(())
}
