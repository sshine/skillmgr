//! Dashboard state and key handling.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use promptui_core::input::Input;
use promptui_core::theme::Theme;
use promptui_core::widgets::notice::{Notice, NoticeLevel};
use promptui_core::widgets::spinner::{Spinner, SpinnerStyle};
use promptui_core::widgets::text_field::{TextField, TextFieldAction};

use skillmgr_core::ops::{self, Context, RemoveOptions};
use skillmgr_core::registry::Registry;
use skillmgr_core::status::SkillStatus;

use ratatui::crossterm::event::{KeyEvent, KeyEventKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Filter,
    Form,
    Confirm,
    Help,
    Init,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub label: &'static str,
    pub placeholder: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormKind {
    Add,
    New,
    Adopt,
}

#[derive(Debug, Clone)]
pub struct Form {
    pub title: &'static str,
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub active: usize,
    pub hint: &'static str,
}

#[derive(Debug, Clone)]
pub enum Submission {
    Add {
        repo: String,
        path: String,
        name: Option<String>,
        branch: Option<String>,
        rev: Option<String>,
    },
    New {
        name: String,
        description: String,
    },
    Adopt {
        name: String,
        from: Option<PathBuf>,
    },
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    Remove(String),
    UpdateAll,
    SyncAll,
    InstallAll,
    Uninstall(String),
}

#[derive(Debug, Clone)]
pub struct Confirm {
    pub prompt: String,
    pub action: ConfirmAction,
}

enum WorkerEvent {
    Status(Vec<SkillStatus>),
    Error(String),
}

pub struct App {
    pub ctx: Context,
    pub registry: Option<Registry>,
    pub statuses: Vec<SkillStatus>,
    pub selected: usize,
    pub mode: Mode,
    pub theme: Theme,
    pub notice: Notice,
    pub spinner: Spinner,
    pub busy: bool,
    pub filter: TextField,
    pub filter_text: String,
    pub form: Option<Form>,
    pub confirm: Option<Confirm>,
    pub should_quit: bool,
    worker: Option<Receiver<WorkerEvent>>,
    pending_edit: Option<PathBuf>,
}

impl App {
    pub fn new(ctx: Context) -> Self {
        let theme = Theme::default();
        let mut filter = TextField::new(theme);
        filter.set_min_height(1);
        filter.set_max_height(1);
        filter.set_padding(promptui_core::widgets::padding::Padding::uniform(0));
        filter.set_focused(false);

        let mut app = Self {
            ctx,
            registry: None,
            statuses: Vec::new(),
            selected: 0,
            mode: Mode::Normal,
            theme,
            notice: Notice::new(theme),
            spinner: Spinner::new(SpinnerStyle::BREATHING_DOT),
            busy: false,
            filter,
            filter_text: String::new(),
            form: None,
            confirm: None,
            should_quit: false,
            worker: None,
            pending_edit: None,
        };
        app.reload();
        app
    }

    pub fn reload(&mut self) {
        if !self.ctx.paths.registry().exists() {
            self.registry = None;
            self.statuses.clear();
            self.mode = Mode::Init;
            return;
        }

        match Registry::load(&self.ctx.paths.registry()) {
            Ok(registry) => {
                let selected_name = self.selected_status().map(|s| s.name.clone());
                match ops::list(&self.ctx, &registry, &[], false, false) {
                    Ok(statuses) => {
                        self.statuses = statuses;
                        self.registry = Some(registry);
                        if self.mode == Mode::Init {
                            self.mode = Mode::Normal;
                        }
                        if let Some(name) = selected_name
                            && let Some(index) = self
                                .visible_indices()
                                .into_iter()
                                .position(|i| self.statuses.get(i).map(|s| &s.name) == Some(&name))
                        {
                            self.selected = index;
                        }
                        self.clamp_selection();
                    }
                    Err(e) => self.error(format!("could not list skills: {e}")),
                }
            }
            Err(e) => self.error(format!("could not load registry: {e}")),
        }
    }

    pub fn refresh(&mut self) {
        if self.busy || self.registry.is_none() {
            return;
        }
        let ctx = self.ctx.clone();
        let (tx, rx) = mpsc::channel();
        self.busy = true;
        self.spinner.start();
        self.notice.show(NoticeLevel::Info, "checking upstream…");
        std::thread::spawn(move || {
            let result = Registry::load(&ctx.paths.registry())
                .and_then(|registry| ops::list(&ctx, &registry, &[], true, false));
            let event = match result {
                Ok(statuses) => WorkerEvent::Status(statuses),
                Err(e) => WorkerEvent::Error(e.to_string()),
            };
            let _ = tx.send(event);
        });
        self.worker = Some(rx);
    }

    pub fn tick(&mut self) {
        self.spinner.tick();
        self.notice.tick();
        self.poll_worker();
    }

    fn poll_worker(&mut self) {
        let Some(receiver) = &self.worker else {
            return;
        };
        match receiver.try_recv() {
            Ok(WorkerEvent::Status(statuses)) => {
                let selected_name = self.selected_status().map(|s| s.name.clone());
                self.statuses = statuses;
                if let Some(name) = selected_name
                    && let Some(index) = self
                        .visible_indices()
                        .into_iter()
                        .position(|i| self.statuses.get(i).map(|s| &s.name) == Some(&name))
                {
                    self.selected = index;
                }
                self.clamp_selection();
                self.busy = false;
                self.spinner.stop();
                self.worker = None;
                self.notice
                    .show(NoticeLevel::Info, "upstream status refreshed");
            }
            Ok(WorkerEvent::Error(message)) => {
                self.busy = false;
                self.spinner.stop();
                self.worker = None;
                self.notice.show(NoticeLevel::Error, message);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.busy = false;
                self.spinner.stop();
                self.worker = None;
            }
        }
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        let needle = self.filter_text.to_lowercase();
        self.statuses
            .iter()
            .enumerate()
            .filter(|(_, status)| {
                if needle.is_empty() {
                    return true;
                }
                let haystack = format!(
                    "{} {} {} {}",
                    status.name,
                    status.source,
                    status.description.as_deref().unwrap_or(""),
                    match &status.origin {
                        skillmgr_core::registry::Origin::Vendored { repo, path, .. } => {
                            format!("{repo} {path}")
                        }
                        skillmgr_core::registry::Origin::Authored => String::new(),
                    }
                )
                .to_lowercase();
                haystack.contains(&needle)
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub fn selected_status(&self) -> Option<&SkillStatus> {
        let visible = self.visible_indices();
        visible
            .get(self.selected)
            .and_then(|index| self.statuses.get(*index))
    }

    fn clamp_selection(&mut self) {
        let len = self.visible_indices().len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.visible_indices().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, len as isize - 1) as usize;
    }

    fn move_to(&mut self, end: bool) {
        let len = self.visible_indices().len();
        self.selected = if end { len.saturating_sub(1) } else { 0 };
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        let input = Input::from(key);

        match self.mode {
            Mode::Normal => self.handle_normal(input),
            Mode::Filter => self.handle_filter(input),
            Mode::Form => self.handle_form(input),
            Mode::Confirm => self.handle_confirm(input),
            Mode::Help => self.mode = Mode::Normal,
            Mode::Init => match input {
                Input::Char('i') => self.perform_init(),
                Input::Char('q') | Input::Esc => self.should_quit = true,
                _ => {}
            },
        }
    }

    fn handle_normal(&mut self, input: Input) {
        match input {
            Input::Char('q') | Input::Esc => self.should_quit = true,
            Input::Char('?') => self.mode = Mode::Help,
            Input::Char('/') => {
                self.mode = Mode::Filter;
                self.filter.set_focused(true);
                self.filter_text = self.filter.content();
            }
            Input::Char('r') => self.refresh(),
            Input::Char('j') | Input::Down => self.move_selection(1),
            Input::Char('k') | Input::Up => self.move_selection(-1),
            Input::Char('g') | Input::Home => self.move_to(false),
            Input::Char('G') | Input::End => self.move_to(true),
            Input::PageDown => self.move_selection(10),
            Input::PageUp => self.move_selection(-10),
            Input::Enter => self.show_selected_detail(),
            Input::Char('a') => self.open_form(FormKind::Add),
            Input::Char('n') => self.open_form(FormKind::New),
            Input::Char('o') => self.open_form(FormKind::Adopt),
            Input::Char('u') => self.perform_update(false),
            Input::Char('U') => self.confirm(
                "Update every vendored skill to its branch head?",
                ConfirmAction::UpdateAll,
            ),
            Input::Char('s') => self.perform_sync(false),
            Input::Char('S') => self.confirm(
                "Sync every vendored skill at its pinned revision?",
                ConfirmAction::SyncAll,
            ),
            Input::Char('i') => self.perform_install(false),
            Input::Char('I') => self.confirm(
                "Install every registered skill into the active directory?",
                ConfirmAction::InstallAll,
            ),
            Input::Char('x') => self.perform_uninstall_selected(),
            Input::Char('d') => self.confirm_remove(),
            Input::Char('e') => self.edit_selected(),
            _ => {}
        }
    }

    fn handle_filter(&mut self, input: Input) {
        if input == Input::Esc {
            self.filter.clear();
            self.filter_text.clear();
            self.filter.set_focused(false);
            self.mode = Mode::Normal;
            return;
        }
        match self.filter.input(input) {
            TextFieldAction::Submit(text) => {
                self.filter_text = text;
                self.filter.set_focused(false);
                self.mode = Mode::Normal;
                self.clamp_selection();
            }
            TextFieldAction::Edited | TextFieldAction::Ignored => {
                self.filter_text = self.filter.content();
                self.clamp_selection();
            }
        }
    }

    fn handle_form(&mut self, input: Input) {
        if input == Input::Esc {
            self.form = None;
            self.mode = Mode::Normal;
            return;
        }
        let Some(form) = &mut self.form else {
            self.mode = Mode::Normal;
            return;
        };

        match input {
            Input::Tab => {
                form.active = (form.active + 1) % form.fields.len();
            }
            Input::Enter => {
                let submission = match form.submission() {
                    Ok(submission) => submission,
                    Err(message) => {
                        self.notice.show(NoticeLevel::Warn, message);
                        return;
                    }
                };
                self.form = None;
                self.mode = Mode::Normal;
                self.submit(submission);
            }
            Input::Backspace => {
                form.fields[form.active].value.pop();
            }
            Input::Delete => {
                form.fields[form.active].value.clear();
            }
            Input::Char(c) => {
                form.fields[form.active].value.push(c);
            }
            _ => {}
        }
    }

    fn handle_confirm(&mut self, input: Input) {
        match input {
            Input::Char('y') | Input::Char('Y') | Input::Enter => {
                let Some(confirm) = self.confirm.take() else {
                    self.mode = Mode::Normal;
                    return;
                };
                self.mode = Mode::Normal;
                self.run_confirm(confirm.action);
            }
            Input::Char('n') | Input::Char('N') | Input::Esc => {
                self.confirm = None;
                self.mode = Mode::Normal;
            }
            _ => {}
        }
    }

    fn open_form(&mut self, kind: FormKind) {
        self.form = Some(Form::new(kind));
        self.mode = Mode::Form;
    }

    fn confirm(&mut self, prompt: impl Into<String>, action: ConfirmAction) {
        self.confirm = Some(Confirm {
            prompt: prompt.into(),
            action,
        });
        self.mode = Mode::Confirm;
    }

    fn confirm_remove(&mut self) {
        let Some(status) = self.selected_status() else {
            self.notice.show(NoticeLevel::Warn, "no skill selected");
            return;
        };
        self.confirm(
            format!("Remove '{}'?", status.name),
            ConfirmAction::Remove(status.name.clone()),
        );
    }

    fn submit(&mut self, submission: Submission) {
        let result = (|| -> skillmgr_core::Result<String> {
            match submission {
                Submission::Add {
                    repo,
                    path,
                    name,
                    branch,
                    rev,
                } => {
                    let mut registry = self.ctx.load()?;
                    let outcome = ops::add(
                        &self.ctx,
                        ops::AddOptions {
                            repo,
                            path,
                            name,
                            branch,
                            rev,
                            no_fetch: false,
                            no_install: false,
                        },
                        &mut registry,
                    )?;
                    Ok(format!(
                        "added {} at {}",
                        outcome.entry.name,
                        outcome
                            .entry
                            .rev()
                            .map(|rev| rev.chars().take(9).collect::<String>())
                            .unwrap_or_else(|| "-".to_string())
                    ))
                }
                Submission::New { name, description } => {
                    let mut registry = self.ctx.load()?;
                    ops::new_skill(&self.ctx, &mut registry, &name, &description, false)?;
                    Ok(format!("created authored skill {name}"))
                }
                Submission::Adopt { name, from } => {
                    let mut registry = self.ctx.load()?;
                    ops::adopt(&self.ctx, &mut registry, &name, from.as_deref(), false)?;
                    Ok(format!("adopted {name}"))
                }
            }
        })();

        match result {
            Ok(message) => {
                self.notice.show(NoticeLevel::Info, message);
                self.reload();
                self.refresh();
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    fn run_confirm(&mut self, action: ConfirmAction) {
        let result = (|| -> skillmgr_core::Result<String> {
            let mut registry = self.ctx.load()?;
            match action {
                ConfirmAction::Remove(name) => {
                    ops::remove(
                        &self.ctx,
                        &mut registry,
                        std::slice::from_ref(&name),
                        RemoveOptions {
                            keep_files: false,
                            delete_files: false,
                            no_uninstall: false,
                        },
                    )?;
                    Ok(format!("removed {name}"))
                }
                ConfirmAction::UpdateAll => {
                    let outcomes = ops::update(
                        &self.ctx,
                        &mut registry,
                        &[],
                        ops::UpdateOptions {
                            to: None,
                            branch: None,
                            no_fetch: false,
                            dry_run: false,
                            no_install: false,
                        },
                    )?;
                    Ok(format!("updated {} skill(s)", outcomes.len()))
                }
                ConfirmAction::SyncAll => {
                    let outcomes = ops::sync(&self.ctx, &mut registry, &[], false, false)?;
                    Ok(format!(
                        "synced {} skill(s)",
                        outcomes
                            .iter()
                            .filter(|o| o.state != skillmgr_core::vendor::VendoredState::Current)
                            .count()
                    ))
                }
                ConfirmAction::InstallAll => {
                    let report = ops::install_skills(&self.ctx, &registry, &[], false, true)?;
                    Ok(format!(
                        "linked {}, already {}, pruned {}",
                        report.linked.len(),
                        report.already.len(),
                        report.pruned.len()
                    ))
                }
                ConfirmAction::Uninstall(name) => {
                    let report = ops::uninstall_skills(
                        &self.ctx,
                        &registry,
                        std::slice::from_ref(&name),
                        false,
                        false,
                    )?;
                    Ok(format!("unlinked {}", report.removed.len()))
                }
            }
        })();

        match result {
            Ok(message) => {
                self.notice.show(NoticeLevel::Info, message);
                self.reload();
                self.refresh();
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    fn perform_update(&mut self, all: bool) {
        let Some(name) = self.selected_name() else {
            self.notice.show(NoticeLevel::Warn, "no skill selected");
            return;
        };
        let names = if all { Vec::new() } else { vec![name] };
        let result = (|| -> skillmgr_core::Result<String> {
            let mut registry = self.ctx.load()?;
            let outcomes = ops::update(
                &self.ctx,
                &mut registry,
                &names,
                ops::UpdateOptions {
                    to: None,
                    branch: None,
                    no_fetch: false,
                    dry_run: false,
                    no_install: false,
                },
            )?;
            Ok(format!("updated {} skill(s)", outcomes.len()))
        })();
        self.finish(result);
    }

    fn perform_sync(&mut self, all: bool) {
        let Some(name) = self.selected_name() else {
            self.notice.show(NoticeLevel::Warn, "no skill selected");
            return;
        };
        let names = if all { Vec::new() } else { vec![name] };
        let result = (|| -> skillmgr_core::Result<String> {
            let mut registry = self.ctx.load()?;
            let outcomes = ops::sync(&self.ctx, &mut registry, &names, false, false)?;
            Ok(format!(
                "synced {} skill(s)",
                outcomes
                    .iter()
                    .filter(|o| o.state != skillmgr_core::vendor::VendoredState::Current)
                    .count()
            ))
        })();
        self.finish(result);
    }

    fn perform_install(&mut self, all: bool) {
        let Some(name) = self.selected_name() else {
            self.notice.show(NoticeLevel::Warn, "no skill selected");
            return;
        };
        let names = if all { Vec::new() } else { vec![name] };
        let result = (|| -> skillmgr_core::Result<String> {
            let registry = self.ctx.load()?;
            let report = ops::install_skills(&self.ctx, &registry, &names, false, true)?;
            if report.conflicts.is_empty() {
                Ok(format!("linked {}", report.linked.len()))
            } else {
                Ok(format!("{} conflict(s)", report.conflicts.len()))
            }
        })();
        self.finish(result);
    }

    fn perform_uninstall_selected(&mut self) {
        let Some(name) = self.selected_name() else {
            self.notice.show(NoticeLevel::Warn, "no skill selected");
            return;
        };
        let result = (|| -> skillmgr_core::Result<String> {
            let registry = self.ctx.load()?;
            let report = ops::uninstall_skills(
                &self.ctx,
                &registry,
                std::slice::from_ref(&name),
                false,
                false,
            )?;
            Ok(format!("unlinked {}", report.removed.len()))
        })();
        self.finish(result);
    }

    fn finish(&mut self, result: skillmgr_core::Result<String>) {
        match result {
            Ok(message) => {
                self.notice.show(NoticeLevel::Info, message);
                self.reload();
                self.refresh();
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    fn perform_init(&mut self) {
        match ops::init(&self.ctx, false) {
            Ok(_) => {
                self.notice
                    .show(NoticeLevel::Info, "managed directory initialized");
                self.reload();
                self.refresh();
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    fn edit_selected(&mut self) {
        let Some(status) = self.selected_status() else {
            self.notice.show(NoticeLevel::Warn, "no skill selected");
            return;
        };
        let Some(registry) = &self.registry else {
            return;
        };
        match ops::skill_file(&self.ctx, registry, &status.name) {
            Ok(path) => self.pending_edit = Some(path),
            Err(e) => self.error(e.to_string()),
        }
    }

    fn show_selected_detail(&mut self) {
        if let Some(status) = self.selected_status() {
            let message = match &status.note {
                Some(note) => format!("{}: {note}", status.name),
                None => format!(
                    "{} · {} · {}",
                    status.name,
                    status.source,
                    status.behind_label()
                ),
            };
            self.notice.show(NoticeLevel::Info, message);
        }
    }

    fn selected_name(&self) -> Option<String> {
        self.selected_status().map(|s| s.name.clone())
    }

    fn error(&mut self, message: String) {
        self.notice.show(NoticeLevel::Error, message);
    }

    /// Show an error from the run loop or an external command.
    pub fn notify_error(&mut self, message: impl Into<String>) {
        self.error(message.into());
    }

    pub fn take_edit(&mut self) -> Option<PathBuf> {
        self.pending_edit.take()
    }
}

impl Form {
    fn new(kind: FormKind) -> Self {
        match kind {
            FormKind::Add => Self {
                title: "Vendor a skill",
                kind,
                fields: vec![
                    Field {
                        label: "repo",
                        placeholder: "owner/repo or git URL",
                        value: String::new(),
                    },
                    Field {
                        label: "path",
                        placeholder: ".",
                        value: String::new(),
                    },
                    Field {
                        label: "name",
                        placeholder: "optional",
                        value: String::new(),
                    },
                    Field {
                        label: "branch",
                        placeholder: "remote HEAD",
                        value: String::new(),
                    },
                    Field {
                        label: "rev",
                        placeholder: "branch head",
                        value: String::new(),
                    },
                ],
                active: 0,
                hint: "Tab next · Enter vendor · Esc cancel",
            },
            FormKind::New => Self {
                title: "New authored skill",
                kind,
                fields: vec![
                    Field {
                        label: "name",
                        placeholder: "lowercase-hyphenated",
                        value: String::new(),
                    },
                    Field {
                        label: "description",
                        placeholder: "what and when",
                        value: String::new(),
                    },
                ],
                active: 0,
                hint: "Tab next · Enter create · Esc cancel",
            },
            FormKind::Adopt => Self {
                title: "Adopt an active skill",
                kind,
                fields: vec![
                    Field {
                        label: "name",
                        placeholder: "skill name",
                        value: String::new(),
                    },
                    Field {
                        label: "from",
                        placeholder: "~/.agents/skills/<name>",
                        value: String::new(),
                    },
                ],
                active: 0,
                hint: "Tab next · Enter adopt · Esc cancel",
            },
        }
    }

    fn submission(&self) -> std::result::Result<Submission, String> {
        let value = |index: usize| self.fields[index].value.trim().to_string();
        match self.kind {
            FormKind::Add => {
                let repo = value(0);
                if repo.is_empty() {
                    return Err("repo is required".to_string());
                }
                let path = {
                    let path = value(1);
                    if path.is_empty() {
                        ".".to_string()
                    } else {
                        path
                    }
                };
                let name = non_empty(value(2));
                let branch = non_empty(value(3));
                let rev = non_empty(value(4));
                Ok(Submission::Add {
                    repo,
                    path,
                    name,
                    branch,
                    rev,
                })
            }
            FormKind::New => {
                let name = value(0);
                if name.is_empty() {
                    return Err("name is required".to_string());
                }
                Ok(Submission::New {
                    name,
                    description: value(1),
                })
            }
            FormKind::Adopt => {
                let name = value(0);
                if name.is_empty() {
                    return Err("name is required".to_string());
                }
                let from = non_empty(value(1)).map(PathBuf::from);
                Ok(Submission::Adopt { name, from })
            }
        }
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}
