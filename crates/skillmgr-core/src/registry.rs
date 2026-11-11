//! The `skills.toml` registry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, value};

use crate::error::{Error, Result};
use crate::repo::normalize_skill_path;
use crate::skill::validate_name;

/// Where a skill's content comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A resolved upstream skill pinned to one revision.
    Vendored {
        repo: String,
        path: String,
        branch: String,
        rev: String,
    },
    /// A declared upstream repository that has not resolved into skills yet.
    ///
    /// A repository is a skills collection when it contains `SKILL.md` files
    /// anywhere. It may legitimately contain none, and it may be unreachable;
    /// either way the declaration is kept so that a later retry can resolve it.
    Repo {
        repo: String,
        path: String,
        branch: Option<String>,
        rev: Option<String>,
        /// Resolution information, such as "no SKILL.md files found".
        status: Option<String>,
        /// Why resolution failed, if it did.
        error: Option<String>,
    },
    Authored,
}

impl Origin {
    pub fn is_vendored(&self) -> bool {
        matches!(self, Self::Vendored { .. })
    }

    pub fn is_repo(&self) -> bool {
        matches!(self, Self::Repo { .. })
    }
}

/// One declared skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub source: String,
    pub origin: Origin,
}

impl Entry {
    pub fn vendored(
        name: impl Into<String>,
        source: impl Into<String>,
        repo: impl Into<String>,
        path: impl Into<String>,
        branch: impl Into<String>,
        rev: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            origin: Origin::Vendored {
                repo: repo.into(),
                path: path.into(),
                branch: branch.into(),
                rev: rev.into(),
            },
        }
    }

    pub fn authored(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            origin: Origin::Authored,
        }
    }

    /// Declare a repository before (or without) resolving it into skills.
    pub fn repo_declaration(
        name: impl Into<String>,
        source: impl Into<String>,
        repo: impl Into<String>,
        path: impl Into<String>,
        branch: Option<String>,
        rev: Option<String>,
        status: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            origin: Origin::Repo {
                repo: repo.into(),
                path: path.into(),
                branch,
                rev,
                status,
                error: None,
            },
        }
    }

    pub fn rev(&self) -> Option<&str> {
        match &self.origin {
            Origin::Vendored { rev, .. } => Some(rev),
            Origin::Repo { .. } | Origin::Authored => None,
        }
    }

    /// Whether this entry is a resolved skill rather than a repository declaration.
    pub fn is_skill(&self) -> bool {
        !self.origin.is_repo()
    }

    pub fn is_repo(&self) -> bool {
        self.origin.is_repo()
    }

    pub fn is_vendored(&self) -> bool {
        self.origin.is_vendored()
    }

    /// The upstream repository for both resolved and unresolved entries.
    pub fn repo(&self) -> Option<&str> {
        match &self.origin {
            Origin::Vendored { repo, .. } | Origin::Repo { repo, .. } => Some(repo),
            Origin::Authored => None,
        }
    }

    /// The path inside the repository for both resolved and unresolved entries.
    pub fn repo_path(&self) -> Option<&str> {
        match &self.origin {
            Origin::Vendored { path, .. } | Origin::Repo { path, .. } => Some(path),
            Origin::Authored => None,
        }
    }

    /// A repository's non-error resolution status, such as "no skills found".
    pub fn repo_status(&self) -> Option<&str> {
        match &self.origin {
            Origin::Repo { status, .. } => status.as_deref(),
            _ => None,
        }
    }

    /// Why a repository declaration failed to resolve.
    pub fn repo_error(&self) -> Option<&str> {
        match &self.origin {
            Origin::Repo { error, .. } => error.as_deref(),
            _ => None,
        }
    }
}

/// The parsed registry plus the `toml_edit` document used to write it back.
#[derive(Debug)]
pub struct Registry {
    path: PathBuf,
    schema: u32,
    entries: BTreeMap<String, Entry>,
    doc: DocumentMut,
}

