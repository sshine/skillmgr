//! High-level operations shared by the CLI and TUI.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};
use crate::git::{Git, Upstream};
use crate::install::{self, InstallReport};
use crate::paths::Paths;
use crate::registry::{Entry, Origin, Registry};
use crate::repo::{Repo, normalize_skill_path};
use crate::skill::{parse_meta, read_skill_meta, scaffold, validate_name};
use crate::status::{self, SkillStatus};
use crate::vendor::{self, VendoredState};

/// Shared runtime context.
#[derive(Debug, Clone)]
pub struct Context {
    pub paths: Paths,
    pub offline: bool,
}

impl Context {
    pub fn new(paths: Paths, offline: bool) -> Self {
        Self { paths, offline }
    }

    pub fn git(&self) -> Git {
        Git::new(self.paths.cache().to_path_buf(), self.offline)
    }

    pub fn load(&self) -> Result<Registry> {
        Registry::load(&self.paths.registry())
    }
}

#[derive(Debug, Clone)]
pub struct InitReport {
    pub registry_created: bool,
    pub git_initialized: bool,
}

/// Create the managed home, registry and (optionally) a git repository.
pub fn init(ctx: &Context, no_git: bool) -> Result<InitReport> {
    ctx.paths.ensure_layout()?;
    let registry_path = ctx.paths.registry();
    let registry_created = !registry_path.exists();
    if registry_created {
        Registry::empty(&registry_path).save()?;
    }

    let git_dir = ctx.paths.root().join(".git");
    let git_initialized = if !no_git && !git_dir.exists() {
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .arg(ctx.paths.root())
            .status()
            .map_err(|e| Error::Git(format!("could not run git init: {e}")))?;
        if !status.success() {
            return Err(Error::Git("git init failed".to_string()));
        }
        true
    } else {
        false
    };

    Ok(InitReport {
        registry_created,
        git_initialized,
    })
}

#[derive(Debug, Clone)]
pub struct AddOptions {
    pub repo: String,
    pub path: String,
    pub name: Option<String>,
    pub branch: Option<String>,
    pub rev: Option<String>,
    pub no_fetch: bool,
    pub no_install: bool,
}

/// The result of resolving a repository declaration into skills.
#[derive(Debug, Clone, Default)]
pub struct ResolveReport {
    pub repo: String,
    pub entries: Vec<Entry>,
    pub skipped: Vec<String>,
    pub note: Option<String>,
    pub error: Option<String>,
    pub install: Option<InstallReport>,
}

impl ResolveReport {
    /// A short, user-facing summary.
    pub fn summary(&self) -> String {
        if let Some(error) = &self.error {
            return format!(
                "could not resolve {}: {error}; kept as a faulty entry",
                self.repo
            );
        }
        if let Some(note) = &self.note {
            let mut text = format!("{}: {note}", self.repo);
            if !self.skipped.is_empty() {
                text.push_str(&format!(" ({})", self.skipped.join("; ")));
            }
            return text;
        }
        let names: Vec<&str> = self
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        let mut text = if names.is_empty() {
            format!("{}: no skills found", self.repo)
        } else {
            format!("added {} from {}", names.join(", "), self.repo)
        };
        if !self.skipped.is_empty() {
            text.push_str(&format!(" (skipped {})", self.skipped.join("; ")));
        }
        text
    }
}

/// Declare a repository immediately, without touching the network.
///
/// Resolution into one or more skills happens separately in [`resolve_repo`]
/// so that callers can keep the UI responsive.
pub fn declare_repo(_ctx: &Context, opts: &AddOptions, registry: &mut Registry) -> Result<Entry> {
    let repo = Repo::parse(&opts.repo)?;
    let path = normalize_skill_path(&opts.path)?;
    let repo_slug = sanitize_name(repo.repo_name());
    let requested = match &opts.name {
        Some(name) => {
            validate_name(name)?;
            name.clone()
        }
        None => repo_slug.clone(),
    };

    if let Some(existing) = registry.get(&requested).cloned()
        && existing.is_repo()
        && existing.repo() == Some(repo.original())
    {
        let entry = Entry::repo_declaration(
            requested.clone(),
            existing.source,
            repo.original(),
            path.clone(),
            opts.branch.clone(),
            opts.rev.clone(),
            None,
        );
        registry.insert(entry.clone())?;
        registry.save()?;
        return Ok(entry);
    }

    if opts.name.is_some() && registry.contains(&requested) {
        return Err(Error::Conflict(format!(
            "skill '{requested}' is already declared in {}",
            registry.path().display()
        )));
    }

    let name = unique_name(registry, &requested);
    let entry = Entry::repo_declaration(
        name,
        "vendored",
        repo.original(),
        path,
        opts.branch.clone(),
        opts.rev.clone(),
        None,
    );
    registry.insert(entry.clone())?;
    registry.save()?;
    Ok(entry)
}

