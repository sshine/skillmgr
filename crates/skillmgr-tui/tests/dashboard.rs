use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use skillmgr_core::ops::{self, Context};
use skillmgr_core::paths::Paths;
use skillmgr_core::registry::Entry;

use skillmgr_tui::app::{App, Mode};
use skillmgr_tui::ui;

fn context(name: &str) -> (tempfile::TempDir, Context) {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::resolve(
        Some(dir.path().join(format!("{name}-root"))),
        Some(dir.path().join(format!("{name}-dest"))),
        Some(dir.path().join(format!("{name}-cache"))),
        Some(dir.path().join(format!("{name}-state"))),
    )
    .expect("paths");
    (dir, Context::new(paths, true))
}

fn online_context(name: &str) -> (tempfile::TempDir, Context) {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::resolve(
        Some(dir.path().join(format!("{name}-root"))),
        Some(dir.path().join(format!("{name}-dest"))),
        Some(dir.path().join(format!("{name}-cache"))),
        Some(dir.path().join(format!("{name}-state"))),
    )
    .expect("paths");
    (dir, Context::new(paths, false))
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key(c));
    }
}

fn wait_idle(app: &mut App) {
    let started = Instant::now();
    loop {
        app.tick();
        if !app.busy {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "background worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Create a local git repository with two skills and return its file URL.
fn upstream_repo(dir: &Path) -> String {
    let repo = dir.join("upstream");
    for skill in ["hello", "goodbye"] {
        let path = repo.join("skills").join(skill);
        std::fs::create_dir_all(&path).expect("skill dir");
        std::fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {skill}\ndescription: A demo skill.\n---\n"),
        )
        .expect("skill");
    }
    let commands: &[&[&str]] = &[
        &["init", "-q", "-b", "main"],
        &["config", "user.email", "test@example.com"],
        &["config", "user.name", "Test"],
        &["add", "."],
        &["commit", "-qm", "initial"],
    ];
    for args in commands {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(*args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?}");
    }
    format!("file://{}", repo.display())
}

fn seed_vendored(ctx: &Context, entries: &[(&str, &str)]) {
    let mut registry = ctx.load().expect("registry");
    for (name, repo) in entries {
        let dir = ctx.paths.skill_dir("vendored", name);
        std::fs::create_dir_all(&dir).expect("skill dir");
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Seeded.\n---\n"),
        )
        .expect("skill");
        registry
            .insert(Entry::vendored(
                *name,
                "vendored",
                *repo,
                *name,
                "main",
                "0123456789abcdef",
            ))
            .expect("insert");
    }
    registry.save().expect("save");
    ops::install_skills(ctx, &registry, &[], false, false).expect("install");
}

#[test]
fn init_screen_is_rendered_without_a_registry() {
    let (_dir, ctx) = context("init");
    let mut app = App::new(ctx);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, &mut app))
        .expect("draw");
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("No managed skill directory yet"));
    assert!(text.contains("initialize"));
    assert!(text.contains("skillmgr skill"));
}

#[test]
fn initializing_asks_whether_to_install_the_skillmgr_skill() {
    let (_dir, ctx) = context("init-prompt");
    let mut app = App::new(ctx);

    app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));

    assert_eq!(app.mode, Mode::Confirm);
    let prompt = &app.confirm.as_ref().expect("confirmation").prompt;
    assert!(prompt.contains("skillmgr skill"));
    assert!(prompt.contains(ops::SELF_SKILL_NAME));
}

#[test]
fn a_declined_wizard_question_is_not_asked_again() {
    let (_dir, ctx) = context("wizard-declined");
    ops::init(&ctx, true).expect("init");

    let mut app = App::new(ctx.clone());
    assert_eq!(app.mode, Mode::Confirm);
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);

    let app = App::new(ctx);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.confirm.is_none());
}

