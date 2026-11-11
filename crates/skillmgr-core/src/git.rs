//! Git access through a disposable, named origin directory per source.
//!
//! Every upstream is a bare mirror under
//! `$XDG_CACHE_HOME/skillmgr/origins/<method>/<host>/<user>/<namespace>/<repo>.git`.
//! The layout is derived from the canonical source URL, so it is human-readable and
//! unambiguous. Deleting the cache never affects vendored or installed skills.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::error::{Error, Result};
use crate::registry::validate_branch;
use crate::repo::Repo;

/// What the upstream history says about a pinned revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upstream {
    Current,
    Behind { repo: u32, path: u32 },
    Diverged,
    PathRemoved { repo: u32 },
    Unreachable,
    Unknown(String),
}

/// The git cache and its fetch policy.
#[derive(Debug, Clone)]
pub struct Git {
    cache_root: PathBuf,
    offline: bool,
}

impl Git {
    pub fn new(cache_root: PathBuf, offline: bool) -> Self {
        Self {
            cache_root,
            offline,
        }
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub fn is_offline(&self) -> bool {
        self.offline
    }

    /// The origin mirror path for a repository.
    pub fn origin_path(&self, repo: &Repo) -> PathBuf {
        repo.origin_path(&self.cache_root)
    }

    /// Ensure a bare mirror exists, and optionally fetch it.
    pub fn ensure(&self, repo: &Repo, fetch: bool) -> Result<PathBuf> {
        let path = self.origin_path(repo);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let path_arg = path.to_string_lossy().into_owned();
            run(None, &["init", "--bare", "--quiet", &path_arg])?;
        }

        if !path.join("objects").is_dir() {
            return Err(Error::Git(format!(
                "origin path is not a git repository: {}",
                path.display()
            )));
        }

        self.configure_remote(repo, &path)?;

        if fetch && !self.offline {
            self.fetch(repo)?;
        }
        Ok(path)
    }

