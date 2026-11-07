//! Resolved filesystem paths for a skillmgr installation.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The managed home, the active skills directory, and the disposable origin cache.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
    dest: PathBuf,
    cache: PathBuf,
}

impl Paths {
    /// Resolve paths from optional CLI overrides, falling back to the defaults.
    pub fn resolve(
        root: Option<PathBuf>,
        dest: Option<PathBuf>,
        cache: Option<PathBuf>,
    ) -> Result<Self> {
        let home = home_dir()?;

        let root = root.unwrap_or_else(|| home.join(".agents/managed-skills"));
        let dest = dest.unwrap_or_else(|| home.join(".agents/skills"));
        let cache = cache.unwrap_or_else(|| {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".cache"))
                .join("skillmgr/origins")
        });

        Ok(Self {
            root: absolute(root)?,
            dest: absolute(dest)?,
            cache: absolute(cache)?,
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
    }
}
