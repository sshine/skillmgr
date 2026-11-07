//! Errors shared by the skillmgr crates.

/// The crate result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong in skillmgr.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML error: {0}")]
    Toml(String),

    #[error("YAML error: {0}")]
    Yaml(String),

    #[error("git error: {0}")]
    Git(String),

    #[error("invalid argument: {0}")]
    Invalid(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("not a skill directory: {0}")]
    NotASkill(String),

    #[error("offline and {0}")]
    Offline(String),

    #[error("neither HOME nor XDG_CACHE_HOME is set")]
    NoHome,
}
