//! Forum channels: a list of posts, each one a thread opened like a chat.
//! The list comes from Discord's thread search, the way its own client asks.

use std::collections::HashSet;

use serde_json::Value;

/// One post as the forum list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Post {
    pub id: String,
    pub title: String,
    pub author: String,
    /// The start of the first message.
    pub preview: String,
    /// The first message's top reaction and its count (empty when none).
    pub reaction: String,
    pub reactions: u64,
    pub replies: u64,
    /// Since the last message: "54 мин. назад".
    pub when: String,
    /// A picture from the first message, for the thumbnail.
    pub image: Option<String>,
    pub pinned: bool,
}

/// Search results to ask for: active posts and archived ones, newest activity first.
pub fn search_paths(channel: impl std::fmt::Display) -> [String; 2] {
    let base = format!("/channels/{channel}/threads/search?sort_by=last_message_time&sort_order=desc&limit=25&offset=0");
    [format!("{base}&archived=false"), format!("{base}&archived=true")]
}

/// Discord's epoch, in Unix milliseconds; snowflakes count from it.
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;

fn snowflake_ms(id: &str) -> Option<i64> {
    Some((id.parse::<u64>().ok()? >> 22) as i64 + DISCORD_EPOCH_MS)
}

/// "только что", "5 мин. назад", "3 ч. назад", "2 д. назад".
pub fn ago(ms: i64, now_ms: i64) -> String {
    let minutes = (now_ms - ms).max(0) / 60_000;
    match minutes {
        0 => crate::i18n::t("только что"),
        m if m < 60 => crate::i18n::tf("{} мин. назад", &[&m]),
        m if m < 60 * 24 => crate::i18n::tf("{} ч. назад", &[&(m / 60)]),
        m => crate::i18n::tf("{} д. назад", &[&(m / (60 * 24))]),
    }
}