impl Registry {
    /// Load `skills.toml`, preserving comments and formatting for later writes.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|_| {
            Error::NotFound(format!("{} (run `skillmgr init` first)", path.display()))
        })?;
        let doc = content
            .parse::<DocumentMut>()
            .map_err(|e| Error::Toml(e.to_string()))?;

        let schema = match doc.get("schema") {
            Some(item) => item
                .as_integer()
                .ok_or_else(|| Error::Toml("schema must be an integer".to_string()))?
                as u32,
            None => 1,
        };
        if schema != 1 {
            return Err(Error::Toml(format!("unsupported schema version {schema}")));
        }

        let mut entries = BTreeMap::new();
        if let Some(item) = doc.get("skills") {
            let table = item
                .as_table()
                .ok_or_else(|| Error::Toml("`skills` must be a table".to_string()))?;
            for (name, item) in table.iter() {
                let entry = parse_entry(name, item)?;
                entries.insert(name.to_string(), entry);
            }
        }

        Ok(Self {
            path: path.to_path_buf(),
            schema,
            entries,
            doc,
        })
    }

    /// An empty in-memory registry, before the first save.
    pub fn empty(path: &Path) -> Self {
        let mut doc = DocumentMut::new();
        doc.insert("schema", value(1_i64));
        doc.insert("skills", Item::Table(Table::new()));
        Self {
            path: path.to_path_buf(),
            schema: 1,
            entries: BTreeMap::new(),
            doc,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn schema(&self) -> u32 {
        self.schema
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    pub fn names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Resolve a list of names, where an empty list means every entry.
    pub fn select(&self, names: &[String]) -> Result<Vec<&Entry>> {
        if names.is_empty() {
            return Ok(self.entries.values().collect());
        }
        names
            .iter()
            .map(|name| {
                self.entries
                    .get(name)
                    .ok_or_else(|| Error::NotFound(format!("skill '{name}'")))
            })
            .collect()
    }

    /// Insert or replace an entry.
    pub fn insert(&mut self, entry: Entry) -> Result<()> {
        validate_name(&entry.name)?;
        validate_source(&entry.source)?;

        ensure_skills_table(&mut self.doc)?;
        let table = entry_table(&entry);
        let skills = self
            .doc
            .get_mut("skills")
            .and_then(Item::as_table_mut)
            .ok_or_else(|| Error::Toml("`skills` must be a table".to_string()))?;
        skills.insert(&entry.name, Item::Table(table));

        self.entries.insert(entry.name.clone(), entry);
        Ok(())
    }

    /// Remove an entry, returning it if it existed.
    pub fn remove(&mut self, name: &str) -> Option<Entry> {
        if let Some(skills) = self.doc.get_mut("skills").and_then(Item::as_table_mut) {
            skills.remove(name);
        }
        self.entries.remove(name)
    }

    /// Update a vendored entry's revision.
    pub fn set_rev(&mut self, name: &str, rev: &str) -> Result<()> {
        let entry = self
            .entries
            .get_mut(name)
            .ok_or_else(|| Error::NotFound(format!("skill '{name}'")))?;
        match &mut entry.origin {
            Origin::Vendored { rev: pinned, .. } => *pinned = rev.to_string(),
            Origin::Repo { .. } => {
                return Err(Error::Invalid(format!(
                    "repository '{name}' has not resolved into a skill"
                )));
            }
            Origin::Authored => {
                return Err(Error::Invalid(format!(
                    "skill '{name}' is authored and has no revision"
                )));
            }
        }
        self.set_field(name, "rev", rev)
    }

    /// Update a vendored entry's branch.
    pub fn set_branch(&mut self, name: &str, branch: &str) -> Result<()> {
        validate_branch(branch)?;
        let entry = self
            .entries
            .get_mut(name)
            .ok_or_else(|| Error::NotFound(format!("skill '{name}'")))?;
        match &mut entry.origin {
            Origin::Vendored {
                branch: recorded, ..
            } => *recorded = branch.to_string(),
            Origin::Repo { .. } => {
                return Err(Error::Invalid(format!(
                    "repository '{name}' has not resolved into a skill"
                )));
            }
            Origin::Authored => {
                return Err(Error::Invalid(format!(
                    "skill '{name}' is authored and has no branch"
                )));
            }
        }
        self.set_field(name, "branch", branch)
    }

    /// Record what a repository declaration resolved to, or why it failed.
    pub fn set_repo_resolution(
        &mut self,
        name: &str,
        branch: Option<&str>,
        rev: Option<&str>,
        status: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let entry = self
            .entries
            .get_mut(name)
            .ok_or_else(|| Error::NotFound(format!("repository '{name}'")))?;
        match &mut entry.origin {
            Origin::Repo {
                branch: recorded_branch,
                rev: recorded_rev,
                status: recorded_status,
                error: recorded_error,
                ..
            } => {
                if let Some(branch) = branch {
                    validate_branch(branch)?;
                    *recorded_branch = Some(branch.to_string());
                }
                if let Some(rev) = rev {
                    *recorded_rev = Some(rev.to_string());
                }
                *recorded_status = status.map(str::to_string);
                *recorded_error = error.map(str::to_string);
            }
            Origin::Vendored { .. } | Origin::Authored => {
                return Err(Error::Invalid(format!(
                    "'{name}' is not a repository declaration"
                )));
            }
        }

        let skills = self
            .doc
            .get_mut("skills")
            .and_then(Item::as_table_mut)
            .ok_or_else(|| Error::Toml("`skills` must be a table".to_string()))?;
        let table = skills
            .get_mut(name)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| Error::Toml(format!("missing table for repository '{name}'")))?;
        match branch {
            Some(branch) => {
                table.insert("branch", value(branch.to_string()));
            }
            None => {
                table.remove("branch");
            }
        }
        match rev {
            Some(rev) => {
                table.insert("rev", value(rev.to_string()));
            }
            None => {
                table.remove("rev");
            }
        }
        match status {
            Some(status) => {
                table.insert("status", value(status.to_string()));
            }
            None => {
                table.remove("status");
            }
        }
        match error {
            Some(error) => {
                table.insert("error", value(error.to_string()));
            }
            None => {
                table.remove("error");
            }
        }
        Ok(())
    }

    fn set_field(&mut self, name: &str, field: &str, text: &str) -> Result<()> {
        let skills = self
            .doc
            .get_mut("skills")
            .and_then(Item::as_table_mut)
            .ok_or_else(|| Error::Toml("`skills` must be a table".to_string()))?;
        let table = skills
            .get_mut(name)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| Error::Toml(format!("missing table for skill '{name}'")))?;
        table.insert(field, value(text.to_string()));
        Ok(())
    }

    /// Write the registry atomically.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
            let mut temp = tempfile::NamedTempFile::new_in(parent)?;
            use std::io::Write;
            temp.write_all(self.doc.to_string().as_bytes())?;
            temp.flush()?;
            temp.persist(&self.path).map_err(|e| Error::Io(e.error))?;
        } else {
            std::fs::write(&self.path, self.doc.to_string())?;
        }
        Ok(())
    }
}

