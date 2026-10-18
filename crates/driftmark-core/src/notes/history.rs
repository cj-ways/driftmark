//! A note's history: every action done to it, by whom, when, and where the
//! request came from (decision 0012).
//!
//! Only the reader decides, and an agent with a shell on the same machine
//! could send the reader's requests itself. So the program records, for
//! each action, whether it came from the reader's own page, from the MCP
//! connector, or from any other program that held the token. A reader's
//! action that did not come from the page carries a warning, in plain
//! words, that every page and every agent sees. The rule lives here so the
//! page and the connector say the same thing.

use serde::{Deserialize, Serialize};

use super::{Action, Role};

/// Where a request that acted on a note came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The reader's page: a same-origin request from driftmark's own page
    /// script, with the page's marker and the token cookie.
    Page,
    /// The MCP connector, `driftmark mcp`.
    Mcp,
    /// Any other request that carried the token, such as `curl`.
    Api,
    /// Made before driftmark recorded where actions come from.
    Unknown,
}

impl Source {
    /// Every source.
    pub const ALL: [Source; 4] = [Source::Page, Source::Mcp, Source::Api, Source::Unknown];

    /// The source's name in JSON and in the notes file.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Source::Page => "page",
            Source::Mcp => "mcp",
            Source::Api => "api",
            Source::Unknown => "unknown",
        }
    }

    /// The source with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|source| source.name() == name)
    }
}

/// What one entry of a note's history records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventAction {
    /// The note was made.
    Made,
    /// Its text, stance, ask or blocking flag changed.
    Edit,
    /// It was handed to an agent in a round-1 prompt or review file.
    Send,
    /// A reply was added to its thread.
    Reply,
    /// An agent proposed a decision.
    Propose,
    /// The reader decided.
    Decide,
    /// The commits that applied it were recorded.
    Applied,
    /// The reader took it back.
    Withdraw,
}

impl EventAction {
    /// Every action, in the order of the loop.
    pub const ALL: [EventAction; 8] = [
        EventAction::Made,
        EventAction::Edit,
        EventAction::Send,
        EventAction::Reply,
        EventAction::Propose,
        EventAction::Decide,
        EventAction::Applied,
        EventAction::Withdraw,
    ];

    /// The entry a loop action leaves. Every decision is one `decide`,
    /// whatever its outcome.
    #[must_use]
    pub fn of(action: Action) -> EventAction {
        match action {
            Action::Edit => EventAction::Edit,
            Action::Send => EventAction::Send,
            Action::Reply => EventAction::Reply,
            Action::Propose => EventAction::Propose,
            Action::Decide(_) => EventAction::Decide,
            Action::MarkApplied => EventAction::Applied,
            Action::Withdraw => EventAction::Withdraw,
        }
    }

    /// The action's name in JSON and in the notes file.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            EventAction::Made => "made",
            EventAction::Edit => "edit",
            EventAction::Send => "send",
            EventAction::Reply => "reply",
            EventAction::Propose => "propose",
            EventAction::Decide => "decide",
            EventAction::Applied => "applied",
            EventAction::Withdraw => "withdraw",
        }
    }

    /// The action with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<EventAction> {
        EventAction::ALL
            .into_iter()
            .find(|action| action.name() == name)
    }

    /// The action as the first words of a sentence, such as "Decided".
    fn done(self) -> &'static str {
        match self {
            EventAction::Made => "Made",
            EventAction::Edit => "Edited",
            EventAction::Send => "Sent to an agent",
            EventAction::Reply => "Replied",
            EventAction::Propose => "Proposed",
            EventAction::Decide => "Decided",
            EventAction::Applied => "Marked applied",
            EventAction::Withdraw => "Withdrawn",
        }
    }
}

/// One entry of a note's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// What was done.
    pub action: EventAction,
    /// Whether the reader or an agent did it.
    pub role: Role,
    /// Who did it: the reader's name, or the agent's client.
    pub by: String,
    /// When, in seconds since 1970 UTC.
    pub at: i64,
    /// Where the request came from.
    pub source: Source,
    /// What the program that sent it calls itself: a browser's or a tool's
    /// `User-Agent`, or the MCP client's name and version, cut short.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Why this entry should be looked at, when it should: a reader's action
    /// that did not come from the page. See [`warning`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl Event {
    /// An entry, with its warning when it needs one.
    #[must_use]
    pub fn new(
        action: EventAction,
        role: Role,
        by: String,
        at: i64,
        source: Source,
        user_agent: Option<String>,
    ) -> Event {
        Event {
            action,
            role,
            by,
            at,
            source,
            user_agent,
            warning: warning(action, role, source),
        }
    }
}