#[test]
fn the_wizard_skips_an_already_registered_skill() {
    let (_dir, ctx) = context("wizard-present");
    ops::init(&ctx, true).expect("init");
    let mut registry = ctx.load().expect("registry");
    ops::new_skill(&ctx, &mut registry, ops::SELF_SKILL_NAME, "Mine.", false).expect("new");

    let app = App::new(ctx);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.confirm.is_none());
}

#[test]
fn dashboard_renders_authored_skill_and_shortcuts() {
    let (_dir, ctx) = context("dashboard");
    ops::init(&ctx, true).expect("init");
    let mut registry = ctx.load().expect("registry");
    ops::new_skill(&ctx, &mut registry, "my-skill", "Mine.", false).expect("new");

    let mut app = App::new(ctx);
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, &mut app))
        .expect("draw");
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("my-skill"));
    assert!(text.contains("authored"));
    assert!(text.contains("add"));
    assert!(text.contains("update"));
    assert!(text.contains("filter"));
}

#[test]
fn a_faulty_repository_is_visible_and_marked() {
    let (dir, ctx) = context("faulty");
    ops::init(&ctx, true).expect("init");
    let mut registry = ctx.load().expect("registry");
    let missing = format!("file://{}", dir.path().join("missing").display());
    let entry = ops::declare_repo(
        &ctx,
        &ops::AddOptions {
            repo: missing,
            path: ".".to_string(),
            name: None,
            branch: None,
            rev: None,
            no_fetch: true,
            no_install: true,
        },
        &mut registry,
    )
    .expect("declare");
    ops::resolve_repo(&ctx, &mut registry, &entry.name, false, true);

    let mut app = App::new(ctx);
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, &mut app))
        .expect("draw");
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("faulty"));
    assert!(text.contains("missing"));
}

#[test]
fn installing_one_unlinked_skill_keeps_every_other_link() {
    let (_dir, ctx) = context("install-one");
    ops::init(&ctx, true).expect("init");
    let mut registry = ctx.load().expect("registry");
    ops::new_skill(&ctx, &mut registry, "alpha", "Alpha.", false).expect("alpha");
    ops::new_skill(&ctx, &mut registry, "beta", "Beta.", false).expect("beta");
    ops::uninstall_skills(&ctx, &registry, &["alpha".to_string()], false, false)
        .expect("uninstall");
    assert!(!ctx.paths.dest().join("alpha").exists());

    let mut app = App::new(ctx.clone());
    // Decline the wizard offer so keys reach the dashboard.
    if app.mode == Mode::Confirm {
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    }
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.selected_status().map(|status| status.name.as_str()),
        Some("alpha")
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));

    assert!(ctx.paths.dest().join("alpha").is_symlink());
    assert!(ctx.paths.dest().join("beta").is_symlink());
}

#[test]
fn adding_from_the_tui_vendors_without_linking_unless_the_toggle_is_on() {
    let (dir, ctx) = online_context("tui-add");
    let url = upstream_repo(dir.path());
    ops::init(&ctx, true).expect("init");
    let mut registry = ctx.load().expect("registry");
    ops::new_skill(&ctx, &mut registry, ops::SELF_SKILL_NAME, "Mine.", false).expect("self");

    let mut app = App::new(ctx.clone());
    assert_eq!(app.mode, Mode::Normal);

    // The add form starts with the link toggle off.
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(app.mode, Mode::Form);
    let link = app
        .form
        .as_ref()
        .expect("form")
        .fields
        .iter()
        .find(|field| field.label == "link")
        .expect("link field");
    assert!(!link.is_on(), "default is vendor only");

    type_text(&mut app, &url);
    press(&mut app, KeyCode::Tab);
    type_text(&mut app, "skills/goodbye");
    press(&mut app, KeyCode::Enter);
    wait_idle(&mut app);

    assert!(
        ctx.paths
            .skill_dir("vendored", "goodbye")
            .join("SKILL.md")
            .is_file()
    );
    assert!(
        !ctx.paths.dest().join("goodbye").exists(),
        "the default add does not link"
    );

    // Turn the toggle on for the second skill.
    press(&mut app, KeyCode::Char('a'));
    type_text(&mut app, &url);
    press(&mut app, KeyCode::Tab);
    type_text(&mut app, "skills/hello");
    for _ in 0..4 {
        press(&mut app, KeyCode::Tab);
    }
    assert_eq!(
        app.form.as_ref().expect("form").fields[5].label,
        "link",
        "the toggle is the active field"
    );
    press(&mut app, KeyCode::Char(' '));
    assert!(app.form.as_ref().expect("form").fields[5].is_on());
    press(&mut app, KeyCode::Enter);
    wait_idle(&mut app);

    assert!(
        ctx.paths.dest().join("hello").is_symlink(),
        "the toggle links the new skill"
    );
    assert!(!ctx.paths.dest().join("goodbye").exists());
}

