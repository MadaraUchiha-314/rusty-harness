//! `TaskRecord`, participants, sub- and parent-task links and the state machine (R12,
//! R21). Task 13 seeds the participant types the commands need; task 14 adds the record
//! and the state machine.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A participant's role on a task (R12.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// Does the work.
    Assignee,
    /// Asked for it.
    Reporter,
    /// Follows it.
    Watcher,
    /// May change anything about it.
    Admin,
}

impl Role {
    /// The role's name as the string catalog and the `participants/v1` extension spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Assignee => "assignee",
            Self::Reporter => "reporter",
            Self::Watcher => "watcher",
            Self::Admin => "admin",
        }
    }

    /// The role spelled `name`, as a plugin's `commands.json` lists them.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Assignee, Self::Reporter, Self::Watcher, Self::Admin]
            .into_iter()
            .find(|r| r.name() == name)
    }
}

/// What kind of thing a participant is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParticipantKind {
    /// A person.
    Human,
    /// An agent, by its registry identity.
    Agent(crate::registry::EntityId),
    /// A system (an event source, the harness itself).
    System,
}

/// A person, agent or system on a task, with its roles (R12.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    /// Stable id (the authenticated caller's id for a person).
    pub id: String,
    /// What to show.
    pub display: String,
    /// What it is.
    pub kind: ParticipantKind,
    /// Its roles.
    pub roles: BTreeSet<Role>,
}

impl Participant {
    /// A human participant with `roles`.
    #[must_use]
    pub fn human(
        id: impl Into<String>,
        display: impl Into<String>,
        roles: impl IntoIterator<Item = Role>,
    ) -> Self {
        Self {
            id: id.into(),
            display: display.into(),
            kind: ParticipantKind::Human,
            roles: roles.into_iter().collect(),
        }
    }

    /// Whether the participant holds `role`.
    #[must_use]
    pub fn has(&self, role: Role) -> bool {
        self.roles.contains(&role)
    }
}

/// Who is acting: the authenticated caller mapped to a participant (design.md § Security
/// design, boundary 1). Roles come from the task's participant list, never from text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caller {
    /// The caller's id.
    pub id: String,
    /// The roles the caller holds on the task at hand; empty for a non-participant.
    pub roles: BTreeSet<Role>,
    /// Whether the caller is a person (commands are parsed from people only).
    pub human: bool,
}

impl Caller {
    /// A human caller with `roles`.
    #[must_use]
    pub fn human(id: impl Into<String>, roles: impl IntoIterator<Item = Role>) -> Self {
        Self {
            id: id.into(),
            roles: roles.into_iter().collect(),
            human: true,
        }
    }

    /// A system or agent caller.
    #[must_use]
    pub fn system(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            roles: BTreeSet::new(),
            human: false,
        }
    }
}