/// Resolve a declared repository into vendored skills.
///
/// Failures are recorded on the repository entry instead of removing it, so a
/// faulty declaration stays visible and retryable. This function performs the
/// fetch and may block on the network; callers should run it off the UI thread.
pub fn resolve_repo(
    ctx: &Context,
    registry: &mut Registry,
    name: &str,
    fetch: bool,
    no_install: bool,
) -> ResolveReport {
    let repo_label = registry
        .get(name)
        .and_then(|entry| entry.repo())
        .unwrap_or(name)
        .to_string();

    let result = resolve_repo_inner(ctx, registry, name, fetch, no_install);
    match result {
        Ok(report) => report,
        Err(e) => {
            let message = e.to_string();
            if registry.get(name).is_some_and(Entry::is_repo) {
                let _ = registry.set_repo_resolution(name, None, None, None, Some(&message));
                let _ = registry.save();
            }
            ResolveReport {
                repo: repo_label,
                error: Some(message),
                ..ResolveReport::default()
            }
        }
    }
}

fn resolve_repo_inner(
    ctx: &Context,
    registry: &mut Registry,
    name: &str,
    fetch: bool,
    no_install: bool,
) -> Result<ResolveReport> {
    let entry = registry
        .get(name)
        .cloned()
        .ok_or_else(|| Error::NotFound(format!("repository '{name}'")))?;
    let Origin::Repo {
        repo: repo_text,
        path,
        branch: stored_branch,
        rev: stored_rev,
        ..
    } = entry.origin.clone()
    else {
        return Err(Error::Invalid(format!(
            "'{name}' is not a repository declaration"
        )));
    };

    let repo = Repo::parse(&repo_text)?;
    let git = ctx.git();
    git.ensure(&repo, fetch)?;

    let branch = match stored_branch.filter(|branch| !branch.is_empty()) {
        Some(branch) => branch,
        None => git.remote_head_branch_with(&repo, false)?,
    };
    crate::registry::validate_branch(&branch)?;

    let rev = match stored_rev.filter(|rev| !rev.is_empty()) {
        Some(rev) => git.resolve_rev_with(&repo, &rev, false)?,
        None => git.resolve_branch_with(&repo, &branch, false)?,
    };

    let paths = skill_paths_for(&git, &repo, &rev, &path)?;
    if paths.is_empty() {
        registry.set_repo_resolution(
            name,
            Some(&branch),
            Some(&rev),
            Some("no SKILL.md files found"),
            None,
        )?;
        registry.save()?;
        return Ok(ResolveReport {
            repo: repo.original().to_string(),
            note: Some("no SKILL.md files found".to_string()),
            ..ResolveReport::default()
        });
    }

    let single = paths.len() == 1;
    let repo_slug = sanitize_name(repo.repo_name());
    let mut entries = Vec::new();
    let mut skipped = Vec::new();

    for skill_path in &paths {
        let content = git.read_file(&repo, &rev, skill_path)?;
        let meta = match parse_meta(&content) {
            Ok(meta) => meta,
            Err(e) => {
                skipped.push(format!("{skill_path}: {e}"));
                continue;
            }
        };

        let candidate = if single {
            if entry.name != repo_slug {
                entry.name.clone()
            } else {
                meta.name
                    .clone()
                    .unwrap_or_else(|| default_name(skill_path, repo.repo_name()))
            }
        } else {
            meta.name
                .clone()
                .unwrap_or_else(|| default_name(skill_path, repo.repo_name()))
        };
        let skill_name = sanitize_name(&candidate);
        if skill_name.is_empty() {
            skipped.push(format!("{skill_path}: could not derive a skill name"));
            continue;
        }

        if registry.contains(&skill_name) && skill_name != name {
            skipped.push(format!("{skill_name}: already declared"));
            continue;
        }

        let dest = ctx.paths.skill_dir("vendored", &skill_name);
        if dest.exists() && !registry.contains(&skill_name) {
            skipped.push(format!(
                "{skill_name}: {} already exists but is not registered",
                dest.display()
            ));
            continue;
        }

        vendor::vendor(&git, &repo, &rev, skill_path, &dest)?;
        let resolved = Entry::vendored(
            &skill_name,
            "vendored",
            repo.original(),
            skill_path,
            &branch,
            &rev,
        );
        registry.insert(resolved.clone())?;
        entries.push(resolved);
    }

    if entries.is_empty() {
        let status = if skipped.is_empty() {
            "no skills added".to_string()
        } else {
            format!("no skills added: {}", skipped.join("; "))
        };
        registry.set_repo_resolution(name, Some(&branch), Some(&rev), Some(&status), None)?;
        registry.save()?;
        return Ok(ResolveReport {
            repo: repo.original().to_string(),
            skipped,
            note: Some("no skills added".to_string()),
            ..ResolveReport::default()
        });
    }

    if registry.get(name).is_some_and(Entry::is_repo) {
        registry.remove(name);
    }
    registry.save()?;

    let install = if no_install {
        None
    } else {
        let names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
        Some(install::install(
            &ctx.paths, registry, &names, false, false,
        )?)
    };

    Ok(ResolveReport {
        repo: repo.original().to_string(),
        entries,
        skipped,
        note: None,
        error: None,
        install,
    })
}

