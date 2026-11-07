//! Core registry, git, vendoring and symlink logic for skillmgr.

pub mod error;
pub mod git;
pub mod install;
pub mod ops;
pub mod paths;
pub mod registry;
pub mod repo;
pub mod skill;
pub mod status;
pub mod vendor;

pub use error::{Error, Result};
pub use paths::Paths;
