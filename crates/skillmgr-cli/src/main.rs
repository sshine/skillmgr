//! # skillmgr
//!
//! Manage agent skills as a mixture of vendored upstream copies and skills authored
//! in the managed directory.
//!
//! `skillmgr` keeps a managed home at `~/.agents/managed-skills/` with a
//! `skills.toml` registry and two conventional source directories:
//!
//! ```text
//! skills/vendored/<name>/   copies pinned to an upstream git commit
//! skills/authored/<name>/   skills written here
//! ```
//!
//! Active skills are symlinked into `~/.agents/skills/`. Vendored skills can be
//! synced at their pinned revision or updated to the upstream branch head, with a
//! heuristic that reports both how many commits the repository is ahead and how
//! many of those commits touch the skill's directory.
//!
//! ## Usage
//!
//! ```text
//! $ skillmgr --help
//! Manage agent skills as a mixture of vendored upstream copies and skills authored
//! in the managed directory.
//!
//! Usage: skillmgr [OPTIONS] [COMMAND]
//!
//! Commands:
//!   init        Create the managed directory and an empty skills.toml
//!   add         Vendor a skill from a git repository
//!   remove      Unregister skills, remove their links, and optionally delete files
//!   sync        Re-vendor skills at their pinned revisions
//!   update      Move pins to the branch head and re-vendor
//!   list        List registered and unregistered skills
//!   status      List skills with upstream behind counts
//!   install     Symlink registered skills into the active directory
//!   uninstall   Remove managed symlinks from the active directory
//!   new         Scaffold an authored skill
//!   register    Register an existing directory below skills/
//!   adopt       Move an existing active skill directory into skills/authored/
//!   edit        Open a skill's SKILL.md in $EDITOR
//!   completion  Print a shell completion script
//!   tui         Open the interactive dashboard
//! ```
//!
//! Run without a command in a terminal to open the dashboard. The TUI uses the same
//! operations as the subcommands and keeps every shortcut visible in its keybar.
//!
//! ## Registry
//!
//! ```toml
//! schema = 1
//!
//! [skills.agentic-playwright-nix]
//! source = "vendored"
//! repo = "sshine/skills"
//! path = "agentic-playwright-nix"
//! branch = "main"
//! rev = "84a1a07..."
//!
//! [skills.blog-post]
//! source = "authored"
//! ```
//!
//! `source` names the directory below `skills/`; `vendored` and `authored` are
//! conventions, not hardcoded behaviour. An entry with `repo`, `path`, `branch` and
//! `rev` is vendored. Anything else is authored.
//!
//! ## Origin cache
//!
//! Upstream repositories are bare mirrors under
//! `$XDG_CACHE_HOME/skillmgr/origins/`, named by transport, host, transport user,
//! namespace and repository:
//!
//! ```text
//! git+https/github.com/anonymous/sshine/skills.git
//! git+ssh/github.com/git/sshine/skills.git
//! ```
//!
//! The cache is disposable. Vendored copies live in the managed directory and are
//! the source of truth; deleting the cache never removes an installed skill.
//!
//! ## Installation
//!
//! ```text
//! nix run github:sshine/skillmgr
//! nix profile install github:sshine/skillmgr
//! cargo install --path crates/skillmgr-cli
//! ```
//!
//! For NixOS, add the flake as an input and use `pkgs.skillmgr` from the overlay, or
//! the prebuilt `skillmgr-bin` package from the release.
//!
//! `git` must be on `PATH`; the Nix package wraps the binary with it.

mod args;
mod color;
mod output;

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};

use args::{Args, Command as CliCommand, Shell};
use color::Color;
use skillmgr_core::git::Upstream;
use skillmgr_core::ops::{self, AddOptions, Context, RemoveOptions};
use skillmgr_core::vendor::VendoredState;
use skillmgr_core::{Error, Paths, Result};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let args = Args::parse();
    let paths = Paths::resolve(args.root, args.dest, args.cache_dir)?;
    let ctx = Context::new(paths, args.offline);
    let color = Color::new(args.no_color);

    match args.command {
        None => {
            if std::io::stdout().is_terminal() {
                skillmgr_tui::run(ctx)?;
                Ok(ExitCode::SUCCESS)
            } else {
                Args::command().print_help().map_err(Error::Io)?;
                println!();
                Ok(ExitCode::SUCCESS)
            }
        }
        Some(command) => dispatch(&ctx, &color, args.verbose, command),
    }
}

