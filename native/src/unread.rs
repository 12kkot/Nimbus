//! What's unread and where you're mentioned: the white pill by a server,
//! bold channel names and the red mention counts, from Discord's read states.

use std::collections::HashMap;

use serde_json::Value;

fn snowflake(v: &Value) -> u64 {
    v.as_str().and_then(|s| s.parse().ok()).or(v.as_u64()).unwrap_or(0)
}

/// Where you stopped reading each channel, and its unread mentions.
#[derive(Default)]
pub struct Unread {
    /// Channel id -> (last message read, mentions since).
    read: HashMap<String, (u64, u32)>,
    /// Channel id -> newest message in it.
    last: HashMap<String, u64>,
    /// Channel id -> its server (absent for DMs).
    guild_of: HashMap<String, String>,
    /// What each server and channel is set to tell about, and what's muted.
    pub settings: crate::notify::Settings,
}

/// A server's or a DM side's summary for its icon.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Badge {
    pub unread: bool,
    pub mentions: u32,
}

/// READY's lists come either bare or as `{ "entries": [...] }`.
fn entries(v: &Value) -> &[Value] {
    v.as_array().or_else(|| v["entries"].as_array()).map(Vec::as_slice).unwrap_or_default()
}

impl Unread {
    pub fn from_ready(ready: &Value) -> Self {
        let mut u = Unread::default();
        for r in entries(&ready["read_state"]) {
            if let Some(id) = r["id"].as_str() {
                u.read.insert(id.to_string(), (snowflake(&r["last_message_id"]), r["mention_count"].as_u64().unwrap_or(0) as u32));
            }
        }
        for g in ready["guilds"].as_array().into_iter().flatten() {
            let Some(gid) = g["id"].as_str() else { continue };
            for c in g["channels"].as_array().into_iter().flatten() {
                if let Some(cid) = c["id"].as_str() {
                    u.last.insert(cid.to_string(), snowflake(&c["last_message_id"]));
                    u.guild_of.insert(cid.to_string(), gid.to_string());
                }
            }
        }
        for c in ready["private_channels"].as_array().into_iter().flatten() {
            if let Some(cid) = c["id"].as_str() {
                u.last.insert(cid.to_string(), snowflake(&c["last_message_id"]));
            }
        }
        u.settings = crate::notify::Settings::from_ready(ready, chrono::Utc::now());
        u
    }

    /// Unread: a newer message than the last one read. Channels never opened
    /// have no read state; they don't count (they're often ones you can't see).
    pub fn is_unread(&self, channel: &str) -> bool {
        match (self.read.get(channel), self.last.get(channel)) {
            (Some((read, _)), Some(last)) => last > read,
            _ => false,
        }
    }

    pub fn mentions(&self, channel: &str) -> u32 {
        self.read.get(channel).map(|r| r.1).unwrap_or(0)
    }

    pub fn muted(&self, channel: &str) -> bool {
        self.settings.muted(channel, self.guild_of_channel(channel))
    }

    /// The server a channel belongs to, when known (none for DMs).
    pub fn guild_of_channel(&self, channel: &str) -> Option<&str> {
        self.guild_of.get(channel).map(String::as_str)
    }

    /// A message arrived. `mentions_me` counts it as a mention (always true in DMs).
    pub fn on_message(&mut self, channel: &str, guild: Option<&str>, id: u64, mentions_me: bool) {
        self.last.insert(channel.to_string(), id);
        if let Some(g) = guild {
            self.guild_of.insert(channel.to_string(), g.to_string());
        }
        // A channel seen for the first time starts read up to just before this message.
        let entry = self.read.entry(channel.to_string()).or_insert((id.saturating_sub(1), 0));
        if mentions_me {
            entry.1 += 1;
        }
    }

    /// A fresher "newest message" for a server channel (from its channel list).
    pub fn note_last(&mut self, channel: &str, guild: &str, id: u64) {
        let last = self.last.entry(channel.to_string()).or_insert(0);
        *last = (*last).max(id);
        self.guild_of.insert(channel.to_string(), guild.to_string());
    }

