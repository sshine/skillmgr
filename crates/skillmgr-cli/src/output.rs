//! Human-readable and JSON output.

use skillmgr_core::git::Upstream;
use skillmgr_core::install::{InstallReport, LinkState};
use skillmgr_core::registry::Origin;
use skillmgr_core::status::SkillStatus;
use unicode_width::UnicodeWidthStr;

use crate::color::Color;

pub fn print_statuses(statuses: &[SkillStatus], color: &Color) {
    let headers = ["NAME", "ORIGIN", "LINK", "REV", "BEHIND", "PATH"];
    let rows: Vec<[String; 6]> = statuses
        .iter()
        .map(|s| {
            [
                s.name.clone(),
                origin_label(s).to_string(),
                link_label(&s.link).to_string(),
                short_rev(s),
                s.behind_label(),
                path_label(s),
            ]
        })
        .collect();

    let mut widths = headers.map(str::len);
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.width());
        }
    }

    println!(
        "{}  {}  {}  {}  {}  {}",
        pad(headers[0], widths[0]),
        pad(headers[1], widths[1]),
        pad(headers[2], widths[2]),
        pad(headers[3], widths[3]),
        pad(headers[4], widths[4]),
        headers[5],
    );

    for row in &rows {
        println!(
            "{}  {}  {}  {}  {}  {}",
            pad(&row[0], widths[0]),
            painted_cell(paint_origin(&row[1], color), &row[1], widths[1]),
            painted_cell(paint_link(&row[2], color), &row[2], widths[2]),
            pad(&row[3], widths[3]),
            painted_cell(paint_behind(&row[4], color), &row[4], widths[4]),
            row[5],
        );
    }
}

pub fn print_install_report(report: &InstallReport, color: &Color) {
    for name in &report.linked {
        println!("  {} {name}", color.green("linked"));
    }
    for name in &report.already {
        println!("  {} {name}", color.dim("ok"));
    }
    for name in &report.pruned {
        println!("  {} {name}", color.yellow("pruned"));
    }
    for name in &report.removed {
        println!("  {} {name}", color.yellow("unlinked"));
    }
    for conflict in &report.conflicts {
        eprintln!(
            "  {} {}: {}",
            color.red("conflict"),
            conflict.name,
            conflict.detail
        );
    }
}

pub fn statuses_json(statuses: &[SkillStatus]) -> String {
    let values: Vec<serde_json::Value> = statuses
        .iter()
        .map(|s| {
            let (repo, path, branch, rev) = match &s.origin {
                Origin::Vendored {
                    repo,
                    path,
                    branch,
                    rev,
                } => (
                    Some(repo.clone()),
                    Some(path.clone()),
                    Some(branch.clone()),
                    Some(rev.clone()),
                ),
                Origin::Repo {
                    repo,
                    path,
                    branch,
                    rev,
                    ..
                } => (
                    Some(repo.clone()),
                    Some(path.clone()),
                    branch.clone(),
                    rev.clone(),
                ),
                Origin::Authored => (None, None, None, None),
            };
            serde_json::json!({
                "name": s.name,
                "source": s.source,
                "origin": origin_label(s),
                "registered": s.registered,
                "link": link_label(&s.link),
                "managed_path": s.managed_path.display().to_string(),
                "repo": repo,
                "path": path,
                "branch": branch,
                "rev": rev,
                "behind": match &s.upstream {
                    Some(Upstream::Behind { repo, path }) => Some((*repo, *path)),
                    _ => None,
                },
                "upstream": upstream_label(s.upstream.as_ref()),
                "description": s.description,
                "version": s.version,
                "note": s.note,
            })
        })
        .collect();
    serde_json::to_string_pretty(&values).unwrap_or_else(|_| "[]".to_string())
}

fn upstream_label(upstream: Option<&Upstream>) -> &'static str {
    match upstream {
        Some(Upstream::Current) => "current",
        Some(Upstream::Behind { .. }) => "behind",
        Some(Upstream::Diverged) => "diverged",
        Some(Upstream::PathRemoved { .. }) => "path-removed",
        Some(Upstream::Unreachable) => "unreachable",
        Some(Upstream::Unknown(_)) => "unknown",
        None => "unchecked",
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

fn path_label(status: &SkillStatus) -> String {
    match &status.origin {
        Origin::Vendored { repo, path, .. } | Origin::Repo { repo, path, .. } => {
            if path == "." {
                repo.clone()
            } else {
                format!("{repo}:{path}")
            }
        }
        Origin::Authored => status.source.clone(),
    }
}

fn short_rev(status: &SkillStatus) -> String {
    match &status.origin {
        Origin::Vendored { rev, .. } => rev.chars().take(7).collect(),
        Origin::Repo { .. } | Origin::Authored => "-".to_string(),
    }
}

fn link_label(link: &LinkState) -> &'static str {
    match link {
        LinkState::Linked => "linked",
        LinkState::Unlinked => "unlinked",
        LinkState::Broken => "broken",
        LinkState::ForeignDir => "foreign-dir",
        LinkState::ForeignSymlink => "foreign-link",
    }
}

fn paint_origin(text: &str, color: &Color) -> String {
    match text {
        "vendored" => color.blue(text),
        "authored" => color.green(text),
        "repo" => color.yellow(text),
        "empty" => color.dim(text),
        "faulty" => color.red(text),
        "unregistered" => color.yellow(text),
        _ => text.to_string(),
    }
}

fn paint_link(text: &str, color: &Color) -> String {
    match text {
        "linked" => color.green(text),
        "unlinked" => color.yellow(text),
        "broken" | "foreign-dir" | "foreign-link" => color.red(text),
        _ => text.to_string(),
    }
}

fn paint_behind(text: &str, color: &Color) -> String {
    match text {
        "0/0" | "-" => color.dim(text),
        "?" | "unreachable" | "diverged" | "0/gone" => color.red(text),
        _ => {
            if text.ends_with("/0") {
                color.yellow(text)
            } else if text == "0/0" {
                color.dim(text)
            } else {
                color.red(text)
            }
        }
    }
}

fn pad(text: &str, width: usize) -> String {
    let current = text.width();
    if current >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - current))
    }
}

fn painted_cell(painted: String, plain: &str, width: usize) -> String {
    let current = plain.width();
    if current >= width {
        painted
    } else {
        format!("{painted}{}", " ".repeat(width - current))
    }
}
