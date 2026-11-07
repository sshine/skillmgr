//! Command-line arguments.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "skillmgr",
    version,
    about = "Manage vendored and authored agent skills",
    long_about = None,
)]
pub struct Args {
    /// Managed skill directory. Defaults to ~/.agents/managed-skills.
    #[arg(long, global = true, value_name = "DIR")]
    pub root: Option<PathBuf>,

    /// Active skills directory. Defaults to ~/.agents/skills.
    #[arg(long, global = true, value_name = "DIR")]
    pub dest: Option<PathBuf>,

    /// Origin cache. Defaults to $XDG_CACHE_HOME/skillmgr/origins.
    #[arg(long, global = true, value_name = "DIR")]
    pub cache_dir: Option<PathBuf>,

    /// Never fetch; use only cached origins and vendored copies.
    #[arg(long, global = true)]
    pub offline: bool,

    /// Disable ANSI colour.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Print extra diagnostics.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create the managed directory and an empty skills.toml.
    Init {
        /// Do not initialize a git repository in the managed directory.
        #[arg(long)]
        no_git: bool,
    },

    /// Vendor a skill from a git repository.
    Add {
        /// Repository: owner/repo, an https/ssh URL, or a file:// URL.
        repo: String,

        /// Directory containing SKILL.md inside the repository.
        #[arg(default_value = ".")]
        path: String,

        /// Skill name. Defaults to the SKILL.md name or the path basename.
        #[arg(long)]
        name: Option<String>,

        /// Branch to track. Defaults to the remote HEAD.
        #[arg(long)]
        branch: Option<String>,

        /// Pin an exact revision instead of the branch head.
        #[arg(long)]
        rev: Option<String>,

        /// Do not fetch the origin first.
        #[arg(long)]
        no_fetch: bool,

        /// Do not symlink the skill into the active directory.
        #[arg(long)]
        no_install: bool,
    },

    /// Unregister skills, remove their links, and optionally delete files.
    #[command(alias = "rm")]
    Remove {
        /// Skill names. At least one.
        #[arg(required = true)]
        names: Vec<String>,

        /// Keep files even for vendored skills.
        #[arg(long, conflicts_with = "delete_files")]
        keep_files: bool,

        /// Delete files even for authored skills.
        #[arg(long)]
        delete_files: bool,

        /// Leave active-directory links alone.
        #[arg(long)]
        no_uninstall: bool,
    },

    /// Re-vendor skills at their pinned revisions.
    Sync {
        /// Skill names. Omit for every vendored skill.
        names: Vec<String>,

        /// Report differences and exit non-zero instead of writing.
        #[arg(long)]
        check: bool,

        /// Do not relink after syncing.
        #[arg(long)]
        no_install: bool,
    },

    /// Move pins to the branch head and re-vendor.
    Update {
        /// Skill names. Omit for every vendored skill.
        names: Vec<String>,

        /// Pin this exact revision. Requires exactly one skill name.
        #[arg(long, value_name = "REV")]
        to: Option<String>,

        /// Change the tracked branch.
        #[arg(long)]
        branch: Option<String>,

        /// Do not fetch origins first.
        #[arg(long)]
        no_fetch: bool,

        /// Show what would change without writing.
        #[arg(long)]
        dry_run: bool,

        /// Do not relink after updating.
        #[arg(long)]
        no_install: bool,
    },

    /// List registered and unregistered skills.
    #[command(alias = "ls")]
    List {
        /// Skill names. Omit for everything.
        names: Vec<String>,

        /// Query upstream and show behind counts.
        #[arg(long)]
        check: bool,

        /// Show only skills that are behind; exits non-zero if any are.
        #[arg(long)]
        outdated: bool,

        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },

    /// List skills with upstream behind counts.
    Status {
        /// Skill names. Omit for everything.
        names: Vec<String>,

        /// Show only skills that are behind; exits non-zero if any are.
        #[arg(long)]
        outdated: bool,

        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },

    /// Symlink registered skills into the active directory.
    Install {
        /// Skill names. Omit for every registered skill.
        names: Vec<String>,

        /// Report what would happen without writing.
        #[arg(long)]
        dry_run: bool,

        /// Leave stale managed links in place.
        #[arg(long)]
        no_prune: bool,
    },

    /// Remove managed symlinks from the active directory.
    Uninstall {
        /// Skill names.
        names: Vec<String>,

        /// Remove every managed link.
        #[arg(long)]
        all: bool,

        /// Report what would happen without writing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Scaffold an authored skill.
    New {
        /// Skill name.
        name: String,

        /// Initial description.
        #[arg(long)]
        description: Option<String>,

        /// Open SKILL.md in $EDITOR after creating it.
        #[arg(long)]
        edit: bool,

        /// Do not symlink the skill into the active directory.
        #[arg(long)]
        no_install: bool,
    },

    /// Register an existing directory below skills/.
    Register {
        /// Skill name.
        name: String,

        /// Source directory name, e.g. authored or vendored.
        #[arg(long)]
        source: Option<String>,

        /// Do not symlink the skill into the active directory.
        #[arg(long)]
        no_install: bool,
    },

    /// Move an existing active skill directory into skills/authored/.
    Adopt {
        /// Skill name.
        name: String,

        /// Source directory. Defaults to the active directory entry.
        #[arg(long, value_name = "DIR")]
        from: Option<PathBuf>,

        /// Do not symlink the skill after adopting it.
        #[arg(long)]
        no_install: bool,
    },

    /// Open a skill's SKILL.md in $EDITOR.
    Edit {
        /// Skill name.
        name: String,
    },

    /// Print a shell completion script.
    Completion {
        /// Shell to generate for.
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Open the interactive dashboard.
    Tui,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}
