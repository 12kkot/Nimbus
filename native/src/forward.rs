//! Forwarding a message to another chat, and reading forwarded messages that arrive.

use serde_json::{json, Value};

use crate::discord::snowflake;

/// Discord's kind of message reference that forwards rather than replies.
const FORWARD: u64 = 1;
/// How many places the picker lists at once.
pub const MAX_TARGETS: usize = 60;

/// The path and body that forward `message` (from `channel`, in `guild` if any) to `dest`.
pub fn request(dest: &str, message: &str, channel: &str, guild: Option<&str>) -> Result<(String, Value), String> {
    let mut reference = json!({ "type": FORWARD, "message_id": snowflake(message)?, "channel_id": snowflake(channel)? });
    if let Some(guild) = guild {
        reference["guild_id"] = json!(snowflake(guild)?);
    }
    Ok((format!("/channels/{}/messages", snowflake(dest)?), json!({ "message_reference": reference })))
}

/// What a forwarded message carries: the original as it was (its text, files and previews), or none
/// for an ordinary message.
pub fn snapshot(m: &Value) -> Option<&Value> {
    if m["message_reference"]["type"].as_u64() != Some(FORWARD) {
        return None;
    }
    m["message_snapshots"].as_array()?.first().map(|s| &s["message"]).filter(|s| s.is_object())
}

/// A place a message can be forwarded to.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub id: String,
    pub name: String,
    /// "Личные сообщения" or the server's name.
    pub detail: String,
    pub dm: bool,
}

/// The places whose name has what's typed (any case), the first `MAX_TARGETS` of them.
pub fn filter(all: &[Target], query: &str) -> Vec<Target> {
    let query = query.trim().to_lowercase();
    all.iter().filter(|t| query.is_empty() || t.name.to_lowercase().contains(&query)).take(MAX_TARGETS).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forward_names_the_original_and_checks_ids() {
        let (path, body) = request("300", "100", "200", Some("400")).unwrap();
        assert_eq!(path, "/channels/300/messages");
        assert_eq!(body, json!({ "message_reference": { "type": 1, "message_id": "100", "channel_id": "200", "guild_id": "400" } }));
        let (_, dm) = request("300", "100", "200", None).unwrap();
        assert!(dm["message_reference"].get("guild_id").is_none());
        assert!(request("3/../0", "100", "200", None).is_err());
        assert!(request("300", "", "200", None).is_err());
        assert!(request("300", "100", "200", Some("@me")).is_err());
    }

    #[test]
    fn snapshots_of_forwarded_messages_are_read() {
        let forwarded = json!({
            "content": "",
            "message_reference": { "type": 1, "channel_id": "1", "message_id": "2" },
            "message_snapshots": [{ "message": { "content": "оригинал", "attachments": [] } }],
        });
        assert_eq!(snapshot(&forwarded).unwrap()["content"], "оригинал");
        let reply = json!({ "message_reference": { "type": 0, "message_id": "2" }, "message_snapshots": [{ "message": {} }] });
        assert!(snapshot(&reply).is_none());
        assert!(snapshot(&json!({ "message_reference": { "type": 1 } })).is_none());
        assert!(snapshot(&json!({ "message_reference": { "type": 1 }, "message_snapshots": [{ "message": null }] })).is_none());
    }

    #[test]
    fn places_are_found_by_name() {
        let t = |name: &str, dm: bool| Target { id: name.into(), name: name.into(), detail: String::new(), dm };
        let all = vec![t("general", false), t("Алекс", true), t("мемы", false)];
        assert_eq!(filter(&all, "").len(), 3);
        assert_eq!(filter(&all, " АЛ ").iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["Алекс"]);
        assert!(filter(&all, "нет такого").is_empty());
    }
}
