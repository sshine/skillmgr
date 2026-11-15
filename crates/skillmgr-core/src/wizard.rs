//! Persistent answers for optional setup questions.
//!
//! The state file is deliberately separate from the skill registry. It holds
//! user-local choices keyed by managed root and prompt ID, so a new optional
//! question can be added without introducing another file or asking old
//! questions again.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, value};

use crate::error::{Error, Result};

/// The state file format version.
pub const SCHEMA: u32 = 1;

/// One wizard question. The ID is stable; bump `version` when the question or
/// the meaning of its answer changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prompt {
    pub id: &'static str,
    pub version: u32,
}

/// A recorded answer. Yes/no questions use `Bool`; a future choice can use
/// `Text` without changing the state format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Bool(bool),
    Text(String),
}

/// Remembered state for one managed root.
pub struct State {
    path: PathBuf,
    root: String,
    doc: DocumentMut,
}

impl State {
    /// Load state for `root` from `path`.
    ///
    /// A missing file is simply empty state: every currently defined prompt is
    /// pending. Nothing is written until an answer is recorded.
    pub fn load(path: &Path, root: &Path) -> Result<Self> {
        let root = root_key(root);

        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    path: path.to_path_buf(),
                    root,
                    doc: empty_document(),
                });
            }
            Err(e) => return Err(e.into()),
        };

        let doc = content
            .parse::<DocumentMut>()
            .map_err(|e| Error::Toml(e.to_string()))?;
        let schema = match doc.get("schema") {
            Some(item) => item
                .as_integer()
                .ok_or_else(|| Error::Toml("wizard state schema must be an integer".to_string()))?
                as u32,
            None => SCHEMA,
        };
        if schema != SCHEMA {
            return Err(Error::Toml(format!(
                "unsupported wizard state schema version {schema}"
            )));
        }

        Ok(Self {
            path: path.to_path_buf(),
            root,
            doc,
        })
    }

    /// Whether the prompt has not been answered at its current version.
    pub fn needs(&self, prompt: Prompt) -> bool {
        let recorded = self
            .doc
            .get("roots")
            .and_then(Item::as_table)
            .and_then(|roots| roots.get(&self.root))
            .and_then(Item::as_table)
            .and_then(|root| root.get("prompts"))
            .and_then(Item::as_table)
            .and_then(|prompts| prompts.get(prompt.id))
            .and_then(Item::as_table)
            .and_then(|entry| entry.get("version"))
            .and_then(Item::as_integer)
            .unwrap_or(0);

        recorded < i64::from(prompt.version)
    }

    /// Record a yes/no or textual answer and write the state file.
    pub fn record(&mut self, prompt: Prompt, answer: Answer) -> Result<()> {
        let entry = self.prompt_table_mut(prompt.id)?;
        entry.insert("version", value(i64::from(prompt.version)));
        entry.insert("status", value("answered"));
        match answer {
            Answer::Bool(answer) => {
                entry.insert("answer", value(answer));
            }
            Answer::Text(answer) => {
                entry.insert("answer", value(answer));
            }
        }
        entry.insert("answered_at", value(now()));
        self.save()
    }

    /// Mark a prompt as deliberately skipped without recording an answer.
    pub fn skip(&mut self, prompt: Prompt) -> Result<()> {
        let entry = self.prompt_table_mut(prompt.id)?;
        entry.insert("version", value(i64::from(prompt.version)));
        entry.insert("status", value("skipped"));
        entry.remove("answer");
        entry.remove("answered_at");
        self.save()
    }

    /// Write the state file atomically.
    pub fn save(&self) -> Result<()> {
        let Some(parent) = self.path.parent() else {
            std::fs::write(&self.path, self.doc.to_string())?;
            return Ok(());
        };
        std::fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        use std::io::Write;
        temp.write_all(self.doc.to_string().as_bytes())?;
        temp.flush()?;
        temp.persist(&self.path).map_err(|e| Error::Io(e.error))?;
        Ok(())
    }

    fn prompt_table_mut(&mut self, id: &str) -> Result<&mut Table> {
        ensure_table(&mut self.doc, "schema")?;
        ensure_table(&mut self.doc, "roots")?;
        let roots = self
            .doc
            .get_mut("roots")
            .and_then(Item::as_table_mut)
            .ok_or_else(|| Error::Toml("wizard state `roots` must be a table".to_string()))?;
        let root = roots
            .entry(&self.root)
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| Error::Toml("wizard root entry must be a table".to_string()))?;
        root.entry("created_at").or_insert_with(|| value(now()));
        let prompts = root
            .entry("prompts")
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| Error::Toml("wizard `prompts` must be a table".to_string()))?;
        prompts
            .entry(id)
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| Error::Toml(format!("wizard prompt '{id}' must be a table")))
    }
}

fn empty_document() -> DocumentMut {
    let mut doc = DocumentMut::new();
    doc.insert("schema", value(i64::from(SCHEMA)));
    doc.insert("roots", Item::Table(Table::new()));
    doc
}

fn ensure_table(doc: &mut DocumentMut, key: &str) -> Result<()> {
    if doc.get(key).is_none() {
        if key == "schema" {
            doc.insert(key, value(i64::from(SCHEMA)));
        } else {
            doc.insert(key, Item::Table(Table::new()));
        }
    }
    if key == "schema" {
        if doc.get(key).and_then(Item::as_integer).is_none() {
            return Err(Error::Toml(
                "wizard state schema must be an integer".to_string(),
            ));
        }
    } else if doc.get(key).and_then(Item::as_table).is_none() {
        return Err(Error::Toml(format!("wizard state `{key}` must be a table")));
    }
    Ok(())
}

fn root_key(root: &Path) -> String {
    std::fs::canonicalize(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(version: u32) -> Prompt {
        Prompt {
            id: "manage-skills",
            version,
        }
    }

    #[test]
    fn a_missing_state_file_leaves_prompts_pending() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("root");

        let state = State::load(&state_path, &root).expect("load");
        assert!(state.needs(prompt(1)));
        assert!(!state_path.exists());
    }

    #[test]
    fn an_answer_is_remembered_until_the_prompt_version_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("root");

        let mut state = State::load(&state_path, &root).expect("load");
        state
            .record(prompt(1), Answer::Bool(false))
            .expect("record");

        let state = State::load(&state_path, &root).expect("reload");
        assert!(!state.needs(prompt(1)));
        assert!(state.needs(prompt(2)));
    }

    #[test]
    fn answers_are_scoped_to_the_managed_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::create_dir_all(&first).expect("first");
        std::fs::create_dir_all(&second).expect("second");

        let mut state = State::load(&state_path, &first).expect("load first");
        state.record(prompt(1), Answer::Bool(true)).expect("record");
        assert!(!state.needs(prompt(1)));

        let other = State::load(&state_path, &second).expect("load second");
        assert!(other.needs(prompt(1)));
    }

    #[test]
    fn unknown_fields_survive_a_save() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state_path = dir.path().join("state.toml");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("root");
        let key = root_key(&root);
        std::fs::write(
            &state_path,
            format!(
                "# keep me\nschema = 1\nfuture_field = \"hello\"\n\n[roots.\"{key}\"]\ncreated_at = 1\n"
            ),
        )
        .expect("write");

        let mut state = State::load(&state_path, &root).expect("load");
        state.record(prompt(1), Answer::Bool(true)).expect("record");

        let text = std::fs::read_to_string(&state_path).expect("read");
        assert!(text.starts_with("# keep me\n"));
        assert!(text.contains("future_field = \"hello\""));
    }
}