fn ensure_skills_table(doc: &mut DocumentMut) -> Result<()> {
    if doc.get("schema").is_none() {
        doc.insert("schema", value(1_i64));
    }
    if doc.get("skills").is_none() {
        doc.insert("skills", Item::Table(Table::new()));
    }
    if doc.get("skills").and_then(Item::as_table).is_none() {
        return Err(Error::Toml("`skills` must be a table".to_string()));
    }
    Ok(())
}

fn entry_table(entry: &Entry) -> Table {
    let mut table = Table::new();
    table.insert("source", value(entry.source.clone()));
    match &entry.origin {
        Origin::Vendored {
            repo,
            path,
            branch,
            rev,
        } => {
            table.insert("repo", value(repo.clone()));
            table.insert("path", value(path.clone()));
            table.insert("branch", value(branch.clone()));
            table.insert("rev", value(rev.clone()));
        }
        Origin::Repo {
            repo,
            path,
            branch,
            rev,
            status,
            error,
        } => {
            table.insert("kind", value("repository"));
            table.insert("repo", value(repo.clone()));
            table.insert("path", value(path.clone()));
            if let Some(branch) = branch {
                table.insert("branch", value(branch.clone()));
            }
            if let Some(rev) = rev {
                table.insert("rev", value(rev.clone()));
            }
            if let Some(status) = status {
                table.insert("status", value(status.clone()));
            }
            if let Some(error) = error {
                table.insert("error", value(error.clone()));
            }
        }
        Origin::Authored => {}
    }
    table
}

