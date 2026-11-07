use ratatui::Terminal;
use ratatui::backend::TestBackend;

use skillmgr_core::ops::{self, Context};
use skillmgr_core::paths::Paths;

use skillmgr_tui::app::App;
use skillmgr_tui::ui;

fn context(name: &str) -> (tempfile::TempDir, Context) {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::resolve(
        Some(dir.path().join(format!("{name}-root"))),
        Some(dir.path().join(format!("{name}-dest"))),
        Some(dir.path().join(format!("{name}-cache"))),
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
