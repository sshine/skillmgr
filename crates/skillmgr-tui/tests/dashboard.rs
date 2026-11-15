use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use skillmgr_core::ops::{self, Context};
use skillmgr_core::paths::Paths;

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
