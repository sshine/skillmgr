//! Optional setup questions shown by the dashboard.
//!
//! Each question is defined once with a stable ID. Its answer is stored in
//! `skillmgr_core::wizard` state, so adding a question later does not require
//! another state file or re-asking earlier questions.

use std::collections::BTreeSet;

use skillmgr_core::ops;
use skillmgr_core::registry::Registry;
use skillmgr_core::wizard::{Prompt, State};

/// What to do when the user accepts a wizard question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardAction {
    ManageSkills,
}

/// One question shown in the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WizardPrompt {
    pub id: &'static str,
    pub version: u32,
    pub question: &'static str,
    pub action: WizardAction,
}

impl WizardPrompt {
    pub fn spec(self) -> Prompt {
        Prompt {
            id: self.id,
            version: self.version,
        }
    }
}

/// Every optional setup question, in the order they should be offered.
pub const PROMPTS: &[WizardPrompt] = &[WizardPrompt {
    id: "manage-skills",
    version: 1,
    question: "Install the skillmgr skill (manage-skills) now?",
    action: WizardAction::ManageSkills,
}];

/// The next question that applies and has not been answered at this version.
pub fn next_prompt(
    state: &State,
    registry: &Registry,
    deferred: &BTreeSet<&'static str>,
) -> Option<WizardPrompt> {
    PROMPTS.iter().copied().find(|prompt| {
        !deferred.contains(prompt.id) && applies(*prompt, registry) && state.needs(prompt.spec())
    })
}

fn applies(prompt: WizardPrompt, registry: &Registry) -> bool {
    match prompt.action {
        WizardAction::ManageSkills => !registry.contains(ops::SELF_SKILL_NAME),
    }
}
