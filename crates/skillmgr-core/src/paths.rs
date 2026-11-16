//! Resolved filesystem paths for a skillmgr installation.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The managed home, the active skills directory, the disposable origin cache,
/// and the directory for wizard state.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
    dest: PathBuf,
    cache: PathBuf,
    state: PathBuf,
}

impl Paths {
    /// Resolve paths from optional CLI overrides, then environment variables,
    /// falling back to the defaults.
    ///
    /// `SKILLMGR_ROOT`, `SKILLMGR_DEST`, `SKILLMGR_CACHE_DIR` and
    /// `SKILLMGR_STATE_DIR` name each path independently. This makes the binary
    /// fully redirectable from the environment, which tests rely on, without
    /// having to fake `HOME`.
    pub fn resolve(
        root: Option<PathBuf>,
        dest: Option<PathBuf>,
        cache: Option<PathBuf>,
        state: Option<PathBuf>,
    ) -> Result<Self> {
        let home = home_dir()?;

        let root = root
            .or_else(|| env_path("SKILLMGR_ROOT"))
            .unwrap_or_else(|| home.join(".agents/managed-skills"));
        let dest = dest
            .or_else(|| env_path("SKILLMGR_DEST"))
            .unwrap_or_else(|| home.join(".agents/skills"));
        let cache = cache
            .or_else(|| env_path("SKILLMGR_CACHE_DIR"))
            .unwrap_or_else(|| {
                env_path("XDG_CACHE_HOME")
                    .unwrap_or_else(|| home.join(".cache"))
                    .join("skillmgr/origins")
            });
        let state = state
            .or_else(|| env_path("SKILLMGR_STATE_DIR"))
            .unwrap_or_else(|| {
                env_path("XDG_STATE_HOME")
                    .unwrap_or_else(|| home.join(".local/state"))
                    .join("skillmgr")
            });

        Ok(Self {
            root: absolute(root)?,
            dest: absolute(dest)?,
            cache: absolute(cache)?,
            state: absolute(state)?,
        })
    }

    /// The managed skill directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The active skills directory that agents read.
    pub fn dest(&self) -> &Path {
        &self.dest
    }

    /// The origin cache root.
    pub fn cache(&self) -> &Path {
        &self.cache
    }

    /// The directory containing wizard state.
    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    /// `state.toml`, which remembers answers to optional setup questions.
    pub fn state(&self) -> PathBuf {
        self.state.join("state.toml")
    }

    /// `skills.toml` inside the managed home.
    pub fn registry(&self) -> PathBuf {
        self.root.join("skills.toml")
    }

    /// `skills/` inside the managed home.
    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// `skills/vendored/`.
    pub fn vendored_dir(&self) -> PathBuf {
        self.skills_dir().join("vendored")
    }

    /// `skills/authored/`.
    pub fn authored_dir(&self) -> PathBuf {
        self.skills_dir().join("authored")
    }

    /// A source directory such as `skills/vendored/`.
    pub fn source_dir(&self, source: &str) -> PathBuf {
        self.skills_dir().join(source)
    }

    /// The directory for one skill.
    pub fn skill_dir(&self, source: &str, name: &str) -> PathBuf {
        self.source_dir(source).join(name)
    }

    /// Create the managed home and its two conventional source directories.
    pub fn ensure_layout(&self) -> Result<()> {
        std::fs::create_dir_all(self.vendored_dir())?;
        std::fs::create_dir_all(self.authored_dir())?;
        std::fs::create_dir_all(&self.dest)?;
        Ok(())
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(Error::NoHome)
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn absolute(path: PathBuf) -> Result<PathBuf> {
    let expanded = expand_tilde(path)?;
    if expanded.is_absolute() {
        Ok(expanded)
    } else {
        Ok(std::env::current_dir()?.join(expanded))
    }
}

fn expand_tilde(path: PathBuf) -> Result<PathBuf> {
    let Some(text) = path.to_str() else {
        return Ok(path);
    };
    if text == "~" || text.starts_with("~/") {
        let home = home_dir()?;
        let rest = text.strip_prefix("~/").unwrap_or("");
        return Ok(home.join(rest));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_under_root() {
        let paths = Paths::resolve(
            Some(PathBuf::from("/tmp/skillmgr-root")),
            Some(PathBuf::from("/tmp/skillmgr-dest")),
            Some(PathBuf::from("/tmp/skillmgr-cache")),
            Some(PathBuf::from("/tmp/skillmgr-state")),
        )
        .expect("paths");
        assert_eq!(
            paths.registry(),
            PathBuf::from("/tmp/skillmgr-root/skills.toml")
        );
        assert_eq!(
            paths.skill_dir("authored", "blog-post"),
            PathBuf::from("/tmp/skillmgr-root/skills/authored/blog-post")
        );
        assert_eq!(
            paths.state(),
            PathBuf::from("/tmp/skillmgr-state/state.toml")
        );
    }
}
