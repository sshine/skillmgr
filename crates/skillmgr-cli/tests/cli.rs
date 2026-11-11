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

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("skillmgr").expect("binary");
        command
            .arg("--root")
            .arg(self.root())
            .arg("--dest")
            .arg(self.dest())
            .arg("--cache-dir")
            .arg(self.cache())
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