fn skill_paths_for(git: &Git, repo: &Repo, rev: &str, path: &str) -> Result<Vec<String>> {
    if path == "." {
        return git.skill_paths(repo, rev);
    }
    if git.skill_file_exists(repo, rev, path)? {
        return Ok(vec![path.to_string()]);
    }

    let all = git.skill_paths(repo, rev)?;
    let prefix = format!("{path}/");
    let within: Vec<String> = all
        .into_iter()
        .filter(|candidate| candidate == path || candidate.starts_with(&prefix))
        .collect();
    if within.is_empty() {
        return Err(Error::NotASkill(format!(
            "{path} contains no SKILL.md in {}",
            repo.original()
        )));
    }
    Ok(within)
}

/// Declare and vendor one skill or an entire repository of skills.
pub fn add(ctx: &Context, opts: AddOptions, registry: &mut Registry) -> Result<ResolveReport> {
    let declared = declare_repo(ctx, &opts, registry)?;
    Ok(resolve_repo(
        ctx,
        registry,
        &declared.name,
        !opts.no_fetch,
        opts.no_install,
    ))
}

fn unique_name(registry: &Registry, requested: &str) -> String {
    if !registry.contains(requested) {
        return requested.to_string();
    }
    for suffix in 2.. {
        let candidate = format!("{requested}-{suffix}");
        if !registry.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("an unbounded suffix search always finds a free name")
}

/// Fold a repository or directory name into the skill naming convention.
fn sanitize_name(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = false;
    for ch in input.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub name: String,
    pub state: VendoredState,
}

/// Re-vendor selected skills at their pinned revisions.
pub fn sync(
    ctx: &Context,
    registry: &mut Registry,
    names: &[String],
    check: bool,
    no_install: bool,
) -> Result<Vec<SyncOutcome>> {
    let git = ctx.git();
    let entries = select_vendored(registry, names)?;

    let mut outcomes = Vec::new();
    let mut changed = false;
    for entry in entries {
        let (repo_text, path, _branch, rev) = vendored_parts(entry)?;
        let repo = Repo::parse(&repo_text)?;
        let rev = git.resolve_rev(&repo, &rev)?;

        if !git.path_exists(&repo, &rev, &path)? {
            return Err(Error::Git(format!(
                "skill path '{}' does not exist at {} in {}",
                path,
                rev,
                repo.original()
            )));
        }

        let dest = ctx.paths.skill_dir(&entry.source, &entry.name);
        let state = if check {
            vendor::check(&git, &repo, &rev, &path, &dest)?
        } else {
            vendor::vendor(&git, &repo, &rev, &path, &dest)?
        };

        if !check && state != VendoredState::Current {
            changed = true;
        }
        outcomes.push(SyncOutcome {
            name: entry.name.clone(),
            state,
        });
    }

    if changed && !no_install && !check {
        install::install(&ctx.paths, registry, names, false, false)?;
    }

    Ok(outcomes)
}

#[derive(Debug, Clone)]
pub struct UpdateOutcome {
    pub name: String,
    pub old_rev: String,
    pub new_rev: String,
    pub state: VendoredState,
    pub upstream: Option<Upstream>,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateOptions<'a> {
    pub to: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub no_fetch: bool,
    pub dry_run: bool,
    pub no_install: bool,
}

/// Move pins to the branch head (or a chosen revision) and re-vendor.
pub fn update(
    ctx: &Context,
    registry: &mut Registry,
    names: &[String],
    opts: UpdateOptions<'_>,
) -> Result<Vec<UpdateOutcome>> {
    let git = ctx.git();
    let selected: Vec<Entry> = select_vendored(registry, names)?
        .into_iter()
        .cloned()
        .collect();
    if opts.to.is_some() && selected.len() != 1 {
        return Err(Error::Invalid(
            "--to requires exactly one skill name".to_string(),
        ));
    }

    let mut outcomes = Vec::new();
    let mut changed = false;

    for entry in selected {
        let (repo_text, path, old_branch, old_rev) = vendored_parts(&entry)?;
        let repo = Repo::parse(&repo_text)?;
        let branch = opts.branch.unwrap_or(&old_branch).to_string();
        crate::registry::validate_branch(&branch)?;

        if !opts.no_fetch {
            git.ensure(&repo, true)?;
        }

        let new_rev = match opts.to {
            Some(rev) => git.resolve_rev_with(&repo, rev, false)?,
            None => git.resolve_branch_with(&repo, &branch, false)?,
        };

        if new_rev == old_rev && branch == old_branch {
            outcomes.push(UpdateOutcome {
                name: entry.name.clone(),
                old_rev: old_rev.clone(),
                new_rev: new_rev.clone(),
                state: VendoredState::Current,
                upstream: None,
                dry_run: opts.dry_run,
            });
            continue;
        }

        if !git.path_exists(&repo, &new_rev, &path)? {
            return Err(Error::Git(format!(
                "skill path '{}' does not exist at {} in {}; refusing to update",
                path,
                new_rev,
                repo.original()
            )));
        }

        let upstream = git
            .upstream_with(&repo, &old_rev, &branch, &path, false)
            .ok();

        if !opts.dry_run {
            let dest = ctx.paths.skill_dir(&entry.source, &entry.name);
            let state = vendor::vendor(&git, &repo, &new_rev, &path, &dest)?;
            registry.set_rev(&entry.name, &new_rev)?;
            if branch != old_branch {
                registry.set_branch(&entry.name, &branch)?;
            }
            registry.save()?;
            changed = true;
            outcomes.push(UpdateOutcome {
                name: entry.name.clone(),
                old_rev,
                new_rev,
                state,
                upstream,
                dry_run: opts.dry_run,
            });
        } else {
            outcomes.push(UpdateOutcome {
                name: entry.name.clone(),
                old_rev,
                new_rev,
                state: VendoredState::Changed,
                upstream,
                dry_run: opts.dry_run,
            });
        }
    }

    if changed && !opts.no_install {
        install::install(&ctx.paths, registry, names, false, false)?;
    }

    Ok(outcomes)
}

#[derive(Debug, Clone)]
pub struct RemoveOptions {
    pub keep_files: bool,
    pub delete_files: bool,
    pub no_uninstall: bool,
}

#[derive(Debug, Clone)]
pub struct RemoveOutcome {
    pub name: String,
    pub files_deleted: bool,
}

/// Remove declarations, links, and (depending on origin) files.
pub fn remove(
    ctx: &Context,
    registry: &mut Registry,
    names: &[String],
    opts: RemoveOptions,
) -> Result<Vec<RemoveOutcome>> {
    if names.is_empty() {
        return Err(Error::Invalid(
            "remove requires at least one skill name".to_string(),
        ));
    }
    let selected: Vec<Entry> = registry.select(names)?.into_iter().cloned().collect();
    let mut outcomes = Vec::new();

    for entry in selected {
        let delete = if opts.keep_files {
            false
        } else if opts.delete_files {
            true
        } else {
            entry.origin.is_vendored()
        };

        if !opts.no_uninstall {
            install::uninstall(
                &ctx.paths,
                registry,
                std::slice::from_ref(&entry.name),
                false,
                false,
            )?;
        }

        if delete {
            vendor::remove_dir(&ctx.paths.skill_dir(&entry.source, &entry.name))?;
        }

        registry.remove(&entry.name);
        registry.save()?;
        outcomes.push(RemoveOutcome {
            name: entry.name.clone(),
            files_deleted: delete,
        });
    }

    Ok(outcomes)
}

/// Link selected (or all) registered skills into the active directory.
pub fn install_skills(
    ctx: &Context,
    registry: &Registry,
    names: &[String],
    dry_run: bool,
    prune: bool,
) -> Result<InstallReport> {
    install::install(&ctx.paths, registry, names, dry_run, prune)
}

/// Remove links from the active directory without touching skill files.
pub fn uninstall_skills(
    ctx: &Context,
    registry: &Registry,
    names: &[String],
    all: bool,
    dry_run: bool,
) -> Result<InstallReport> {
    install::uninstall(&ctx.paths, registry, names, all, dry_run)
}

/// Scaffold an authored skill.
pub fn new_skill(
    ctx: &Context,
    registry: &mut Registry,
    name: &str,
    description: &str,
    no_install: bool,
) -> Result<Entry> {
    validate_name(name)?;
    if registry.contains(name) {
        return Err(Error::Conflict(format!(
            "skill '{name}' is already declared"
        )));
    }
    let dir = ctx.paths.skill_dir("authored", name);
    if dir.exists() {
        return Err(Error::Conflict(format!(
            "{} already exists; use `skillmgr register {name}`",
            dir.display()
        )));
    }

    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("SKILL.md"), scaffold(name, description))?;

    let entry = Entry::authored(name, "authored");
    registry.insert(entry.clone())?;
    registry.save()?;

    if !no_install {
        install::install(
            &ctx.paths,
            registry,
            std::slice::from_ref(&name.to_string()),
            false,
            false,
        )?;
    }
    Ok(entry)
}

