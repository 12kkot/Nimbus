//! The newest page of the chats opened lately, kept in memory only: going back to a chat shows it
//! at once, while the fresh page is on its way.

use std::collections::{HashMap, VecDeque};

use serde_json::{json, Value};

/// A page is some 50 messages, so this is a few megabytes at most.
const MAX_PAGES: usize = 40;
/// The chats remembered between runs, as ids only (never their messages).
const MAX_RECENT: usize = 10;
const RECENT_KEY: &str = "recent_chats";

/// A chat as (server, channel).
pub type Place = (String, String);

/// The list with this chat first, and no longer than it may be.
fn with_recent(mut list: Vec<Place>, place: Place) -> Vec<Place> {
    list.retain(|p| p.1 != place.1);
    list.insert(0, place);
    list.truncate(MAX_RECENT);
    list
}

fn parse_recent(saved: &Value) -> Vec<Place> {
    saved.as_array().into_iter().flatten().filter_map(|p| Some((p[0].as_str()?.to_string(), p[1].as_str()?.to_string()))).take(MAX_RECENT).collect()
}

/// The chats opened lately, the last one first.
pub fn recent() -> Vec<Place> {
    parse_recent(&crate::prefs::read()[RECENT_KEY])
}

/// Remembers a chat as the one opened last.
pub fn note_recent(guild: &str, channel: &str) {
    let list = recent();
    if list.first().is_some_and(|p| p.1 == channel && p.0 == guild) {
        return;
    }
    let list = with_recent(list, (guild.to_string(), channel.to_string()));
    crate::prefs::write(RECENT_KEY, json!(list.iter().map(|p| [p.0.as_str(), p.1.as_str()]).collect::<Vec<_>>()));
}

/// Signing out: another account must not start with this one's chats.
pub fn forget_recent() {
    crate::prefs::write(RECENT_KEY, json!([]));
}

#[derive(Default)]
pub struct Pages {
    /// Oldest use first.
    order: VecDeque<String>,
    kept: HashMap<String, Value>,
}

impl Pages {
    pub fn get(&self, channel: &str) -> Option<&Value> {
        self.kept.get(channel)
    }

    pub fn put(&mut self, channel: &str, page: Value) {
        self.order.retain(|c| c != channel);
        self.order.push_back(channel.to_string());
        self.kept.insert(channel.to_string(), page);
        while self.order.len() > MAX_PAGES {
            if let Some(old) = self.order.pop_front() {
                self.kept.remove(&old);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recent_chats_keep_the_last_one_first_without_repeats() {
        let place = |c: &str| ("g".to_string(), c.to_string());
        let list = with_recent(vec![place("1"), place("2")], place("2"));
        assert_eq!(list, vec![place("2"), place("1")]);
        let mut long = Vec::new();
        for n in 0..MAX_RECENT + 5 {
            long = with_recent(long, place(&n.to_string()));
        }
        assert_eq!(long.len(), MAX_RECENT);
        assert_eq!(long[0], place(&(MAX_RECENT + 4).to_string()));
        let saved = json!([["g", "1"], ["bad"], ["g", "2"]]);
        assert_eq!(parse_recent(&saved), vec![place("1"), place("2")], "what can't be read is skipped");
    }

    #[test]
    fn the_chats_used_longest_ago_go_first() {
        let mut pages = Pages::default();
        for n in 0..MAX_PAGES {
            pages.put(&n.to_string(), json!(n));
        }
        pages.put("0", json!("again"));
        pages.put("new", json!("new"));
        assert_eq!(pages.get("0"), Some(&json!("again")), "used again, so it stays");
        assert_eq!(pages.get("1"), None, "the oldest use left");
        assert_eq!(pages.get("new"), Some(&json!("new")));
    }
}