fn dispatch(ctx: &Context, color: &Color, verbose: bool, command: CliCommand) -> Result<ExitCode> {
    match command {
        CliCommand::Init { no_git } => {
            let report = ops::init(ctx, no_git)?;
            println!("managed directory: {}", ctx.paths.root().display());
            if report.registry_created {
                println!("  created {}", ctx.paths.registry().display());
            } else {
                println!("  kept {}", ctx.paths.registry().display());
            }
            if report.git_initialized {
                println!("  initialized git repository");
            }
            Ok(ExitCode::SUCCESS)
        }

        CliCommand::Add {
            repo,
            path,
            name,
            branch,
            rev,
            no_fetch,
            no_install,
        } => {
            let mut registry = ctx.load()?;
            let outcome = ops::add(
                ctx,
                AddOptions {
                    repo,
                    path,
                    name,
                    branch,
                    rev,
                    no_fetch,
                    no_install,
                },
                &mut registry,
            )?;
            print_add(&outcome, color);
            if let Some(report) = &outcome.install {
                output::print_install_report(report, color);
                if !report.is_ok() {
                    return Ok(ExitCode::from(1));
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        CliCommand::Remove {
            names,
            keep_files,
            delete_files,
            no_uninstall,
        } => {
            let mut registry = ctx.load()?;
            let outcomes = ops::remove(
                ctx,
                &mut registry,
                &names,
                RemoveOptions {
                    keep_files,
                    delete_files,
                    no_uninstall,
                },
            )?;
            for outcome in outcomes {
                if outcome.files_deleted {
                    println!(
                        "  {} {} (files deleted)",
                        color.red("removed"),
                        outcome.name
                    );
                } else {
                    println!(
                        "  {} {} (files kept)",
                        color.yellow("removed"),
                        outcome.name
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        CliCommand::Sync {
            names,
            check,
            no_install,
        } => {
            let mut registry = ctx.load()?;
            let outcomes = ops::sync(ctx, &mut registry, &names, check, no_install)?;
            let mut out_of_sync = false;
            for outcome in &outcomes {
                match outcome.state {
                    VendoredState::Current => {
                        if verbose {
                            println!("  {} {}", color.dim("ok"), outcome.name);
                        }
                    }
                    VendoredState::Changed => {
                        out_of_sync = true;
                        if check {
                            eprintln!("  {} {}", color.red("changed"), outcome.name);
                        } else {
                            println!("  {} {}", color.yellow("updated"), outcome.name);
                        }
                    }
                    VendoredState::Missing => {
                        out_of_sync = true;
                        if check {
                            eprintln!("  {} {}", color.red("missing"), outcome.name);
                        } else {
                            println!("  {} {}", color.green("vendored"), outcome.name);
                        }
                    }
                }
            }
            if check {
                if out_of_sync {
                    Ok(ExitCode::from(1))
                } else {
                    println!("{} skill(s) in sync", outcomes.len());
                    Ok(ExitCode::SUCCESS)
                }
            } else if !out_of_sync {
                println!("{} skill(s) already in sync", outcomes.len());
                Ok(ExitCode::SUCCESS)
            } else {
                println!(
                    "synced {} of {} skill(s)",
                    outcomes
                        .iter()
                        .filter(|o| o.state != VendoredState::Current)
                        .count(),
                    outcomes.len()
                );
                Ok(ExitCode::SUCCESS)
            }
        }

        CliCommand::Update {
            names,
            to,
            branch,
            no_fetch,
            dry_run,
            no_install,
        } => {
            let mut registry = ctx.load()?;
            let outcomes = ops::update(
                ctx,
                &mut registry,
                &names,
                ops::UpdateOptions {
                    to: to.as_deref(),
                    branch: branch.as_deref(),
                    no_fetch,
                    dry_run,
                    no_install,
                },
            )?;
            for outcome in &outcomes {
                if outcome.state == VendoredState::Current && outcome.old_rev == outcome.new_rev {
                    println!("  {} {} (already current)", color.dim("ok"), outcome.name);
                    continue;
                }
                let verb = if outcome.dry_run {
                    color.yellow("would update")
                } else {
                    color.green("updated")
                };
                let behind = match &outcome.upstream {
                    Some(Upstream::Behind { repo, path }) => format!(" [{repo}/{path} behind]"),
                    Some(Upstream::Diverged) => " [diverged]".to_string(),
                    Some(Upstream::PathRemoved { .. }) => " [path removed]".to_string(),
                    _ => String::new(),
                };
                println!(
                    "  {verb} {} {} -> {}{behind}",
                    outcome.name,
                    short(&outcome.old_rev),
                    short(&outcome.new_rev)
                );
            }
            Ok(ExitCode::SUCCESS)
        }

        CliCommand::List {
            names,
            check,
            outdated,
            json,
        } => list_command(ctx, color, &names, check, outdated, json),

        CliCommand::Status {
            names,
            outdated,
            json,
        } => list_command(ctx, color, &names, true, outdated, json),

        CliCommand::Install {
            names,
            dry_run,
            no_prune,
        } => {
            let registry = ctx.load()?;
            let report = ops::install_skills(ctx, &registry, &names, dry_run, !no_prune)?;
            output::print_install_report(&report, color);
            Ok(exit_for_report(&report))
        }

        CliCommand::Uninstall {
            names,
            all,
            dry_run,
        } => {
            let registry = ctx.load()?;
            let report = ops::uninstall_skills(ctx, &registry, &names, all, dry_run)?;
            output::print_install_report(&report, color);
            Ok(exit_for_report(&report))
        }

        CliCommand::New {
            name,
            description,
            edit,
            no_install,
        } => {
            let mut registry = ctx.load()?;
            let entry = ops::new_skill(
                ctx,
                &mut registry,
                &name,
                description.as_deref().unwrap_or(""),
                no_install,
            )?;
            println!("  {} authored skill {}", color.green("created"), entry.name);
            println!(
                "  edit {} to write the instructions",
                ctx.paths
                    .skill_dir(&entry.source, &entry.name)
                    .join("SKILL.md")
                    .display()
            );
            if edit {
                let path = ops::skill_file(ctx, &registry, &entry.name)?;
                ops::open_editor(&path)?;
            }
            Ok(ExitCode::SUCCESS)
        }

        CliCommand::Register {
            name,
            source,
            no_install,
        } => {
            let mut registry = ctx.load()?;
            let entry = ops::register(ctx, &mut registry, &name, source.as_deref(), no_install)?;
            println!(
                "  {} {} as {}/{}",
                color.green("registered"),
                entry.name,
                entry.source,
                entry.name
            );
            Ok(ExitCode::SUCCESS)
        }

        CliCommand::Adopt {
            name,
            from,
            no_install,
        } => {
            let mut registry = ctx.load()?;
            let entry = ops::adopt(ctx, &mut registry, &name, from.as_deref(), no_install)?;
            println!(
                "  {} {} into {}/{}",
                color.green("adopted"),
                entry.name,
                entry.source,
                entry.name
            );
            Ok(ExitCode::SUCCESS)
        }

        CliCommand::Edit { name } => {
            let registry = ctx.load()?;
            let path = ops::skill_file(ctx, &registry, &name)?;
            ops::open_editor(&path)?;
            Ok(ExitCode::SUCCESS)
        }

        CliCommand::Completion { shell } => {
            let mut command = Args::command();
            clap_complete::generate(
                match shell {
                    Shell::Bash => clap_complete::Shell::Bash,
                    Shell::Zsh => clap_complete::Shell::Zsh,
                    Shell::Fish => clap_complete::Shell::Fish,
                },
                &mut command,
                "skillmgr",
                &mut std::io::stdout(),
            );
            Ok(ExitCode::SUCCESS)
        }

        CliCommand::Tui => {
            skillmgr_tui::run(ctx.clone())?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn list_command(
    ctx: &Context,
    color: &Color,
    names: &[String],
    check: bool,
    outdated: bool,
    json: bool,
) -> Result<ExitCode> {
    let registry = ctx.load()?;
    let statuses = ops::list(ctx, &registry, names, check, outdated)?;
    if json {
        println!("{}", output::statuses_json(&statuses));
    } else {
        output::print_statuses(&statuses, color);
    }
    if (outdated && !statuses.is_empty()) || (check && statuses.is_empty() && !names.is_empty()) {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn print_add(outcome: &ops::AddOutcome, color: &Color) {
    let name = &outcome.entry.name;
    let rev = outcome.entry.rev().unwrap_or("-");
    let state = match outcome.state {
        VendoredState::Current => "already current",
        VendoredState::Changed => "updated",
        VendoredState::Missing => "vendored",
    };
    println!(
        "  {} {name} at {} ({state})",
        color.green("added"),
        short(rev)
    );
    if let skillmgr_core::registry::Origin::Vendored { repo, path, .. } = &outcome.entry.origin {
        println!("  {repo}:{path}");
    }
}

fn exit_for_report(report: &skillmgr_core::install::InstallReport) -> ExitCode {
    if report.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn short(rev: &str) -> String {
    rev.chars().take(9).collect()
}
