use std::path::{Path, PathBuf};
use std::process::Command;

use skillmgr_core::ops::{self, AddOptions, Context, RemoveOptions};
use skillmgr_core::paths::Paths;
use skillmgr_core::registry::Registry;
use skillmgr_core::status;

struct Fixture {
    dir: tempfile::TempDir,
    repo: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("skills/hello")).expect("mkdir");
        std::fs::write(
            repo.join("skills/hello/SKILL.md"),
            "---\nname: hello\ndescription: A demo skill.\nmetadata:\n  version: \"1.0\"\n---\n# Hello\n",
        )
        .expect("skill");
        std::fs::write(repo.join("README.md"), "one\n").expect("readme");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "initial"]);
        Self { dir, repo }
    }

    fn context(&self) -> Context {
        let paths = Paths::resolve(
            Some(self.dir.path().join("root")),
            Some(self.dir.path().join("dest")),
            Some(self.dir.path().join("cache")),
        )
        .expect("paths");
        let ctx = Context::new(paths, false);
        ops::init(&ctx, true).expect("init");
        ctx
    }

    fn commit(&self, message: &str, path: &str, content: &str) {
        let target = self.repo.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(target, content).expect("write");
        git(&self.repo, &["add", "."]);
        git(&self.repo, &["commit", "-qm", message]);
    }

    fn head(&self) -> String {
        git_output(&self.repo, &["rev-parse", "HEAD"])
    }

    fn url(&self) -> String {
        format!("file://{}", self.repo.display())
    }
}

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?}");
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn add_hello(ctx: &Context, fixture: &Fixture, rev: Option<&str>) {
    let mut registry = ctx.load().expect("load");
    ops::add(
        ctx,
        AddOptions {
            repo: fixture.url(),
            path: "skills/hello".to_string(),
            name: None,
            branch: Some("main".to_string()),
            rev: rev.map(str::to_string),
            no_fetch: false,
            no_install: false,
        },
        &mut registry,
    )
    .expect("add");
}

#[test]
fn add_vendors_and_links_a_skill() {
    let fixture = Fixture::new();
    let ctx = fixture.context();
    add_hello(&ctx, &fixture, None);

    let registry = Registry::load(&ctx.paths.registry()).expect("registry");
    let entry = registry.get("hello").expect("entry");
    assert_eq!(entry.source, "vendored");
    assert!(
        ctx.paths
            .skill_dir("vendored", "hello")
            .join("SKILL.md")
            .is_file()
    );
    assert!(ctx.paths.dest().join("hello").is_symlink());

    let statuses = ops::list(&ctx, &registry, &[], false, false).expect("list");
    assert_eq!(statuses.len(), 1);
    assert!(statuses[0].link == skillmgr_core::install::LinkState::Linked);
}

#[test]
fn behind_counts_separate_repo_and_path_commits() {
    let fixture = Fixture::new();
    let ctx = fixture.context();
    let first = fixture.head();
    add_hello(&ctx, &fixture, Some(&first));

    fixture.commit("unrelated", "README.md", "two\n");
    fixture.commit(
        "skill change",
        "skills/hello/SKILL.md",
        "---\nname: hello\ndescription: A demo skill.\n---\n# Hello\nBe kind.\n",
    );

    let registry = ctx.load().expect("registry");
    let statuses = ops::list(&ctx, &registry, &[], true, false).expect("status");
    let upstream = statuses[0].upstream.as_ref().expect("upstream");
    assert_eq!(
        upstream,
        &skillmgr_core::git::Upstream::Behind { repo: 2, path: 1 }
    );
}