    fn configure_remote(&self, repo: &Repo, path: &Path) -> Result<()> {
        let url = repo.url();
        let _ = run(Some(path), &["config", "remote.origin.url", url]);
        let _ = run(
            Some(path),
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/*",
            ],
        );
        Ok(())
    }

    /// Fetch branch heads and tags into the mirror.
    pub fn fetch(&self, repo: &Repo) -> Result<()> {
        if self.offline {
            return Ok(());
        }
        let path = self.origin_path(repo);
        if !path.join("objects").is_dir() {
            return Err(Error::Offline(format!(
                "no origin mirror for {}",
                repo.original()
            )));
        }

        // A partial clone keeps the mirror small. The commit graph is complete,
        // which is what the behind counts need; trees and blobs are fetched on
        // demand when a skill is archived.
        let _ = run(Some(&path), &["config", "remote.origin.promisor", "true"]);
        let _ = run(
            Some(&path),
            &["config", "remote.origin.partialclonefilter", "tree:0"],
        );

        let filtered = run(
            Some(&path),
            &[
                "fetch",
                "--quiet",
                "--prune",
                "--filter=tree:0",
                "origin",
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ],
        );
        if filtered.is_ok() {
            return Ok(());
        }

        // Self-hosted servers may refuse filters. Fall back to a complete fetch
        // rather than treating an inconvenience as an error.
        let _ = run(
            Some(&path),
            &["config", "--unset-all", "remote.origin.promisor"],
        );
        let _ = run(
            Some(&path),
            &["config", "--unset-all", "remote.origin.partialclonefilter"],
        );
        run(
            Some(&path),
            &[
                "fetch",
                "--quiet",
                "--prune",
                "origin",
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ],
        )?;
        Ok(())
    }

    /// The remote's default branch, without requiring a full fetch.
    pub fn remote_head_branch(&self, repo: &Repo) -> Result<String> {
        self.remote_head_branch_with(repo, true)
    }

    /// The default branch, optionally asking the remote first.
    pub fn remote_head_branch_with(&self, repo: &Repo, use_remote: bool) -> Result<String> {
        if use_remote && !self.offline {
            let output = run(None, &["ls-remote", "--symref", repo.url(), "HEAD"])?;
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("ref: ")
                    && let Some((target, _)) = rest.split_once('\t')
                    && let Some(branch) = target.strip_prefix("refs/heads/")
                {
                    validate_branch(branch)?;
                    return Ok(branch.to_string());
                }
            }
        }

        // Offline or no symref advertised: use the local mirror.
        let path = self.ensure(repo, false)?;
        for candidate in ["main", "master"] {
            if run(
                Some(&path),
                &["rev-parse", "--verify", &format!("refs/heads/{candidate}")],
            )
            .is_ok()
            {
                return Ok(candidate.to_string());
            }
        }
        let output = run(
            Some(&path),
            &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
        )?;
        let branch = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| Error::Offline(format!("no cached branch for {}", repo.original())))?;
        Ok(branch)
    }

    /// Resolve a branch name to a full commit SHA in the mirror.
    pub fn resolve_branch(&self, repo: &Repo, branch: &str) -> Result<String> {
        self.resolve_branch_with(repo, branch, true)
    }

    /// Resolve a branch, optionally skipping a fetch.
    pub fn resolve_branch_with(&self, repo: &Repo, branch: &str, fetch: bool) -> Result<String> {
        validate_branch(branch)?;
        let path = self.ensure(repo, fetch)?;
        let output = run(
            Some(&path),
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{branch}^{{commit}}"),
            ],
        )?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Resolve any rev-ish expression to a full commit SHA.
    pub fn resolve_rev(&self, repo: &Repo, rev: &str) -> Result<String> {
        self.resolve_rev_with(repo, rev, true)
    }

    /// Resolve a revision, optionally skipping a fetch.
    pub fn resolve_rev_with(&self, repo: &Repo, rev: &str, fetch: bool) -> Result<String> {
        if rev.is_empty() || rev.starts_with('-') || rev.contains(char::is_whitespace) {
            return Err(Error::Invalid(format!("invalid revision: {rev}")));
        }
        let path = self.ensure(repo, fetch)?;
        let output = run(
            Some(&path),
            &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
        )
        .map_err(|_| {
            Error::Git(format!(
                "revision '{rev}' was not found in {}; it may not be reachable from any branch",
                repo.original()
            ))
        })?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Whether a skill path exists at a revision.
    pub fn path_exists(&self, repo: &Repo, rev: &str, path: &str) -> Result<bool> {
        let origin = self.ensure(repo, false)?;
        let treeish = treeish(rev, path);
        Ok(run(Some(&origin), &["cat-file", "-e", &treeish]).is_ok())
    }

    /// Every directory containing a `SKILL.md` at a revision.
    ///
    /// The root directory is reported as `"."`. An empty result means the
    /// repository is a perfectly valid skills collection with no skills in it.
    pub fn skill_paths(&self, repo: &Repo, rev: &str) -> Result<Vec<String>> {
        let origin = self.ensure(repo, false)?;
        let output = match run(Some(&origin), &["ls-tree", "-r", "--name-only", rev]) {
            Ok(output) => output,
            Err(_) if !self.offline => {
                // A partial clone may lack trees for this revision. Refetch
                // without the filter and retry once.
                self.fetch_complete(repo)?;
                run(Some(&origin), &["ls-tree", "-r", "--name-only", rev])?
            }
            Err(e) => return Err(e),
        };

        let mut paths = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line == "SKILL.md" {
                paths.push(".".to_string());
            } else if let Some(parent) = line.strip_suffix("/SKILL.md") {
                paths.push(parent.to_string());
            }
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    /// Whether `<rev>:<path>/SKILL.md` exists.
    pub fn skill_file_exists(&self, repo: &Repo, rev: &str, path: &str) -> Result<bool> {
        let origin = self.ensure(repo, false)?;
        let file = if path == "." {
            format!("{rev}:SKILL.md")
        } else {
            format!("{rev}:{path}/SKILL.md")
        };
        Ok(run(Some(&origin), &["cat-file", "-e", &file]).is_ok())
    }

    /// Read a file from a revision.
    pub fn read_file(&self, repo: &Repo, rev: &str, path: &str) -> Result<String> {
        let origin = self.ensure(repo, false)?;
        let file = if path == "." {
            format!("{rev}:SKILL.md")
        } else {
            format!("{rev}:{path}/SKILL.md")
        };
        let output = run(Some(&origin), &["show", &file])?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Extract `<rev>:<path>` into `dest` as a tar archive.
    pub fn archive(&self, repo: &Repo, rev: &str, path: &str, dest: &Path) -> Result<()> {
        let origin = self.ensure(repo, false)?;
        std::fs::create_dir_all(dest)?;
        let treeish = treeish(rev, path);
        let output = run(Some(&origin), &["archive", "--format=tar", &treeish]);

        let output = match output {
            Ok(output) => output,
            Err(_) if !self.offline => {
                // A partial clone may lack the trees/blobs for this revision.
                // Refetch without the filter and retry once.
                self.fetch_complete(repo)?;
                run(Some(&origin), &["archive", "--format=tar", &treeish])?
            }
            Err(e) => return Err(e),
        };

        let mut archive = tar::Archive::new(Cursor::new(output.stdout));
        archive
            .unpack(dest)
            .map_err(|e| Error::Git(format!("failed to unpack {}: {e}", repo.original())))?;
        Ok(())
    }

    fn fetch_complete(&self, repo: &Repo) -> Result<()> {
        if self.offline {
            return Err(Error::Offline(format!("cannot fetch {}", repo.original())));
        }
        let path = self.ensure(repo, false)?;
        let _ = run(
            Some(&path),
            &["config", "--unset-all", "remote.origin.promisor"],
        );
        let _ = run(
            Some(&path),
            &["config", "--unset-all", "remote.origin.partialclonefilter"],
        );
        run(
            Some(&path),
            &[
                "fetch",
                "--quiet",
                "--prune",
                "origin",
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ],
        )?;
        Ok(())
    }

    /// Compare a pinned revision with the recorded branch head.
    pub fn upstream(&self, repo: &Repo, rev: &str, branch: &str, path: &str) -> Result<Upstream> {
        self.upstream_with(repo, rev, branch, path, true)
    }

    /// Compare a pin, optionally skipping a fetch.
    pub fn upstream_with(
        &self,
        repo: &Repo,
        rev: &str,
        branch: &str,
        path: &str,
        fetch: bool,
    ) -> Result<Upstream> {
        let origin = self.ensure(repo, fetch)?;

        if run(
            Some(&origin),
            &["cat-file", "-e", &format!("{rev}^{{commit}}")],
        )
        .is_err()
        {
            return Ok(Upstream::Unreachable);
        }

        let tip = match self.resolve_branch_with(repo, branch, false) {
            Ok(tip) => tip,
            Err(e) => {
                return Ok(Upstream::Unknown(e.to_string()));
            }
        };

        if rev == tip {
            return Ok(Upstream::Current);
        }

        if run(Some(&origin), &["merge-base", "--is-ancestor", rev, &tip]).is_err() {
            return Ok(Upstream::Diverged);
        }

        let repo_behind = count(&origin, rev, &tip, None)?;
        if path != "." && !self.path_exists(repo, &tip, path)? {
            return Ok(Upstream::PathRemoved { repo: repo_behind });
        }
        let path_behind = if path == "." {
            repo_behind
        } else {
            count(&origin, rev, &tip, Some(path))?
        };

        Ok(Upstream::Behind {
            repo: repo_behind,
            path: path_behind,
        })
    }

    /// The last commit date touching a path at a branch tip.
    pub fn last_commit_date(
        &self,
        repo: &Repo,
        branch: &str,
        path: &str,
    ) -> Result<Option<String>> {
        let origin = self.ensure(repo, true)?;
        let tip = self.resolve_branch(repo, branch)?;
        let mut args = vec!["log", "-1", "--format=%cs", &tip];
        if path != "." {
            args.push("--");
            args.push(path);
        }
        let output = run(Some(&origin), &args)?;
        let date = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok((!date.is_empty()).then_some(date))
    }
}

fn treeish(rev: &str, path: &str) -> String {
    if path == "." {
        rev.to_string()
    } else {
        format!("{rev}:{path}")
    }
}

fn count(origin: &Path, from: &str, to: &str, path: Option<&str>) -> Result<u32> {
    let range = format!("{from}..{to}");
    let mut args = vec!["rev-list", "--count", &range];
    if let Some(path) = path {
        args.push("--");
        args.push(path);
    }
    let output = run(Some(origin), &args)?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .map_err(|e| Error::Git(format!("could not parse commit count: {e}")))
}

fn run(dir: Option<&Path>, args: &[&str]) -> Result<Output> {
    let mut command = Command::new("git");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    command.args(args);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());

    let output = command
        .output()
        .map_err(|e| Error::Git(format!("could not run git (is it on PATH?): {e}")))?;

    if output.status.success() {
        Ok(output)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        Err(Error::Git(format!(
            "git {} failed: {detail}",
            args.join(" ")
        )))
    }
}
