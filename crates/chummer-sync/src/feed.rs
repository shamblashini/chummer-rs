//! Activity feed lines, built the same way by the authority and the
//! replicas.

use chummer_core::command::{Command, Envelope};
use chummer_net::invite::Role;
use chummer_net::EndpointId;

use crate::msg::{CharacterId, Entry, FeedEntry, MemberInfo};

/// Most feed lines kept (oldest dropped first).
pub const FEED_LIMIT: usize = 2000;

/// The feed text for a command: its description, with the amount added
/// for manual karma and nuyen entries ("Gained 100 karma: Run payout"),
/// which the plain description leaves out.
pub fn text(env: &Envelope, description: &str) -> String {
    if let Command::ManualExpense { karma, gain, expense } = &env.cmd {
        let verb = if *gain { "Gained" } else { "Spent" };
        let amount = if *karma { format!("{} karma", expense.amount) } else { format!("{}¥", expense.amount) };
        return if expense.reason.is_empty() { format!("{verb} {amount}") } else { format!("{verb} {amount}: {}", expense.reason) };
    }
    description.to_owned()
}

/// A short name for a member without one: the start of the node id.
pub fn short_name(id: &EndpointId) -> String {
    id.fmt_short().to_string()
}

pub(crate) fn member_label(members: &[MemberInfo], id: &EndpointId) -> (String, Role) {
    match members.iter().find(|m| m.id == *id) {
        Some(m) => (if m.name.is_empty() { short_name(id) } else { m.name.clone() }, m.role),
        None => (short_name(id), Role::Player),
    }
}

/// The feed line for an applied log entry.
pub fn from_entry(members: &[MemberInfo], character: &CharacterId, character_name: &str, e: &Entry) -> FeedEntry {
    let (author_name, author_role) = member_label(members, &e.author);
    FeedEntry {
        at: e.env.at,
        character: character.clone(),
        character_name: character_name.to_owned(),
        author: e.author,
        author_name,
        author_role,
        text: text(&e.env, &e.description),
        version: Some(e.version),
        rejected: None,
    }
}

pub(crate) fn push(feed: &mut std::collections::VecDeque<FeedEntry>, entry: FeedEntry) {
    feed.push_back(entry);
    while feed.len() > FEED_LIMIT {
        feed.pop_front();
    }
}