#[test]
fn the_upstream_overview_lists_repositories_and_unlinks_one_as_a_group() {
    let (_dir, ctx) = context("tui-upstreams");
    ops::init(&ctx, true).expect("init");
    let mut registry = ctx.load().expect("registry");
    ops::new_skill(&ctx, &mut registry, ops::SELF_SKILL_NAME, "Mine.", false).expect("self");
    seed_vendored(
        &ctx,
        &[
            ("alpha", "git@github.com:sshine/skills.git"),
            ("beta", "git@github.com:sshine/skills.git"),
            ("gamma", "git@github.com:anthropics/skills.git"),
        ],
    );

    let mut app = App::new(ctx.clone());
    assert_eq!(app.mode, Mode::Normal);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");

    // The shortcut is visible in the keybar.
    terminal
        .draw(|frame| ui::draw(frame, &mut app))
        .expect("draw");
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("upstreams"));

    // The help screen documents it too.
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.mode, Mode::Help);
    terminal
        .draw(|frame| ui::draw(frame, &mut app))
        .expect("draw");
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("upstream overview"));
    press(&mut app, KeyCode::Char('x'));
    assert_eq!(app.mode, Mode::Normal);

    press(&mut app, KeyCode::Char('v'));
    assert_eq!(app.mode, Mode::Upstreams);
    assert_eq!(app.upstreams.len(), 2);
    assert_eq!(
        app.upstreams[0].repo,
        "git@github.com:anthropics/skills.git"
    );
    assert_eq!(app.upstreams[1].repo, "git@github.com:sshine/skills.git");

    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.upstream_selected, 1);

    terminal
        .draw(|frame| ui::draw(frame, &mut app))
        .expect("draw");
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("git@github.com:sshine/skills.git"));
    assert!(text.contains("SKILLS"));
    assert!(text.contains("LINKED"));

    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, Mode::Confirm);
    let prompt = &app.confirm.as_ref().expect("confirmation").prompt;
    assert!(prompt.contains("Unlink 2 skill(s)"));

    press(&mut app, KeyCode::Char('y'));
    wait_idle(&mut app);
    assert_eq!(
        app.mode,
        Mode::Upstreams,
        "unlinking returns to the overview"
    );
    assert!(!ctx.paths.dest().join("alpha").exists());
    assert!(!ctx.paths.dest().join("beta").exists());
    assert!(
        ctx.paths.dest().join("gamma").is_symlink(),
        "another upstream keeps its links"
    );
    assert!(
        ctx.paths
            .skill_dir("vendored", "alpha")
            .join("SKILL.md")
            .is_file(),
        "unlinking keeps the vendored files"
    );
    assert!(
        ctx.load().expect("registry").get("alpha").is_some(),
        "unlinking keeps the registry entries"
    );
}

fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
    let area = buffer.area();
    let mut text = String::new();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                text.push_str(cell.symbol());
            }
        }
        text.push('\n');
    }
    text
}