    /// Read up to `id` (opened here, or read elsewhere: MESSAGE_ACK).
    pub fn mark_read(&mut self, channel: &str, id: u64) {
        let id = id.max(self.last.get(channel).copied().unwrap_or(0));
        self.read.insert(channel.to_string(), (id, 0));
    }

    pub fn last_message(&self, channel: &str) -> u64 {
        self.last.get(channel).copied().unwrap_or(0)
    }

    /// A server's badge: unread anywhere not muted, mentions anywhere.
    pub fn guild(&self, guild: &str) -> Badge {
        let mut badge = Badge::default();
        for (cid, g) in &self.guild_of {
            if g != guild {
                continue;
            }
            badge.mentions += self.mentions(cid);
            badge.unread |= self.is_unread(cid) && !self.muted(cid);
        }
        badge
    }

    /// The DM side's badge: unread DMs count as mentions, like Discord.
    pub fn dms(&self, dm_channels: &[String]) -> Badge {
        let mentions = dm_channels.iter().map(|c| self.mentions(c)).sum();
        Badge { unread: mentions > 0, mentions }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ready() -> Value {
        json!({
            "read_state": { "entries": [
                { "id": "c1", "last_message_id": "100", "mention_count": 0 },
                { "id": "c2", "last_message_id": "200", "mention_count": 2 },
                { "id": "c3", "last_message_id": "300", "mention_count": 0 },
                { "id": "d1", "last_message_id": "50", "mention_count": 1 },
            ] },
            "guilds": [
                { "id": "g1", "channels": [
                    { "id": "c1", "last_message_id": "150" },
                    { "id": "c2", "last_message_id": "200" },
                    { "id": "c9", "last_message_id": "999" },
                ] },
                { "id": "g2", "channels": [{ "id": "c3", "last_message_id": "400" }] },
            ],
            "private_channels": [{ "id": "d1", "last_message_id": "60" }],
            "user_guild_settings": [{ "guild_id": "g2", "muted": true, "channel_overrides": [] }],
        })
    }

    #[test]
    fn unread_mentions_and_muted_servers() {
        let u = Unread::from_ready(&ready());
        assert!(u.is_unread("c1"));
        assert!(!u.is_unread("c2"));
        assert!(!u.is_unread("c9"), "never opened: no read state, no dot");
        assert_eq!(u.guild("g1"), Badge { unread: true, mentions: 2 });
        // Muted: no white pill even though c3 is unread.
        assert_eq!(u.guild("g2"), Badge { unread: false, mentions: 0 });
        assert_eq!(u.dms(&["d1".into()]), Badge { unread: true, mentions: 1 });
    }

    #[test]
    fn a_muted_channel_lights_no_pill_and_follows_updates() {
        let mut u = Unread::from_ready(&ready());
        u.settings.apply(&json!({ "guild_id": "g1", "channel_overrides": [{ "channel_id": "c1", "muted": true }] }), chrono::Utc::now());
        assert!(u.muted("c1") && !u.muted("c2"));
        assert_eq!(u.guild("g1"), Badge { unread: false, mentions: 2 }, "c1 is the only unread one, and it's muted");
        u.settings.apply(&json!({ "guild_id": "g1", "channel_overrides": [] }), chrono::Utc::now());
        assert!(u.guild("g1").unread);
    }

    #[test]
    fn messages_and_reading() {
        let mut u = Unread::from_ready(&ready());
        u.on_message("c2", Some("g1"), 250, true);
        assert!(u.is_unread("c2"));
        assert_eq!(u.mentions("c2"), 3);
        u.mark_read("c2", 0);
        assert!(!u.is_unread("c2"));
        assert_eq!(u.guild("g1").mentions, 0);
        // A brand-new channel: its first message is unread.
        u.on_message("new", None, 500, true);
        assert!(u.is_unread("new"));
        assert_eq!(u.mentions("new"), 1);
    }
}