fn parse_entry(name: &str, item: &Item) -> Result<Entry> {
    validate_name(name)?;
    let table = item
        .as_table()
        .ok_or_else(|| Error::Toml(format!("skill '{name}' must be a table")))?;

    let source = required_string(table, name, "source")?;
    validate_source(&source)?;

    let repo = optional_string(table, "repo");
    let path = optional_string(table, "path");
    let branch = optional_string(table, "branch");
    let rev = optional_string(table, "rev");
    let status = optional_string(table, "status");
    let error = optional_string(table, "error");
    let kind = optional_string(table, "kind");
    if let Some(kind) = &kind
        && kind != "repository"
    {
        return Err(Error::Toml(format!(
            "skill '{name}' has unknown kind '{kind}'"
        )));
    }
    let declared_repository = kind.as_deref() == Some("repository");

    let any_vendor_field = declared_repository
        || repo.is_some()
        || path.is_some()
        || branch.is_some()
        || rev.is_some();
    if any_vendor_field || source == "vendored" {
        let repo = repo.ok_or_else(|| missing(name, "repo"))?;
        let path = normalize_skill_path(path.as_deref().unwrap_or("."))?;

        // A fully pinned entry is a skill. Anything else is a repository
        // declaration whose resolution failed, has not run yet, or found no
        // SKILL.md files.
        if !declared_repository
            && status.is_none()
            && error.is_none()
            && let (Some(branch), Some(rev)) = (&branch, &rev)
        {
            validate_branch(branch)?;
            if rev.trim().is_empty() {
                return Err(missing(name, "rev"));
            }
            return Ok(Entry::vendored(name, source, repo, path, branch, rev));
        }

        if let Some(branch) = &branch {
            validate_branch(branch)?;
        }
        let mut entry = Entry::repo_declaration(name, source, repo, path, branch, rev, status);
        if let Origin::Repo {
            error: recorded, ..
        } = &mut entry.origin
        {
            *recorded = error;
        }
        return Ok(entry);
    }

    Ok(Entry::authored(name, source))
}

fn required_string(table: &Table, name: &str, key: &str) -> Result<String> {
    optional_string(table, key).ok_or_else(|| missing(name, key))
}

fn optional_string(table: &Table, key: &str) -> Option<String> {
    table
        .get(key)
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn missing(name: &str, key: &str) -> Error {
    Error::Toml(format!("skill '{name}' is missing '{key}'"))
}

pub fn validate_source(source: &str) -> Result<()> {
    if source.is_empty()
        || source == "."
        || source == ".."
        || source.contains('/')
        || source.contains('\\')
    {
        return Err(Error::Invalid(format!(
            "source must be a single directory name: {source}"
        )));
    }
    Ok(())
}

pub fn validate_branch(branch: &str) -> Result<()> {
    if branch.is_empty()
        || branch.starts_with('-')
        || branch.contains(char::is_whitespace)
        || branch.contains("..")
    {
        return Err(Error::Invalid(format!("invalid branch: {branch}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_header_comments() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("skills.toml");
        std::fs::write(
            &path,
            "# keep me\nschema = 1\n\n[skills.foo]\nsource = \"authored\"\n",
        )
        .expect("write");

        let mut registry = Registry::load(&path).expect("load");
        registry
            .insert(Entry::vendored(
                "bar",
                "vendored",
                "sshine/skills",
                "bar",
                "main",
                "abc123",
            ))
            .expect("insert");
        registry.save().expect("save");

        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.starts_with("# keep me\n"));
        let reloaded = Registry::load(&path).expect("reload");
        assert_eq!(reloaded.entries().count(), 2);
    }

    #[test]
    fn a_repository_declaration_with_a_revision_is_not_mistaken_for_a_skill() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("skills.toml");
        std::fs::write(
            &path,
            "[skills.foo]\nsource = \"vendored\"\nkind = \"repository\"\nrepo = \"sshine/skills\"\npath = \".\"\nbranch = \"main\"\nrev = \"abc123\"\n",
        )
        .expect("write");
        let registry = Registry::load(&path).expect("load");
        assert!(registry.get("foo").expect("entry").is_repo());
    }

    #[test]
    fn an_unpinned_repository_declaration_round_trips_as_faulty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("skills.toml");
        std::fs::write(
            &path,
            "[skills.foo]\nsource = \"vendored\"\nrepo = \"sshine/skills\"\nerror = \"no such host\"\n",
        )
        .expect("write");
        let registry = Registry::load(&path).expect("load");
        let entry = registry.get("foo").expect("entry");
        assert!(entry.is_repo());
        assert_eq!(entry.repo(), Some("sshine/skills"));
        assert_eq!(entry.repo_error(), Some("no such host"));
    }
}
