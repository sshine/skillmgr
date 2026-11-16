use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    repo: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("skills/hello")).expect("mkdir");
        std::fs::write(
            repo.join("skills/hello/SKILL.md"),
            "---\nname: hello\ndescription: A demo skill.\n---\n# Hello\n",
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

    fn commit(&self, message: &str, path: &str, content: &str) {
        let target = self.repo.join(path);
        std::fs::write(target, content).expect("write");
        git(&self.repo, &["add", "."]);
        git(&self.repo, &["commit", "-qm", message]);
    }

    fn url(&self) -> String {
        format!("file://{}", self.repo.display())
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("root")
    }

    fn dest(&self) -> PathBuf {
        self.dir.path().join("dest")
    }

    fn cache(&self) -> PathBuf {
        self.dir.path().join("cache")
    }

    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("skillmgr").expect("binary");
        command
            .arg("--root")
            .arg(self.root())
            .arg("--dest")
            .arg(self.dest())
            .arg("--cache-dir")
            .arg(self.cache())
            .arg("--state-dir")
            .arg(self.state())
            .arg("--no-color");
        command
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

#[test]
fn help_lists_the_full_command_set() {
    let mut command = Command::cargo_bin("skillmgr").expect("binary");
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("add"))
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("update"))
        .stdout(predicate::str::contains("adopt"))
        .stdout(predicate::str::contains("tui"));
}

#[test]
fn init_add_status_update_and_json() {
    let fixture = Fixture::new();
    fixture
        .command()
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("skills.toml"));

    fixture
        .command()
        .args(["add", &fixture.url(), "skills/hello"])
        .assert()
        .success()
        .stdout(predicate::str::contains("added hello"))
        .stdout(predicate::str::contains("linked hello"));

    let dest_link = fixture.dest().join("hello");
    assert!(dest_link.is_symlink());

    fixture
        .command()
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello"))
        .stdout(predicate::str::contains("0/0"));

    fixture.commit("unrelated", "README.md", "two\n");
    fixture.commit(
        "skill change",
        "skills/hello/SKILL.md",
        "---\nname: hello\ndescription: A demo skill.\n---\n# Hello\nBe kind.\n",
    );

    fixture
        .command()
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2/1"));

    fixture
        .command()
        .args(["update"])
        .assert()
        .success()
        .stdout(predicate::str::contains("updated hello"));

    fixture
        .command()
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"behind\": null"))
        .stdout(predicate::str::contains("\"upstream\": \"current\""));
}

#[test]
fn sync_check_exits_nonzero_on_drift() {
    let fixture = Fixture::new();
    fixture.command().arg("init").assert().success();
    fixture
        .command()
        .args(["add", &fixture.url(), "skills/hello"])
        .assert()
        .success();

    std::fs::write(
        fixture.root().join("skills/vendored/hello/SKILL.md"),
        "tampered",
    )
    .expect("tamper");

    fixture
        .command()
        .args(["sync", "--check"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("changed"));

    fixture.command().args(["sync"]).assert().success();

    fixture
        .command()
        .args(["sync", "--check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("in sync"));
}

#[test]
fn install_reports_a_real_directory_conflict() {
    let fixture = Fixture::new();
    fixture.command().arg("init").assert().success();
    fixture
        .command()
        .args(["add", &fixture.url(), "skills/hello", "--no-install"])
        .assert()
        .success();

    std::fs::create_dir_all(fixture.dest().join("hello")).expect("real dir");
    std::fs::write(
        fixture.dest().join("hello/SKILL.md"),
        "---\nname: hello\ndescription: Real.\n---\n",
    )
    .expect("skill");

    fixture
        .command()
        .arg("install")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("conflict"));

    std::fs::remove_dir_all(fixture.dest().join("hello")).expect("remove");
    std::fs::create_dir_all(fixture.dest().join("real")).expect("real dir");
    std::fs::write(
        fixture.dest().join("real/SKILL.md"),
        "---\nname: real\ndescription: Real.\n---\n",
    )
    .expect("skill");

    fixture
        .command()
        .args(["adopt", "real"])
        .assert()
        .success()
        .stdout(predicate::str::contains("adopted real"));

    assert!(fixture.dest().join("real").is_symlink());
    assert!(
        fixture
            .root()
            .join("skills/authored/real/SKILL.md")
            .is_file()
    );
}

#[test]
fn a_faulty_repository_is_kept_and_reported() {
    let fixture = Fixture::new();
    fixture.command().arg("init").assert().success();

    let missing = format!("file://{}", fixture.dir.path().join("missing").display());
    fixture
        .command()
        .args(["add", &missing])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("kept faulty repository"));

    fixture
        .command()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("faulty"));
}

#[test]
fn no_arguments_in_a_pipe_prints_help() {
    let fixture = Fixture::new();
    fixture
        .command()
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: skillmgr"));
}

/// A fixture whose paths are supplied only through the environment, the way a
/// test of an installed binary has to isolate itself from the real home.
struct EnvFixture {
    dir: TempDir,
    home: PathBuf,
    root: PathBuf,
    dest: PathBuf,
    cache: PathBuf,
    state: PathBuf,
}