/// The posts from one or more search results, pinned first, then by latest activity.
pub fn posts(results: &[Value], now_ms: i64) -> Vec<Post> {
    let mut seen = HashSet::new();
    let mut out: Vec<(i64, Post)> = Vec::new();
    for found in results {
        let firsts = found["first_messages"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        for t in found["threads"].as_array().into_iter().flatten() {
            let Some(id) = t["id"].as_str() else { continue };
            if !seen.insert(id.to_string()) {
                continue;
            }
            // A post's first message has the thread's own id.
            let first = firsts
                .iter()
                .find(|m| m["id"].as_str() == Some(id) || m["channel_id"].as_str() == Some(id));
            let last = t["last_message_id"]
                .as_str()
                .and_then(snowflake_ms)
                .or_else(|| snowflake_ms(id))
                .unwrap_or(0);
            let top = first
                .map(crate::chat::reactions_of)
                .unwrap_or_default()
                .into_iter()
                .max_by_key(|r| r.count);
            let preview: String = first
                .and_then(|m| m["content"].as_str())
                .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
                .unwrap_or_default()
                .chars()
                .take(160)
                .collect();
            out.push((
                last,
                Post {
                    id: id.to_string(),
                    title: crate::readable(t["name"].as_str().unwrap_or("")),
                    author: first.map(|m| crate::display_name(&m["author"])).unwrap_or_default(),
                    preview: crate::readable(&preview),
                    reaction: top.as_ref().map(|r| r.text.clone()).unwrap_or_default(),
                    reactions: top.as_ref().map(|r| r.count as u64).unwrap_or(0),
                    replies: t["message_count"].as_u64().unwrap_or(0),
                    when: ago(last, now_ms),
                    image: first.and_then(crate::picture_of).map(|p| p.url),
                    pinned: t["flags"].as_u64().unwrap_or(0) & 2 != 0,
                },
            ));
        }
    }
    out.sort_by(|(a_last, a), (b_last, b)| b.pinned.cmp(&a.pinned).then(b_last.cmp(a_last)));
    out.into_iter().map(|(_, p)| p).collect()
}

/// The longest name of a thread or post, and the longest text of a message.
pub const MAX_NAME: usize = 100;
pub const MAX_TEXT: usize = 2000;
/// A new thread is hidden after a day without messages.
const ARCHIVE_MINUTES: u64 = 1440;

/// A thread's or post's name as typed, trimmed; none when empty or longer than Discord takes.
pub fn valid_name(typed: &str) -> Option<String> {
    let name = typed.trim();
    (!name.is_empty() && name.chars().count() <= MAX_NAME).then(|| name.to_string())
}

/// The path and body of a thread started from a message.
pub fn thread_request(channel: &str, message: &str, name: &str) -> crate::discord::Result<(String, Value)> {
    let name = valid_name(name).ok_or("the name must be 1 to 100 characters")?;
    let path = format!("/channels/{}/messages/{}/threads", crate::discord::snowflake(channel)?, crate::discord::snowflake(message)?);
    Ok((path, serde_json::json!({ "name": name, "auto_archive_duration": ARCHIVE_MINUTES })))
}

/// The path and body of a new forum post: its title and its first message.
pub fn post_request(forum: &str, title: &str, text: &str) -> crate::discord::Result<(String, Value)> {
    let name = valid_name(title).ok_or("the title must be 1 to 100 characters")?;
    let text = text.trim();
    if text.is_empty() || text.chars().count() > MAX_TEXT {
        return Err("the text must be 1 to 2000 characters".into());
    }
    let path = format!("/channels/{}/threads", crate::discord::snowflake(forum)?);
    Ok((path, serde_json::json!({ "name": name, "message": { "content": text } })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn thread_names_are_1_to_100_characters() {
        assert_eq!(valid_name("  план  ").as_deref(), Some("план"));
        assert_eq!(valid_name(""), None);
        assert_eq!(valid_name("   "), None);
        assert!(valid_name(&"я".repeat(100)).is_some(), "characters are counted, not bytes");
        assert_eq!(valid_name(&"я".repeat(101)), None);
    }

    #[test]
    fn thread_and_post_requests() {
        let (path, body) = thread_request("111", "222", " обсуждение ").unwrap();
        assert_eq!(path, "/channels/111/messages/222/threads");
        assert_eq!(body, json!({ "name": "обсуждение", "auto_archive_duration": 1440 }));
        assert!(thread_request("111", "2/../2", "x").is_err());
        assert!(thread_request("111", "222", "").is_err());
        let (path, body) = post_request("333", "Мой кот", " вот он\n").unwrap();
        assert_eq!(path, "/channels/333/threads");
        assert_eq!(body, json!({ "name": "Мой кот", "message": { "content": "вот он" } }));
        assert!(post_request("333", "Мой кот", "").is_err());
        assert!(post_request("333", &"x".repeat(101), "текст").is_err());
        assert!(post_request("333", "x", &"y".repeat(2001)).is_err());
        assert!(post_request("abc", "x", "y").is_err());
    }

    #[test]
    fn times_in_words() {
        let now = 10_000_000_000;
        assert_eq!(ago(now - 20_000, now), "только что");
        assert_eq!(ago(now - 54 * 60_000, now), "54 мин. назад");
        assert_eq!(ago(now - 3 * 3_600_000, now), "3 ч. назад");
        assert_eq!(ago(now - 2 * 86_400_000, now), "2 д. назад");
    }

    #[test]
    fn posts_from_search_results() {
        // Snowflakes: time in the high bits.
        let at = |ms: i64| (((ms - DISCORD_EPOCH_MS) as u64) << 22).to_string();
        let now = 1_800_000_000_000;
        let (old, new, pin) = (at(now - 2 * 86_400_000), at(now - 54 * 60_000), at(now - 9 * 86_400_000));
        let active = json!({
            "threads": [
                {"id": old, "name": "Sirius", "message_count": 8, "last_message_id": old},
                {"id": new, "name": "миша", "message_count": 14, "last_message_id": new},
            ],
            "first_messages": [{
                "id": new, "channel_id": new, "content": "мой  кот\nМиша",
                "author": {"id": "1", "username": "spiderkaras"},
                "reactions": [{"emoji": {"name": "❤️"}, "count": 3, "me": false}],
            }],
        });
        let archived = json!({
            "threads": [
                {"id": new, "name": "миша (повтор)", "message_count": 1},
                {"id": pin, "name": "Правила", "flags": 2, "message_count": 0, "last_message_id": pin},
            ],
            "first_messages": [],
        });
        let list = posts(&[active, archived], now);
        let titles: Vec<&str> = list.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Правила", "миша", "Sirius"], "pinned first, then newest, no repeats");
        let misha = &list[1];
        assert_eq!((misha.author.as_str(), misha.preview.as_str()), ("spiderkaras", "мой кот Миша"));
        assert_eq!((misha.reaction.as_str(), misha.reactions, misha.replies), ("❤️", 3, 14));
        assert_eq!(misha.when, "54 мин. назад");
        assert!(list[2].author.is_empty() && list[2].reaction.is_empty());
    }
}