#[test]
fn update_moves_the_pin_and_revendors() {
    let fixture = Fixture::new();
    let ctx = fixture.context();
    add_hello(&ctx, &fixture, None);
    fixture.commit("unrelated", "README.md", "two\n");
    fixture.commit(
        "skill change",
        "skills/hello/SKILL.md",
        "---\nname: hello\ndescription: A demo skill.\n---\n# Hello\nBe kind.\n",
    );

    let mut registry = ctx.load().expect("registry");
    let outcomes = ops::update(
        &ctx,
        &mut registry,
        &[],
        ops::UpdateOptions {
            to: None,
            branch: None,
            no_fetch: false,
            dry_run: false,
            no_install: false,
        },
    )
    .expect("update");
    assert_eq!(outcomes.len(), 1);

    let registry = ctx.load().expect("registry");
    assert_eq!(
        registry.get("hello").expect("entry").rev(),
        Some(fixture.head().as_str())
    );
    let vendored =
        std::fs::read_to_string(ctx.paths.skill_dir("vendored", "hello").join("SKILL.md"))
            .expect("read");
    assert!(vendored.contains("Be kind."));
}

#[test]
fn sync_check_detects_local_drift_and_sync_repairs_it() {
    let fixture = Fixture::new();
    let ctx = fixture.context();
    add_hello(&ctx, &fixture, None);

    let skill_file = ctx.paths.skill_dir("vendored", "hello").join("SKILL.md");
    std::fs::write(&skill_file, "tampered").expect("tamper");

    let mut registry = ctx.load().expect("registry");
    let checked = ops::sync(&ctx, &mut registry, &[], true, false).expect("check");
    assert_eq!(
        checked[0].state,
        skillmgr_core::vendor::VendoredState::Changed
    );

    ops::sync(&ctx, &mut registry, &[], false, false).expect("sync");
    let checked = ops::sync(&ctx, &mut registry, &[], true, false).expect("check");
    assert_eq!(
        checked[0].state,
        skillmgr_core::vendor::VendoredState::Current
    );
}

#[test]
fn install_refuses_a_real_directory_and_adopt_moves_it() {
    let fixture = Fixture::new();
    let ctx = fixture.context();
    add_hello(&ctx, &fixture, None);

    std::fs::remove_file(ctx.paths.dest().join("hello")).expect("unlink");
    std::fs::create_dir_all(ctx.paths.dest().join("hello")).expect("real dir");
    std::fs::write(
        ctx.paths.dest().join("hello/SKILL.md"),
        "---\nname: hello\ndescription: Real copy.\n---\n",
    )
    .expect("skill");

    let registry = ctx.load().expect("registry");
    let report = ops::install_skills(&ctx, &registry, &[], false, true).expect("install");
    assert!(!report.conflicts.is_empty());

    let mut registry = ctx.load().expect("registry");
    ops::adopt(&ctx, &mut registry, "hello", None, false).expect_err("already registered");

    std::fs::remove_dir_all(ctx.paths.dest().join("hello")).expect("remove");
    std::fs::create_dir_all(ctx.paths.dest().join("real-skill")).expect("real dir");
    std::fs::write(
        ctx.paths.dest().join("real-skill/SKILL.md"),
        "---\nname: real-skill\ndescription: Real.\n---\n",
    )
    .expect("skill");
    ops::adopt(&ctx, &mut registry, "real-skill", None, false).expect("adopt");
    assert!(ctx.paths.skill_dir("authored", "real-skill").is_dir());
    assert!(ctx.paths.dest().join("real-skill").is_symlink());
}

#[test]
fn a_collection_repository_adds_every_skill() {
    let fixture = Fixture::new();
    fixture.commit(
        "second skill",
        "skills/goodbye/SKILL.md",
        "---\nname: goodbye\ndescription: Another demo skill.\n---\n# Goodbye\n",
    );
    let ctx = fixture.context();
    let mut registry = ctx.load().expect("registry");

    let report = ops::add(
        &ctx,
        AddOptions {
            repo: fixture.url(),
            path: ".".to_string(),
            name: None,
            branch: None,
            rev: None,
            no_fetch: false,
            no_install: false,
        },
        &mut registry,
    )
    .expect("add");

    assert!(report.error.is_none());
    let names: Vec<&str> = report.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["goodbye", "hello"]);

    let registry = ctx.load().expect("registry");
    assert!(registry.get("hello").expect("hello").is_vendored());
    assert!(registry.get("goodbye").expect("goodbye").is_vendored());
    assert!(ctx.paths.dest().join("hello").is_symlink());
    assert!(ctx.paths.dest().join("goodbye").is_symlink());
}