/// The warning a history entry carries, or `None`.
///
/// Only the reader's own actions can carry one, because only they speak for
/// the reader: an agent's reply through the connector is what the loop
/// expects. A reader's action warns whenever it did not come from the page,
/// such as "Decided through the API, not on the page". An action recorded
/// before driftmark kept sources warns too, because nothing shows where it
/// came from.
#[must_use]
pub fn warning(action: EventAction, role: Role, source: Source) -> Option<String> {
    if role != Role::Reader {
        return None;
    }
    let how = match source {
        Source::Page => return None,
        Source::Mcp => "through the MCP connector, not on the page",
        Source::Api => "through the API, not on the page",
        Source::Unknown => {
            "before driftmark recorded where actions come from, so it may not have come from the page"
        }
    };
    Some(format!("{} {how}", action.done()))
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::notes::Outcome;

    #[test]
    fn names_go_both_ways() {
        let names: Vec<&str> = Source::ALL.iter().map(|source| source.name()).collect();
        assert_eq!(names, ["page", "mcp", "api", "unknown"]);
        for source in Source::ALL {
            assert_eq!(Source::from_name(source.name()), Some(source));
        }
        assert_eq!(Source::from_name("browser"), None);
        let names: Vec<&str> = EventAction::ALL
            .iter()
            .map(|action| action.name())
            .collect();
        assert_eq!(
            names,
            [
                "made", "edit", "send", "reply", "propose", "decide", "applied", "withdraw"
            ]
        );
        for action in EventAction::ALL {
            assert_eq!(EventAction::from_name(action.name()), Some(action));
        }
        assert_eq!(EventAction::from_name("delete"), None);
    }

    #[test]
    fn each_loop_action_leaves_its_entry() {
        let pairs = [
            (Action::Edit, EventAction::Edit),
            (Action::Send, EventAction::Send),
            (Action::Reply, EventAction::Reply),
            (Action::Propose, EventAction::Propose),
            (Action::Decide(Outcome::Change), EventAction::Decide),
            (Action::Decide(Outcome::Keep), EventAction::Decide),
            (Action::MarkApplied, EventAction::Applied),
            (Action::Withdraw, EventAction::Withdraw),
        ];
        for (action, entry) in pairs {
            assert_eq!(EventAction::of(action), entry, "{action:?}");
        }
    }

    /// Every action, role and source, written out: the warning, or `-`.
    #[test]
    fn only_the_readers_actions_off_the_page_warn() {
        let mut table = String::new();
        for action in EventAction::ALL {
            for role in [Role::Reader, Role::Agent] {
                for source in Source::ALL {
                    let said = warning(action, role, source).unwrap_or_else(|| "-".to_owned());
                    let _ = writeln!(
                        table,
                        "{} {role:?} {}: {said}",
                        action.name(),
                        source.name()
                    );
                }
            }
        }
        assert_eq!(table, include_str!("history.expected.txt"), "\n{table}");
    }

    #[test]
    fn an_entry_carries_its_warning() {
        let quiet = Event::new(
            EventAction::Decide,
            Role::Reader,
            "Ana".into(),
            5,
            Source::Page,
            Some("Firefox".into()),
        );
        assert_eq!(quiet.warning, None);
        assert_eq!(
            (quiet.action, quiet.role, quiet.by.as_str(), quiet.at),
            (EventAction::Decide, Role::Reader, "Ana", 5)
        );
        assert_eq!(
            (quiet.source, quiet.user_agent.as_deref()),
            (Source::Page, Some("Firefox"))
        );
        let loud = Event::new(
            EventAction::Decide,
            Role::Reader,
            "Ana".into(),
            5,
            Source::Api,
            None,
        );
        assert_eq!(
            loud.warning.as_deref(),
            Some("Decided through the API, not on the page")
        );
    }
}