/// Register an existing directory below `skills/`.
pub fn register(
    ctx: &Context,
    registry: &mut Registry,
    name: &str,
    source: Option<&str>,
    no_install: bool,
) -> Result<Entry> {
    validate_name(name)?;
    if registry.contains(name) {
        return Err(Error::Conflict(format!(
            "skill '{name}' is already declared"
        )));
    }

    let candidates: Vec<(String, PathBuf)> = match source {
        Some(source) => vec![(source.to_string(), ctx.paths.skill_dir(source, name))],
        None => status::scan_sources(&ctx.paths)?
            .into_iter()
            .filter(|(_, found, _)| found == name)
            .map(|(source, _, path)| (source, path))
            .collect(),
    };

    let (source, path) = match candidates.as_slice() {
        [] => {
            return Err(Error::NotFound(format!(
                "no directory named '{name}' under {}",
                ctx.paths.skills_dir().display()
            )));
        }
        [one] => (one.0.clone(), one.1.clone()),
        _ => {
            return Err(Error::Conflict(format!(
                "multiple directories named '{name}'; pass --source"
            )));
        }
    };

    if !path.join("SKILL.md").is_file() {
        return Err(Error::NotASkill(path.display().to_string()));
    }

    let entry = Entry::authored(name, source);
    registry.insert(entry.clone())?;
    registry.save()?;

    if !no_install {
        install::install(
            &ctx.paths,
            registry,
            std::slice::from_ref(&name.to_string()),
            false,
            false,
        )?;
    }
    Ok(entry)
}

