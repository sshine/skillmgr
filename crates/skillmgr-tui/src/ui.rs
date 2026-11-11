//! Ratatui rendering for the skillmgr dashboard.

use promptui_core::theme::Theme;
use promptui_core::widgets::border_box::{BorderBox, BorderStyle};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};

use skillmgr_core::git::Upstream;
use skillmgr_core::install::LinkState;
use skillmgr_core::registry::Origin;
use skillmgr_core::status::SkillStatus;

use crate::app::{App, Form, Mode};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if app.mode == Mode::Init {
        draw_init(frame, app, area);
        frame.render_widget(&app.notice, area);
        return;
    }

    let filter_height = if app.mode == Mode::Filter { 1 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(filter_height),
            Constraint::Min(6),
            Constraint::Length(9),
            Constraint::Length(2),
        ])
        .split(area);

    draw_header(frame, app, chunks[0]);
    if app.mode == Mode::Filter {
        app.filter.set_focused(true);
        frame.render_widget(&app.filter, chunks[1]);
    }
    draw_table(frame, app, chunks[2]);
    draw_detail(frame, app, chunks[3]);
    draw_keybar(frame, app, chunks[4]);

    match app.mode {
        Mode::Help => draw_help(frame, app, area),
        Mode::Form => {
            if let Some(form) = &app.form {
                draw_form(frame, app, form, area);
            }
        }
        Mode::Confirm => {
            if let Some(confirm) = &app.confirm {
                draw_confirm(frame, app, &confirm.prompt, area);
            }
        }
        _ => {}
    }

    frame.render_widget(&app.notice, area);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled(
            "skillmgr",
            Style::default()
                .fg(app.theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(
            app.ctx.paths.root().display().to_string(),
            Style::default().fg(app.theme.text_muted),
        ),
        Span::styled("  →  ", Style::default().fg(app.theme.border)),
        Span::styled(
            app.ctx.paths.dest().display().to_string(),
            Style::default().fg(app.theme.text_muted),
        ),
    ];
    if app.ctx.offline {
        spans.push(Span::styled(
            "  offline",
            Style::default().fg(app.theme.warning),
        ));
    }
    if let Some(network) = &app.network {
        let meter = network_meter(network.frame, 12);
        spans.push(Span::styled(
            format!(
                "  {} {} [{}] {}s",
                app.spinner.frame(),
                network.label,
                meter,
                network.elapsed_secs()
            ),
            Style::default().fg(app.theme.info),
        ));
    } else if app.busy {
        spans.push(Span::styled(
            format!("  {} working", app.spinner.frame()),
            Style::default().fg(app.theme.info),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// A moving block in a trough, indicating indeterminate network progress.
fn network_meter(frame: u64, width: usize) -> String {
    let cycle = width + 6;
    let head = (frame as usize) % cycle;
    let mut meter = String::with_capacity(width);
    for position in 0..width {
        let block = position < head.min(width) && position + 4 >= head;
        meter.push(if block { '█' } else { '░' });
    }
    meter
}

fn draw_table(frame: &mut Frame, app: &App, area: Rect) {
    let visible = app.visible_indices();
    if visible.is_empty() {
        let text = if app.statuses.is_empty() {
            "No skills yet. Press a to vendor one, n to author one, o to adopt an existing one."
        } else {
            "No skill matches the filter."
        };
        frame.render_widget(
            Paragraph::new(text)
                .style(Style::default().fg(app.theme.text_muted))
                .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let header = Row::new([
        Cell::from("NAME"),
        Cell::from("ORIGIN"),
        Cell::from("LINK"),
        Cell::from("REV"),
        Cell::from("BEHIND"),
        Cell::from("PATH"),
    ])
    .style(
        Style::default()
            .fg(app.theme.text_muted)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = visible
        .iter()
        .filter_map(|index| app.statuses.get(*index))
        .map(|status| row_for(status, app.theme))
        .collect();

    let widths = [
        Constraint::Length(30),
        Constraint::Length(12),
        Constraint::Length(13),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Min(16),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(
            Style::default()
                .bg(app.theme.element_bg)
                .fg(app.theme.primary)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");

    let mut state = TableState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn row_for(status: &SkillStatus, theme: Theme) -> Row<'static> {
    let origin = if !status.registered {
        ("unregistered", theme.warning)
    } else if status.is_repo() {
        if status.repo_error().is_some() {
            ("faulty", theme.error)
        } else if status.repo_status().is_some() {
            ("empty", theme.warning)
        } else {
            ("repo", theme.warning)
        }
    } else if status.is_vendored() {
        ("vendored", theme.secondary)
    } else {
        ("authored", theme.success)
    };

    let (link, link_color) = if status.is_repo() {
        ("-", theme.text_muted)
    } else {
        match &status.link {
            LinkState::Linked => ("linked", theme.success),
            LinkState::Unlinked => ("unlinked", theme.warning),
            LinkState::Broken => ("broken", theme.error),
            LinkState::ForeignDir => ("foreign-dir", theme.error),
            LinkState::ForeignSymlink => ("foreign-link", theme.error),
        }
    };

    let rev = match &status.origin {
        Origin::Vendored { rev, .. } => rev.chars().take(7).collect::<String>(),
        Origin::Repo { .. } | Origin::Authored => "-".to_string(),
    };

    let behind = status.behind_label();
    let behind_color = if behind == "0/0" || behind == "-" {
        theme.text_muted
    } else if behind.ends_with("/0") {
        theme.warning
    } else {
        theme.error
    };

    let path = match &status.origin {
        Origin::Vendored { repo, path, .. } | Origin::Repo { repo, path, .. } => {
            if path == "." {
                repo.clone()
            } else {
                format!("{repo}:{path}")
            }
        }
        Origin::Authored => status.source.clone(),
    };

    Row::new([
        Cell::from(status.name.clone()),
        Cell::from(Span::styled(origin.0, Style::default().fg(origin.1))),
        Cell::from(Span::styled(link, Style::default().fg(link_color))),
        Cell::from(rev),
        Cell::from(Span::styled(behind, Style::default().fg(behind_color))),
        Cell::from(Span::styled(path, Style::default().fg(theme.text_muted))),
    ])
}

fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let Some(status) = app.selected_status() else {
        let block = BorderBox::new(app.theme, BorderStyle::Rounded).title("details".to_string());
        block.render_chrome(area, frame.buffer_mut());
        return;
    };

    let title = format!("{} · {}", status.name, origin_label(status));
    let block = BorderBox::new(app.theme, BorderStyle::Rounded)
        .title(title)
        .padding(promptui_core::widgets::padding::Padding::symmetric(1, 0));
    let inner = block.inner_area(area);
    block.render_chrome(area, frame.buffer_mut());

    let paragraph = Paragraph::new(detail_text(status, app.theme)).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn detail_text(status: &SkillStatus, theme: Theme) -> Text<'static> {
    let mut lines = Vec::new();
    let description = status
        .description
        .clone()
        .unwrap_or_else(|| "(no description in SKILL.md)".to_string());
    lines.push(Line::from(Span::styled(
        description,
        Style::default().fg(theme.text),
    )));
    lines.push(Line::from(""));

    let mut facts: Vec<(String, String)> = Vec::new();
    facts.push(("source".to_string(), status.source.clone()));
    facts.push(("link".to_string(), link_label(&status.link).to_string()));
    if let Some((repo, path, branch, rev)) = status.repo_field() {
        facts.push(("repo".to_string(), repo.to_string()));
        facts.push(("path".to_string(), path.to_string()));
        facts.push(("branch".to_string(), branch.to_string()));
        facts.push(("rev".to_string(), rev.to_string()));
        facts.push((
            "upstream".to_string(),
            upstream_label(status.upstream.as_ref()),
        ));
    } else if let Some((repo, path)) = status.repo_target() {
        facts.push(("repo".to_string(), repo.to_string()));
        facts.push(("path".to_string(), path.to_string()));
        facts.push(("state".to_string(), "repository declaration".to_string()));
    }
    if let Some(version) = &status.version {
        facts.push(("version".to_string(), version.clone()));
    }
    facts.push((
        "managed".to_string(),
        status.managed_path.display().to_string(),
    ));
    if let Some(note) = &status.note {
        facts.push(("note".to_string(), note.clone()));
    }

    for (key, value) in facts {
        lines.push(Line::from(vec![
            Span::styled(format!("{key:<9}"), Style::default().fg(theme.text_muted)),
            Span::styled(value, Style::default().fg(theme.text)),
        ]));
    }

    Text::from(lines)
}

fn draw_keybar(frame: &mut Frame, app: &App, area: Rect) {
    let lines = if app.mode == Mode::Filter {
        vec![Line::from(vec![
            Span::styled("Enter", key_style(app.theme)),
            Span::raw(" apply filter   "),
            Span::styled("Esc", key_style(app.theme)),
            Span::raw(" clear filter"),
        ])]
    } else {
        vec![
            Line::from(vec![
                Span::raw(" "),
                Span::styled("a", key_style(app.theme)),
                Span::raw(" add   "),
                Span::styled("n", key_style(app.theme)),
                Span::raw(" new   "),
                Span::styled("u", key_style(app.theme)),
                Span::raw(" update   "),
                Span::styled("U", key_style(app.theme)),
                Span::raw(" all   "),
                Span::styled("s", key_style(app.theme)),
                Span::raw(" sync   "),
                Span::styled("S", key_style(app.theme)),
                Span::raw(" all   "),
                Span::styled("i", key_style(app.theme)),
                Span::raw(" install   "),
                Span::styled("I", key_style(app.theme)),
                Span::raw(" all"),
            ]),
            Line::from(vec![
                Span::raw(" "),
                Span::styled("x", key_style(app.theme)),
                Span::raw(" uninstall   "),
                Span::styled("o", key_style(app.theme)),
                Span::raw(" adopt   "),
                Span::styled("d", key_style(app.theme)),
                Span::raw(" remove   "),
                Span::styled("e", key_style(app.theme)),
                Span::raw(" edit   "),
                Span::styled("r", key_style(app.theme)),
                Span::raw(" refresh   "),
                Span::styled("/", key_style(app.theme)),
                Span::raw(" filter   "),
                Span::styled("?", key_style(app.theme)),
                Span::raw(" help   "),
                Span::styled("q", key_style(app.theme)),
                Span::raw(" quit"),
            ]),
        ]
    };
    frame.render_widget(Paragraph::new(lines), area);
}

fn key_style(theme: Theme) -> Style {
    Style::default()
        .fg(theme.primary)
        .add_modifier(Modifier::BOLD)
}

fn draw_help(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(area, 78, 20);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.primary))
        .title(" skillmgr keys ");
    let text = vec![
        Line::from(""),
        Line::from("  a          vendor a new skill          n   new authored skill"),
        Line::from("  u / U      update selected / all        s / S   sync selected / all"),
        Line::from("  i / I      link selected / all          x   unlink selected"),
        Line::from("  d          remove selected              o   adopt active skill"),
        Line::from("  e          edit SKILL.md                r   refresh status"),
        Line::from("  /          filter                       ?   this help"),
        Line::from("  j/k ↑/↓    move                         g/G top / bottom"),
        Line::from("  Enter      show a summary               q   quit"),
        Line::from(""),
        Line::from(
            "  Vendored = copied from upstream. Authored = written in the managed directory.",
        ),
        Line::from("  BEHIND is repo/path: commits on the branch, and those touching the skill."),
        Line::from("  A faulty repo failed to resolve; press s or u on it to retry."),
        Line::from(""),
        Line::from("  Press any key to close."),
    ];
    frame.render_widget(Paragraph::new(text).block(block), rect);
}

fn draw_form(frame: &mut Frame, app: &App, form: &Form, area: Rect) {
    let height = (form.fields.len() as u16 + 6).min(area.height);
    let rect = centered(area, 74, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.primary))
        .title(format!(" {} ", form.title));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let mut lines = Vec::new();
    for (index, field) in form.fields.iter().enumerate() {
        let active = index == form.active;
        let cursor = if active { "›" } else { " " };
        let value = if field.value.is_empty() {
            Span::styled(field.placeholder, Style::default().fg(app.theme.text_muted))
        } else {
            Span::styled(field.value.clone(), Style::default().fg(app.theme.text))
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{cursor} {:<12}", field.label),
                Style::default().fg(if active {
                    app.theme.primary
                } else {
                    app.theme.text_muted
                }),
            ),
            value,
            if active {
                Span::styled("_", Style::default().fg(app.theme.primary))
            } else {
                Span::raw("")
            },
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        form.hint,
        Style::default().fg(app.theme.text_muted),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_confirm(frame: &mut Frame, app: &App, prompt: &str, area: Rect) {
    let rect = centered(area, 60, 7);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.warning))
        .title(" confirm ");
    let text = vec![
        Line::from(""),
        Line::from(prompt.to_string()),
        Line::from(""),
        Line::from(vec![
            Span::styled("y", key_style(app.theme)),
            Span::raw(" yes     "),
            Span::styled("n", key_style(app.theme)),
            Span::raw(" no"),
        ]),
    ];
    frame.render_widget(Paragraph::new(text).block(block), rect);
}

fn draw_init(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(area, 72, 10);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.primary))
        .title(" skillmgr ");
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "No managed skill directory yet.",
            Style::default()
                .fg(app.theme.text)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("  {}", app.ctx.paths.root().display())),
        Line::from(""),
        Line::from(vec![
            Span::styled("i", key_style(app.theme)),
            Span::raw(" initialize with skills.toml and a git repository     "),
            Span::styled("q", key_style(app.theme)),
            Span::raw(" quit"),
        ]),
    ];
    frame.render_widget(Paragraph::new(text).block(block), rect);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

fn origin_label(status: &SkillStatus) -> &'static str {
    if !status.registered {
        "unregistered"
    } else if status.is_repo() {
        if status.repo_error().is_some() {
            "faulty"
        } else if status.repo_status().is_some() {
            "empty"
        } else {
            "repo"
        }
    } else if status.is_vendored() {
        "vendored"
    } else {
        "authored"
    }
}

fn link_label(link: &LinkState) -> &'static str {
    match link {
        LinkState::Linked => "linked",
        LinkState::Unlinked => "unlinked",
        LinkState::Broken => "broken",
        LinkState::ForeignDir => "foreign directory",
        LinkState::ForeignSymlink => "foreign symlink",
    }
}

fn upstream_label(upstream: Option<&Upstream>) -> String {
    match upstream {
        Some(Upstream::Current) => "current".to_string(),
        Some(Upstream::Behind { repo, path }) => format!("{repo}/{path} behind"),
        Some(Upstream::Diverged) => "diverged".to_string(),
        Some(Upstream::PathRemoved { repo }) => format!("{repo} behind, path removed"),
        Some(Upstream::Unreachable) => "unreachable".to_string(),
        Some(Upstream::Unknown(reason)) => format!("unknown ({reason})"),
        None => "not checked".to_string(),
    }
}
