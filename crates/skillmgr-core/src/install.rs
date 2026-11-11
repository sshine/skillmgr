//! Symlinking managed skills into the active skills directory.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::registry::Registry;

/// What currently occupies the active directory entry for a skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    Linked,
    Unlinked,
    Broken,
    ForeignDir,
    ForeignSymlink,
}

/// One conflict that prevented a link from being installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub name: String,
    pub state: LinkState,
    pub detail: String,
}

/// The result of an install or uninstall pass.
#[derive(Debug, Clone, Default)]
pub struct InstallReport {
    pub linked: Vec<String>,
    pub already: Vec<String>,
    pub pruned: Vec<String>,
    pub removed: Vec<String>,
    pub conflicts: Vec<Conflict>,
}

impl InstallReport {
    pub fn is_ok(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Link every selected registered skill into the active directory.
pub fn install(
    paths: &Paths,
    registry: &Registry,
    names: &[String],
    dry_run: bool,
    prune: bool,
) -> Result<InstallReport> {
    std::fs::create_dir_all(paths.dest())?;
    // Repository declarations that have not resolved into skills have no
    // directory to link. Skip them instead of reporting a conflict.
    let selected: Vec<_> = registry
        .select(names)?
        .into_iter()
        .filter(|entry| entry.is_skill())
        .collect();
    let desired: BTreeSet<String> = selected.iter().map(|e| e.name.clone()).collect();

    let mut report = InstallReport::default();

    for entry in selected {
        let target = paths.skill_dir(&entry.source, &entry.name);
        if !target.is_dir() {
            report.conflicts.push(Conflict {
                name: entry.name.clone(),
                state: LinkState::Unlinked,
                detail: format!("missing managed directory: {}", target.display()),
            });
            continue;
        }

        let link = paths.dest().join(&entry.name);
        match inspect(paths, &link, &target)? {
            LinkState::Linked => report.already.push(entry.name.clone()),
            LinkState::Unlinked => {
                if !dry_run {
                    create_link(&target, &link)?;
                }
                report.linked.push(entry.name.clone());
            }
            LinkState::Broken => {
                if !dry_run {
                    remove_link(&link)?;
                    create_link(&target, &link)?;
                }
                report.linked.push(entry.name.clone());
            }
            state @ (LinkState::ForeignDir | LinkState::ForeignSymlink) => {
                report.conflicts.push(Conflict {
                    name: entry.name.clone(),
                    state,
                    detail: format!(
                        "{} is not a link into {}; move it with `skillmgr adopt` or remove it",
                        link.display(),
                        paths.skills_dir().display()
                    ),
                });
            }
        }
    }

    if prune {
        for link in managed_links(paths)? {
            let name = link
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !desired.contains(&name) {
                if !dry_run {
                    remove_link(&link)?;
                }
                report.pruned.push(name);
            }
        }
    }

    Ok(report)
}

/// Remove managed links, either by name or every managed link.
pub fn uninstall(
    paths: &Paths,
    registry: &Registry,
    names: &[String],
    all: bool,
    dry_run: bool,
) -> Result<InstallReport> {
    let mut report = InstallReport::default();
    if !paths.dest().exists() {
        return Ok(report);
    }

    let targets: Vec<(String, PathBuf)> = if all {
        managed_links(paths)?
            .into_iter()
            .filter_map(|path| {
                let name = path.file_name().map(|s| s.to_string_lossy().into_owned())?;
                Some((name, path))
            })
            .collect()
    } else {
        if names.is_empty() {
            return Err(Error::Invalid(
                "uninstall requires skill names or --all".to_string(),
            ));
        }
        names
            .iter()
            .map(|name| (name.clone(), paths.dest().join(name)))
            .collect()
    };

    for (name, link) in targets {
        let Some(entry) = registry.get(&name) else {
            if link.symlink_metadata().is_ok() {
                report.conflicts.push(Conflict {
                    name: name.clone(),
                    state: LinkState::ForeignSymlink,
                    detail: format!("unknown skill '{name}'"),
                });
            }
            continue;
        };
        let target = paths.skill_dir(&entry.source, &entry.name);
        match inspect(paths, &link, &target)? {
            LinkState::Linked | LinkState::Broken => {
                if !dry_run {
                    remove_link(&link)?;
                }
                report.removed.push(name);
            }
            LinkState::Unlinked => {}
            state @ (LinkState::ForeignDir | LinkState::ForeignSymlink) => {
                report.conflicts.push(Conflict {
                    name,
                    state,
                    detail: format!("not a managed link: {}", link.display()),
                });
            }
        }
    }

    Ok(report)
}

/// Inspect what currently occupies a link path.
pub fn inspect(paths: &Paths, link: &Path, target: &Path) -> Result<LinkState> {
    let metadata = match link.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LinkState::Unlinked),
        Err(e) => return Err(e.into()),
    };

    if !metadata.file_type().is_symlink() {
        return Ok(LinkState::ForeignDir);
    }

    let current = std::fs::read_link(link)?;
    let resolved = if current.is_absolute() {
        current
    } else {
        link.parent().unwrap_or(Path::new(".")).join(current)
    };

    if same_path(&resolved, target) {
        return if target.is_dir() {
            Ok(LinkState::Linked)
        } else {
            Ok(LinkState::Broken)
        };
    }

    if resolved.starts_with(paths.skills_dir()) {
        Ok(LinkState::Broken)
    } else {
        Ok(LinkState::ForeignSymlink)
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(unix)]
fn create_link(target: &Path, link: &Path) -> Result<()> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::os::unix::fs::symlink(target, link)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_link(_target: &Path, _link: &Path) -> Result<()> {
    Err(Error::Invalid(
        "symlink installation is only supported on Unix".to_string(),
    ))
}

fn remove_link(link: &Path) -> Result<()> {
    match std::fs::remove_file(link) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn managed_links(paths: &Paths) -> Result<Vec<PathBuf>> {
    let mut links = Vec::new();
    if !paths.dest().exists() {
        return Ok(links);
    }
    for entry in std::fs::read_dir(paths.dest())? {
        let entry = entry?;
        let path = entry.path();
        let metadata = path.symlink_metadata()?;
        if !metadata.file_type().is_symlink() {
            continue;
        }
        let Ok(target) = std::fs::read_link(&path) else {
            continue;
        };
        let resolved = if target.is_absolute() {
            target
        } else {
            path.parent().unwrap_or(Path::new(".")).join(target)
        };
        if resolved.starts_with(paths.skills_dir()) {
            links.push(path);
        }
    }
    links.sort();
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_symlink_is_a_foreign_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::resolve(
            Some(dir.path().join("root")),
            Some(dir.path().join("dest")),
            Some(dir.path().join("cache")),
        )
        .expect("paths");
        std::fs::create_dir_all(paths.dest().join("foo")).expect("dest");
        let state = inspect(
            &paths,
            &paths.dest().join("foo"),
            &paths.skill_dir("authored", "foo"),
        )
        .expect("inspect");
        assert_eq!(state, LinkState::ForeignDir);
    }
}