/// Move an existing real skill directory into `skills/authored/` and link it.
pub fn adopt(
    ctx: &Context,
    registry: &mut Registry,
    name: &str,
    from: Option<&Path>,
    no_install: bool,
) -> Result<Entry> {
    validate_name(name)?;
    if registry.contains(name) {
        return Err(Error::Conflict(format!(
            "skill '{name}' is already declared"
        )));
    }

    let source = from
        .map(Path::to_path_buf)
        .unwrap_or_else(|| ctx.paths.dest().join(name));

    let metadata = source
        .symlink_metadata()
        .map_err(|_| Error::NotFound(format!("{} does not exist", source.display())))?;
    if metadata.file_type().is_symlink() {
        return Err(Error::Conflict(format!(
            "{} is a symlink, not a real directory",
            source.display()
        )));
    }
    if !metadata.is_dir() || !source.join("SKILL.md").is_file() {
        return Err(Error::NotASkill(source.display().to_string()));
    }

    let dest = ctx.paths.skill_dir("authored", name);
    if dest.exists() {
        return Err(Error::Conflict(format!(
            "{} already exists",
            dest.display()
        )));
    }

    vendor::move_dir(&source, &dest)?;
    let entry = Entry::authored(name, "authored");
    registry.insert(entry.clone())?;
    registry.save()?;

    if !no_install {
        install::install(
            &ctx.paths,
            registry,
            std::slice::from_ref(&name.to_string()),
            false,
            false,
        )?;
    }
    Ok(entry)
}

