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

/// Edits of one value in one go (typing in a text box, a spinner): the
/// same author, the same [`Command::coalesce_key`], less than
/// [`COALESCE_MS`] apart. The feed shows them as one line, and a revert
/// takes them back together.
pub fn coalesces(prev: &Entry, next: &Entry) -> bool {
    prev.author == next.author
        && next.version == prev.version + 1
        && next.env.at - prev.env.at <= COALESCE_MS
        && prev.env.cmd.coalesce_key().is_some_and(|k| next.env.cmd.coalesce_key().as_deref() == Some(k.as_str()))
}

/// The gap below which [`coalesces`] merges edits (as `Session` does).
pub const COALESCE_MS: i64 = 1500;

/// Adds `entry`, replacing the line for `prev_version` of the same
/// character when it is the newest line.
pub(crate) fn merge(feed: &mut std::collections::VecDeque<FeedEntry>, entry: FeedEntry, prev_version: u64) {
    match feed.back_mut() {
        Some(last) if last.character == entry.character && last.version == Some(prev_version) => *last = entry,
        _ => push(feed, entry),
    }
}

pub(crate) fn push(feed: &mut std::collections::VecDeque<FeedEntry>, entry: FeedEntry) {
    feed.push_back(entry);
    while feed.len() > FEED_LIMIT {
        feed.pop_front();
    }
}

/// A feed line as the character's owner reads it: GM awards say "GM gave
/// you 100 karma: note" (or "GM took 5 karma from you"), anything else is
/// "<author>: <text>".
pub fn for_owner(e: &FeedEntry) -> String {
    if e.author_role == Role::Gm && e.rejected.is_none() {
        for (verb, form) in [("Gained ", "gave you"), ("Spent ", "took")] {
            if let Some(rest) = e.text.strip_prefix(verb) {
                let (amount, note) = match rest.split_once(": ") {
                    Some((a, n)) => (a, Some(n)),
                    None => (rest, None),
                };
                let line = if form == "took" { format!("{} took {amount} from you", e.author_name) } else { format!("{} gave you {amount}", e.author_name) };
                return match note {
                    Some(n) => format!("{line}: {n}"),
                    None => line,
                };
            }
        }
    }
    e.to_string()
}
