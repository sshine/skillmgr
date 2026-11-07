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
    Vendored {
        repo: String,
        path: String,
        branch: String,
        rev: String,
    },
    Authored,
}

impl Origin {
    pub fn is_vendored(&self) -> bool {
        matches!(self, Self::Vendored { .. })
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

    pub fn rev(&self) -> Option<&str> {
        match &self.origin {
            Origin::Vendored { rev, .. } => Some(rev),
            Origin::Authored => None,
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
            Origin::Authored => {
                return Err(Error::Invalid(format!(
                    "skill '{name}' is authored and has no branch"
                )));
            }
        }
        self.set_field(name, "branch", branch)
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
    if let Origin::Vendored {
        repo,
        path,
        branch,
        rev,
    } = &entry.origin
    {
        table.insert("repo", value(repo.clone()));
        table.insert("path", value(path.clone()));
        table.insert("branch", value(branch.clone()));
        table.insert("rev", value(rev.clone()));
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

    let any_vendor_field = repo.is_some() || path.is_some() || branch.is_some() || rev.is_some();
    if any_vendor_field || source == "vendored" {
        let repo = repo.ok_or_else(|| missing(name, "repo"))?;
        let path = path.ok_or_else(|| missing(name, "path"))?;
        let branch = branch.ok_or_else(|| missing(name, "branch"))?;
        let rev = rev.ok_or_else(|| missing(name, "rev"))?;
        validate_branch(&branch)?;
        if rev.trim().is_empty() {
            return Err(missing(name, "rev"));
        }
        return Ok(Entry::vendored(
            name,
            source,
            repo,
            normalize_skill_path(&path)?,
            branch,
            rev,
        ));
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
    fn vendored_entries_require_all_git_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("skills.toml");
        std::fs::write(
            &path,
            "[skills.foo]\nsource = \"vendored\"\nrepo = \"sshine/skills\"\n",
        )
        .expect("write");
        assert!(Registry::load(&path).is_err());
    }
}