/// Open a file in the user's editor.
pub fn open_editor(path: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let program = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Invalid("empty editor command".to_string()))?;

    let status = Command::new(program)
        .args(parts)
        .arg(path)
        .status()
        .map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("could not run {program}: {e}"),
            ))
        })?;
    if !status.success() {
        return Err(Error::Invalid(format!(
            "editor '{program}' exited with {status}"
        )));
    }
    Ok(())
}

/// The SKILL.md for a registered skill.
pub fn skill_file(ctx: &Context, registry: &Registry, name: &str) -> Result<PathBuf> {
    let entry = registry
        .get(name)
        .ok_or_else(|| Error::NotFound(format!("skill '{name}'")))?;
    let dir = ctx.paths.skill_dir(&entry.source, &entry.name);
    if !dir.is_dir() {
        return Err(Error::NotFound(format!(
            "managed directory {}",
            dir.display()
        )));
    }
    Ok(dir.join("SKILL.md"))
}

/// List skills, optionally checking upstream state.
pub fn list(
    ctx: &Context,
    registry: &Registry,
    names: &[String],
    check: bool,
    outdated: bool,
) -> Result<Vec<SkillStatus>> {
    if !names.is_empty() {
        status::ensure_names_exist(&ctx.paths, registry, names)?;
    }
    let mut statuses = status::local(&ctx.paths, registry)?;
    statuses = status::filter(statuses, names);

    if check || outdated {
        status::check_upstream(&ctx.git(), &mut statuses);
    }
    if outdated {
        statuses.retain(|s| s.is_outdated());
    }
    Ok(statuses)
}

fn select_vendored<'a>(registry: &'a Registry, names: &[String]) -> Result<Vec<&'a Entry>> {
    if names.is_empty() {
        return Ok(registry
            .entries()
            .filter(|e| e.origin.is_vendored())
            .collect());
    }
    names
        .iter()
        .map(|name| {
            let entry = registry
                .get(name)
                .ok_or_else(|| Error::NotFound(format!("skill '{name}'")))?;
            if !entry.origin.is_vendored() {
                return Err(Error::Invalid(format!(
                    "skill '{name}' is authored and has no upstream"
                )));
            }
            Ok(entry)
        })
        .collect()
}

fn vendored_parts(entry: &Entry) -> Result<(String, String, String, String)> {
    match &entry.origin {
        Origin::Vendored {
            repo,
            path,
            branch,
            rev,
        } => Ok((repo.clone(), path.clone(), branch.clone(), rev.clone())),
        Origin::Repo { .. } => Err(Error::Invalid(format!(
            "'{}' is an unresolved repository, not a skill",
            entry.name
        ))),
        Origin::Authored => Err(Error::Invalid(format!(
            "skill '{}' is authored and has no upstream",
            entry.name
        ))),
    }
}

fn default_name(path: &str, repo_name: &str) -> String {
    if path == "." {
        return repo_name.to_string();
    }
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo_name.to_string())
}

/// Read a skill's metadata from the managed directory, if present.
pub fn managed_meta(
    ctx: &Context,
    registry: &Registry,
    name: &str,
) -> Option<crate::skill::SkillMeta> {
    let entry = registry.get(name)?;
    read_skill_meta(&ctx.paths.skill_dir(&entry.source, &entry.name)).ok()
}