impl EnvFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        Self {
            root: dir.path().join("managed"),
            dest: dir.path().join("active"),
            cache: dir.path().join("cache"),
            state: dir.path().join("state"),
            home,
            dir,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("skillmgr").expect("binary");
        command
            .env("HOME", &self.home)
            .env("SKILLMGR_ROOT", &self.root)
            .env("SKILLMGR_DEST", &self.dest)
            .env("SKILLMGR_CACHE_DIR", &self.cache)
            .env("SKILLMGR_STATE_DIR", &self.state)
            // The dedicated variables must win over the XDG fallbacks.
            .env("XDG_CACHE_HOME", self.dir.path().join("xdg-cache"))
            .env("XDG_STATE_HOME", self.dir.path().join("xdg-state"))
            .arg("--no-color");
        command
    }

    fn init(&self) {
        self.command().args(["init", "--no-git"]).assert().success();
    }

    fn new_skill(&self, name: &str) {
        self.command()
            .args(["new", name, "--description", "Only a test.  "])
            .assert()
            .success();
    }

    /// Drive the real dashboard through a pseudo-terminal, returning `false`
    /// when the environment has no `script` command to allocate one.
    fn tui(&self, keys: &[u8]) -> bool {
        use std::io::Write;
        use std::process::Stdio;
        use std::time::{Duration, Instant};

        if Command::new("script").arg("--version").output().is_err() {
            return false;
        }

        let binary = env!("CARGO_BIN_EXE_skillmgr");
        let mut child = Command::new("script")
            .args(["-qec", &format!("'{binary}'"), "/dev/null"])
            .env("HOME", &self.home)
            .env("SKILLMGR_ROOT", &self.root)
            .env("SKILLMGR_DEST", &self.dest)
            .env("SKILLMGR_CACHE_DIR", &self.cache)
            .env("SKILLMGR_STATE_DIR", &self.state)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn tui");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(keys)
            .expect("keys");

        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().expect("wait") {
                assert!(status.success(), "tui exited with {status}");
                break;
            }
            if started.elapsed() > Duration::from_secs(20) {
                let _ = child.kill();
                panic!("tui did not exit after {keys:?}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        true
    }
}

#[test]
fn environment_variables_redirect_all_paths_and_keep_links() {
    let fixture = EnvFixture::new();
    fixture.init();

    assert!(fixture.root.join("skills.toml").is_file());
    assert!(fixture.dest.is_dir());
    assert!(!fixture.home.join(".agents").exists());

    let names = ["alpha", "beta", "gamma", "delta", "epsilon"];
    for name in names {
        fixture.new_skill(name);
    }
    for name in names {
        assert!(fixture.dest.join(name).is_symlink(), "{name} is linked");
    }

    // Relinking one skill must never take another skill's link away.
    for _ in 0..3 {
        fixture
            .command()
            .args(["uninstall", "alpha"])
            .assert()
            .success();
        assert!(!fixture.dest.join("alpha").exists());

        fixture
            .command()
            .args(["install", "alpha"])
            .assert()
            .success();
        for name in names {
            assert!(
                fixture.dest.join(name).is_symlink(),
                "{name} survived installing alpha"
            );
        }
    }

    // A full install prunes only stale links. A detected-but-unregistered
    // skill is not stale: its managed directory still exists.
    let ghost = fixture.root.join("skills/authored/ghost");
    std::fs::create_dir_all(&ghost).expect("ghost");
    std::fs::write(
        ghost.join("SKILL.md"),
        "---\nname: ghost\ndescription: Unregistered.\n---\n",
    )
    .expect("ghost skill");
    std::os::unix::fs::symlink(&ghost, fixture.dest.join("ghost")).expect("ghost link");

    let dead = fixture
        .root
        .join("skills/authored/dead")
        .join("removed-elsewhere");
    std::os::unix::fs::symlink(&dead, fixture.dest.join("dead")).expect("dead link");

    fixture.command().arg("install").assert().success();
    assert!(fixture.dest.join("ghost").is_symlink());
    assert!(!fixture.dest.join("dead").exists());
    for name in names {
        assert!(fixture.dest.join(name).is_symlink());
    }
}

#[test]
fn the_tui_uses_the_environment_paths_and_installs_one_skill() {
    let fixture = EnvFixture::new();
    fixture.init();
    for name in ["alpha", "beta", "gamma"] {
        fixture.new_skill(name);
    }

    // No manage-skills entry yet, so the dashboard asks its setup question.
    // Declining writes the answer below SKILLMGR_STATE_DIR, proving the state
    // path is redirected too.
    if !fixture.tui(b"nq") {
        eprintln!("skipping the TUI assertions: `script` is not available");
        return;
    }
    assert!(fixture.state.join("state.toml").is_file());
    assert!(!fixture.home.join(".agents").exists());

    // Answering the question removes it from later sessions.
    fixture.new_skill("manage-skills");
    fixture
        .command()
        .args(["uninstall", "alpha"])
        .assert()
        .success();
    assert!(!fixture.dest.join("alpha").exists());

    // The first row is alpha; `i` links it and leaves the others alone.
    fixture.tui(b"iq");
    for name in ["alpha", "beta", "gamma"] {
        assert!(
            fixture.dest.join(name).is_symlink(),
            "{name} is linked after pressing i"
        );
    }
}
