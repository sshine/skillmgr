//! Copying an upstream skill subtree into the managed vendored directory.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::error::Result;
use crate::git::Git;
use crate::repo::Repo;

/// How a vendored directory compares to the pinned upstream snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VendoredState {
    Current,
    Changed,
    Missing,
}

/// Re-vendor one skill at a pinned revision.
///
/// Returns `Current` when the destination already matches, in which case the
/// destination is left untouched.
pub fn vendor(git: &Git, repo: &Repo, rev: &str, path: &str, dest: &Path) -> Result<VendoredState> {
    let parent = dest.parent().ok_or_else(|| {
        crate::Error::Invalid(format!("destination has no parent: {}", dest.display()))
    })?;
    std::fs::create_dir_all(parent)?;

    let temp = tempfile::Builder::new()
        .prefix(".skillmgr-vendor-")
        .tempdir_in(parent)?;
    git.archive(repo, rev, path, temp.path())?;

    if !dest.exists() {
        std::fs::rename(temp.path(), dest)?;
        return Ok(VendoredState::Missing);
    }

    if digest(temp.path())? == digest(dest)? {
        return Ok(VendoredState::Current);
    }

    std::fs::remove_dir_all(dest)?;
    std::fs::rename(temp.path(), dest)?;
    Ok(VendoredState::Changed)
}

/// Compare a vendored directory with the snapshot at `rev` without writing.
pub fn check(git: &Git, repo: &Repo, rev: &str, path: &str, dest: &Path) -> Result<VendoredState> {
    if !dest.exists() {
        return Ok(VendoredState::Missing);
    }

    let parent = dest.parent().ok_or_else(|| {
        crate::Error::Invalid(format!("destination has no parent: {}", dest.display()))
    })?;
    std::fs::create_dir_all(parent)?;
    let temp = tempfile::Builder::new()
        .prefix(".skillmgr-check-")
        .tempdir_in(parent)?;
    git.archive(repo, rev, path, temp.path())?;

    if digest(temp.path())? == digest(dest)? {
        Ok(VendoredState::Current)
    } else {
        Ok(VendoredState::Changed)
    }
}

/// A deterministic digest over paths, types, modes, symlink targets and file contents.
pub fn digest(root: &Path) -> Result<[u8; 32]> {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).sort_by_file_name() {
        let entry = entry.map_err(|e| crate::Error::Io(std::io::Error::other(e.to_string())))?;
        let path = entry.path();
        if path == root {
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(path);
        if relative
            .components()
            .any(|c| c.as_os_str() == std::ffi::OsStr::new(".git"))
        {
            continue;
        }
        entries.push((relative.to_path_buf(), entry));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (relative, entry) in entries {
        hasher.update(relative.as_os_str().as_encoded_bytes());
        hasher.update([0]);
        let metadata = std::fs::symlink_metadata(entry.path())?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            hasher.update(b"l");
            let target = std::fs::read_link(entry.path())?;
            hasher.update(target.as_os_str().as_encoded_bytes());
        } else if file_type.is_dir() {
            hasher.update(b"d");
        } else if file_type.is_file() {
            hasher.update(b"f");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let executable = metadata.permissions().mode() & 0o111 != 0;
                hasher.update([u8::from(executable)]);
            }
            hasher.update(std::fs::read(entry.path())?);
        } else {
            hasher.update(b"?");
        }
        hasher.update([0]);
    }
    Ok(hasher.finalize().into())
}

/// Remove a skill directory.
pub fn remove_dir(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// Move a directory, falling back to copy-and-delete across filesystems.
pub fn move_dir(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_dir(from, to)?;
            std::fs::remove_dir_all(from)?;
            Ok(())
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir(&source, &target)?;
        } else if file_type.is_symlink() {
            let target_path = std::fs::read_link(&source)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(target_path, &target)?;
            #[cfg(not(unix))]
            return Err(crate::Error::Invalid(
                "symlinks are not supported on this platform".to_string(),
            ));
        } else {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

/// Path helper for a vendored destination below the managed home.
pub fn vendored_destination(skills_dir: &Path, source: &str, name: &str) -> PathBuf {
    skills_dir.join(source).join(name)
}