#[test]
fn a_repository_without_skills_is_kept_as_an_empty_repository() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("empty");
    std::fs::create_dir_all(&repo).expect("mkdir");
    std::fs::write(repo.join("README.md"), "nothing to see\n").expect("readme");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "initial"]);

    let paths = Paths::resolve(
        Some(dir.path().join("root")),
        Some(dir.path().join("dest")),
        Some(dir.path().join("cache")),
    )
    .expect("paths");
    let ctx = Context::new(paths, false);
    ops::init(&ctx, true).expect("init");

    let mut registry = ctx.load().expect("registry");
    let report = ops::add(
        &ctx,
        AddOptions {
            repo: format!("file://{}", repo.display()),
            path: ".".to_string(),
            name: None,
            branch: None,
            rev: None,
            no_fetch: false,
            no_install: false,
        },
        &mut registry,
    )
    .expect("add");

    assert!(report.error.is_none());
    assert!(report.entries.is_empty());
    assert!(report.note.is_some());

    let registry = ctx.load().expect("registry");
    let entry = registry.get("empty").expect("repository entry");
    assert!(entry.is_repo());
    assert_eq!(entry.repo_status(), Some("no SKILL.md files found"));
}

#[test]
fn an_unreachable_repository_stays_as_a_faulty_entry() {
    let fixture = Fixture::new();
    let ctx = fixture.context();
    let mut registry = ctx.load().expect("registry");

    let missing = format!("file://{}", fixture.dir.path().join("missing").display());
    let entry = ops::declare_repo(
        &ctx,
        &AddOptions {
            repo: missing,
            path: ".".to_string(),
            name: None,
            branch: None,
            rev: None,
            no_fetch: false,
            no_install: false,
        },
        &mut registry,
    )
    .expect("declare");

    let report = ops::resolve_repo(&ctx, &mut registry, &entry.name, true, false);
    assert!(report.error.is_some());

    let registry = ctx.load().expect("registry");
    let entry = registry.get("missing").expect("faulty entry");
    assert!(entry.is_repo());
    assert!(entry.repo_error().is_some());

    // Local processing of a faulty entry is a no-op, not an error.
    let report = ops::install_skills(&ctx, &registry, &[], false, true).expect("install");
    assert!(report.conflicts.is_empty());
    let statuses = ops::list(&ctx, &registry, &[], false, false).expect("list");
    assert!(statuses.iter().any(|s| s.is_repo()));
}

#[test]
fn new_register_and_remove() {
    let fixture = Fixture::new();
    let ctx = fixture.context();
    let mut registry = ctx.load().expect("registry");

    ops::new_skill(&ctx, &mut registry, "mine", "Mine.", false).expect("new");
    assert!(
        ctx.paths
            .skill_dir("authored", "mine")
            .join("SKILL.md")
            .is_file()
    );
    assert!(ctx.paths.dest().join("mine").is_symlink());

    std::fs::create_dir_all(ctx.paths.skill_dir("authored", "stray")).expect("stray");
    std::fs::write(
        ctx.paths.skill_dir("authored", "stray").join("SKILL.md"),
        "---\nname: stray\ndescription: Stray.\n---\n",
    )
    .expect("skill");
    let statuses = status::local(&ctx.paths, &ctx.load().expect("registry")).expect("local");
    assert!(statuses.iter().any(|s| s.name == "stray" && !s.registered));

    ops::register(&ctx, &mut registry, "stray", None, false).expect("register");
    assert!(ctx.paths.dest().join("stray").is_symlink());

    let outcomes = ops::remove(
        &ctx,
        &mut registry,
        &["stray".to_string()],
        RemoveOptions {
            keep_files: false,
            delete_files: true,
            no_uninstall: false,
        },
    )
    .expect("remove");
    assert!(outcomes[0].files_deleted);
    assert!(!ctx.paths.skill_dir("authored", "stray").exists());
    assert!(!ctx.paths.dest().join("stray").exists());
}
