//! The open chat as it changes: who's typing, reactions under messages.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Discord's typing indicator lasts 10 seconds unless renewed.
pub const TYPING_FOR: Duration = Duration::from_secs(10);
/// We tell Discord we're typing at most this often (it renews for 10 s).
pub const TYPING_EVERY: Duration = Duration::from_secs(8);

/// Who's typing where: channel -> (user id, name, until).
#[derive(Default)]
pub struct Typing {
    who: HashMap<String, Vec<(String, String, Instant)>>,
}

impl Typing {
    pub fn start(&mut self, channel: &str, user: &str, name: &str, now: Instant) {
        let list = self.who.entry(channel.to_string()).or_default();
        list.retain(|(id, ..)| id != user);
        list.push((user.to_string(), name.to_string(), now + TYPING_FOR));
    }

    /// Their message arrived: they stopped typing.
    pub fn stop(&mut self, channel: &str, user: &str) {
        if let Some(list) = self.who.get_mut(channel) {
            list.retain(|(id, ..)| id != user);
        }
    }

    /// The line under the message box; empty when nobody's typing.
    pub fn line(&self, channel: &str, now: Instant) -> String {
        let names: Vec<&str> = self
            .who
            .get(channel)
            .into_iter()
            .flatten()
            .filter(|(.., until)| *until > now)
            .map(|(_, name, _)| name.as_str())
            .collect();
        match names.as_slice() {
            [] => String::new(),
            [a] => crate::i18n::tf("{} печатает…", &[a]),
            [a, b] => crate::i18n::tf("{} и {} печатают…", &[a, b]),
            [a, b, c] => crate::i18n::tf("{}, {} и {} печатают…", &[a, b, c]),
            _ => crate::i18n::t("Несколько человек печатают…"),
        }
    }
}

/// One reaction under a message.
#[derive(Clone, Debug, PartialEq)]
pub struct Reaction {
    /// How the API names it: the emoji itself, or `name:id` for a custom one.
    pub key: String,
    /// What's drawn when there's no picture: the emoji, or `:name:`.
    pub text: String,
    /// A custom emoji's picture.
    pub image: Option<String>,
    pub count: u32,
    /// We reacted with it too.
    pub me: bool,
}

const CDN: &str = "https://cdn.discordapp.com";

/// The id of a custom emoji, from its key (`name:id`); None for plain emoji.
pub fn emoji_id(key: &str) -> Option<&str> {
    let (_, id) = key.rsplit_once(':')?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

/// A custom emoji's picture by its id: animated WebP for animated ones, a still WebP otherwise.
pub fn emoji_picture(id: &str, animated: bool) -> String {
    if animated {
        format!("{CDN}/emojis/{id}.webp?size=48&animated=true")
    } else {
        format!("{CDN}/emojis/{id}.webp?size=48")
    }
}

/// The emoji id in the address of its picture (`.../emojis/{id}.webp?...`).
pub fn emoji_id_in_url(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("/emojis/")?;
    let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    (!id.is_empty()).then_some(id)
}

/// A custom emoji's still picture, from its key (`name:id`); None for plain emoji.
pub fn emoji_url(key: &str) -> Option<String> {
    emoji_id(key).map(|id| emoji_picture(id, false))
}

fn reaction_of(emoji: &Value) -> Reaction {
    let name = emoji["name"].as_str().unwrap_or("?");
    match emoji["id"].as_str().filter(|id| emoji_id(&format!(":{id}")).is_some()) {
        Some(id) => Reaction {
            key: format!("{name}:{id}"),
            text: format!(":{name}:"),
            image: Some(emoji_picture(id, emoji["animated"].as_bool().unwrap_or(false))),
            count: 0,
            me: false,
        },
        None => Reaction { key: name.to_string(), text: name.to_string(), image: None, count: 0, me: false },
    }
}

/// The reactions a message arrives with.
pub fn reactions_of(m: &Value) -> Vec<Reaction> {
    m["reactions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| Reaction {
            count: r["count"].as_u64().unwrap_or(1) as u32,
            me: r["me"].as_bool().unwrap_or(false),
            ..reaction_of(&r["emoji"])
        })
        .filter(|r| r.count > 0)
        .collect()
}

/// MESSAGE_REACTION_ADD / _REMOVE / _REMOVE_EMOJI / _REMOVE_ALL on a message's list.
pub fn apply_reaction(list: &mut Vec<Reaction>, kind: &str, event: &Value, me: &str) {
    if kind == "MESSAGE_REACTION_REMOVE_ALL" {
        list.clear();
        return;
    }
    let incoming = reaction_of(&event["emoji"]);
    let mine = event["user_id"].as_str() == Some(me);
    let at = list.iter().position(|r| r.key == incoming.key);
    match (kind, at) {
        ("MESSAGE_REACTION_ADD", Some(i)) => {
            // Our own click already counted it (see `toggle`).
            if !(mine && list[i].me) {
                list[i].count += 1;
            }
            list[i].me |= mine;
        }
        ("MESSAGE_REACTION_ADD", None) => list.push(Reaction { count: 1, me: mine, ..incoming }),
        ("MESSAGE_REACTION_REMOVE", Some(i)) => {
            if mine && !list[i].me {
                return; // already taken back by our click
            }
            list[i].count = list[i].count.saturating_sub(1);
            if mine {
                list[i].me = false;
            }
            if list[i].count == 0 {
                list.remove(i);
            }
        }
        ("MESSAGE_REACTION_REMOVE_EMOJI", Some(i)) => {
            list.remove(i);
        }
        _ => {}
    }
}

/// Our click on a reaction, shown before Discord answers. Returns whether we add it.
pub fn toggle(list: &mut Vec<Reaction>, key: &str) -> Option<bool> {
    // A reaction nobody has used yet: ours starts it (picked from the emoji panel).
    let Some(i) = list.iter().position(|r| r.key == key) else {
        // A server emoji ("name:id") is drawn from its picture, a plain one from its character.
        let text = if emoji_url(key).is_some() { format!(":{}:", key.split(':').next().unwrap_or(key)) } else { key.to_string() };
        list.push(Reaction { key: key.to_string(), text, image: emoji_url(key), count: 1, me: true });
        return Some(true);
    };
    let r = &mut list[i];
    if r.me {
        r.me = false;
        r.count = r.count.saturating_sub(1);
        if r.count == 0 {
            list.remove(i);
        }
        Some(false)
    } else {
        r.me = true;
        r.count += 1;
        Some(true)
    }
}

/// The emoji part of `/reactions/{emoji}/@me`.
pub fn reaction_path(key: &str) -> String {
    key.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'~' | b':' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// One emoji of a message made of emoji only, drawn big.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// `name:id` for a custom emoji, the character(s) for a plain one.
    pub key: String,
    /// What's drawn until (or instead of) the picture: the emoji, or `:name:`.
    pub text: String,
    /// A custom emoji's picture.
    pub image: Option<String>,
}

/// Discord draws a message big when it is nothing but emoji, up to this many.
pub const JUMBO_MAX: usize = 30;

/// The emoji of a message that holds nothing else (spaces aside); None when there is any text.
pub fn jumbo(content: &str) -> Option<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut rest = content.trim_start();
    while !rest.is_empty() {
        if let Some((piece, after)) = custom_emoji(rest) {
            pieces.push(piece);
            rest = after;
        } else {
            let (glyph, after) = emoji_glyph(rest)?;
            pieces.push(Piece { key: glyph.to_string(), text: glyph.to_string(), image: None });
            rest = after;
        }
        if pieces.len() > JUMBO_MAX {
            return None;
        }
        rest = rest.trim_start();
    }
    (!pieces.is_empty()).then_some(pieces)
}

/// `<:name:id>` or `<a:name:id>` at the start of `s`, and what follows it.
pub fn custom_emoji(s: &str) -> Option<(Piece, &str)> {
    let body = s.strip_prefix('<')?;
    let end = body.find('>')?;
    let (inner, after) = (&body[..end], &body[end + 1..]);
    let (animated, inner) = match inner.strip_prefix("a:") {
        Some(inner) => (true, inner),
        None => (false, inner.strip_prefix(':')?),
    };
    let (name, id) = inner.split_once(':')?;
    let name_ok = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    let id_ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit());
    if !(name_ok && id_ok) {
        return None;
    }
    Some((Piece { key: format!("{name}:{id}"), text: format!(":{name}:"), image: Some(emoji_picture(id, animated)) }, after))
}

/// Characters that are emoji by themselves (pictographs, symbols, arrows).
fn is_pictograph(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x2190..=0x21FF | 0x2300..=0x23FF | 0x2900..=0x297F
        | 0x25A0..=0x25FF | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0xA9 | 0xAE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x24C2)
}

/// What changes the emoji before it: the emoji style, skin tones, a keycap, the tags of a flag.
fn is_emoji_modifier(c: char) -> bool {
    matches!(c as u32, 0xFE0F | 0xFE0E | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F)
}

/// One emoji (with its modifiers, joined parts or flag pair) at the start of `s`, and what follows it.
fn emoji_glyph(s: &str) -> Option<(&str, &str)> {
    let regional = |c: char| (0x1F1E6..=0x1F1FF).contains(&(c as u32));
    let mut chars = s.char_indices().peekable();
    let (_, first) = chars.next()?;
    let keycap = first.is_ascii_digit() || first == '#' || first == '*';
    if !is_pictograph(first) && !keycap {
        return None;
    }
    let (mut end, mut joined, mut half_flag) = (first.len_utf8(), false, regional(first));
    while let Some(&(i, c)) = chars.peek() {
        let take = if joined {
            joined = false;
            is_pictograph(c)
        } else if c == '\u{200D}' {
            joined = true;
            true
        } else if is_emoji_modifier(c) {
            true
        } else if half_flag && regional(c) {
            half_flag = false;
            true
        } else {
            false
        };
        if !take {
            break;
        }
        end = i + c.len_utf8();
        chars.next();
    }
    // A digit is an emoji only as a keycap (1️⃣).
    if keycap && !s[..end].contains('\u{20E3}') {
        return None;
    }
    Some((&s[..end], &s[end..]))
}

/// The name TYPING_START gives us: a server nickname, a display name, or the cached user.
pub fn typing_name(event: &Value, known: Option<&Value>) -> Option<String> {
    let member = &event["member"];
    member["nick"]
        .as_str()
        .or(member["user"]["global_name"].as_str())
        .or(member["user"]["username"].as_str())
        .or_else(|| known.and_then(|u| u["global_name"].as_str().or(u["username"].as_str())))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typing_lines() {
        let mut t = Typing::default();
        let now = Instant::now();
        assert_eq!(t.line("c", now), "");
        t.start("c", "1", "demo_carol", now);
        assert_eq!(t.line("c", now), "demo_carol печатает…");
        t.start("c", "2", "demo_bob", now);
        assert_eq!(t.line("c", now), "demo_carol и demo_bob печатают…");
        t.start("c", "3", "demo_zed", now);
        t.start("c", "4", "demo_alice", now);
        assert_eq!(t.line("c", now), "Несколько человек печатают…");
        t.stop("c", "3");
        t.stop("c", "4");
        t.stop("c", "1");
        assert_eq!(t.line("c", now), "demo_bob печатает…");
        assert_eq!(t.line("other", now), "");
        // Ten seconds later it's gone by itself.
        assert_eq!(t.line("c", now + TYPING_FOR + Duration::from_millis(1)), "");
    }

    #[test]
    fn reactions_come_and_go() {
        let m = json!({ "reactions": [
            { "emoji": { "id": null, "name": "👍" }, "count": 2, "me": true },
            { "emoji": { "id": "77", "name": "pog", "animated": false }, "count": 1, "me": false },
        ] });
        let mut list = reactions_of(&m);
        assert_eq!(list[0].key, "👍");
        assert_eq!(list[1].key, "pog:77");
        assert_eq!(list[1].image.as_deref(), Some("https://cdn.discordapp.com/emojis/77.webp?size=48"));

        let add = |emoji: Value, user: &str| json!({ "emoji": emoji, "user_id": user });
        apply_reaction(&mut list, "MESSAGE_REACTION_ADD", &add(json!({ "id": "77", "name": "pog" }), "9"), "me");
        assert_eq!(list[1].count, 2);
        apply_reaction(&mut list, "MESSAGE_REACTION_ADD", &add(json!({ "id": null, "name": "🔥" }), "me"), "me");
        assert_eq!(list[2], Reaction { key: "🔥".into(), text: "🔥".into(), image: None, count: 1, me: true });
        apply_reaction(&mut list, "MESSAGE_REACTION_REMOVE", &add(json!({ "id": null, "name": "🔥" }), "me"), "me");
        assert_eq!(list.len(), 2);
        apply_reaction(&mut list, "MESSAGE_REACTION_REMOVE_EMOJI", &json!({ "emoji": { "id": "77", "name": "pog" } }), "me");
        assert_eq!(list.len(), 1);
        apply_reaction(&mut list, "MESSAGE_REACTION_REMOVE_ALL", &json!({}), "me");
        assert!(list.is_empty());
    }

    #[test]
    fn our_click_is_not_counted_twice() {
        let mut list = reactions_of(&json!({ "reactions": [{ "emoji": { "id": null, "name": "👍" }, "count": 1, "me": false }] }));
        assert_eq!(toggle(&mut list, "👍"), Some(true));
        assert_eq!((list[0].count, list[0].me), (2, true));
        // Then the gateway tells us about our own reaction.
        apply_reaction(&mut list, "MESSAGE_REACTION_ADD", &json!({ "emoji": { "id": null, "name": "👍" }, "user_id": "me" }), "me");
        assert_eq!(list[0].count, 2);
        assert_eq!(toggle(&mut list, "👍"), Some(false));
        apply_reaction(&mut list, "MESSAGE_REACTION_REMOVE", &json!({ "emoji": { "id": null, "name": "👍" }, "user_id": "me" }), "me");
        assert_eq!(list[0].count, 1);
        // A reaction picked from the emoji panel that nobody used yet starts with us.
        assert_eq!(toggle(&mut list, "🎉"), Some(true));
        assert_eq!((list[1].key.as_str(), list[1].count, list[1].me), ("🎉", 1, true));
        assert_eq!(toggle(&mut list, "🎉"), Some(false));
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn reaction_paths_and_typing_names() {
        assert_eq!(reaction_path("👍"), "%F0%9F%91%8D");
        assert_eq!(reaction_path("pog:77"), "pog:77");
        assert_eq!(emoji_url("👍"), None);
        assert_eq!(emoji_url("a:b:9").as_deref(), Some("https://cdn.discordapp.com/emojis/9.webp?size=48"));
        let ev = json!({ "member": { "nick": null, "user": { "global_name": "Голд", "username": "goldie" } } });
        assert_eq!(typing_name(&ev, None).as_deref(), Some("Голд"));
        let dm = json!({ "user_id": "5" });
        assert_eq!(typing_name(&dm, Some(&json!({ "username": "demo_carol" }))).as_deref(), Some("demo_carol"));
        assert_eq!(typing_name(&dm, None), None);
    }

    #[test]
    fn animated_reactions_get_the_animated_picture() {
        let m = json!({ "reactions": [{ "emoji": { "id": "88", "name": "dance", "animated": true }, "count": 1, "me": false }] });
        let list = reactions_of(&m);
        assert_eq!(list[0].image.as_deref(), Some("https://cdn.discordapp.com/emojis/88.webp?size=48&animated=true"));
        assert_eq!(emoji_id(&list[0].key), Some("88"));
        assert_eq!(emoji_id("👍"), None);
        assert_eq!(emoji_id_in_url(list[0].image.as_deref().unwrap()).as_deref(), Some("88"));
        assert_eq!(emoji_id_in_url("https://cdn.discordapp.com/avatars/1/a.png"), None);
    }

    #[test]
    fn custom_emoji_are_read_from_text() {
        let (piece, rest) = custom_emoji("<a:party_blob:123> hi").unwrap();
        assert_eq!(piece.key, "party_blob:123");
        assert_eq!(piece.text, ":party_blob:");
        assert_eq!(piece.image.as_deref(), Some("https://cdn.discordapp.com/emojis/123.webp?size=48&animated=true"));
        assert_eq!(rest, " hi");
        let (still, _) = custom_emoji("<:pog:77>").unwrap();
        assert_eq!(still.image.as_deref(), Some("https://cdn.discordapp.com/emojis/77.webp?size=48"));
        assert!(custom_emoji("<@123>").is_none());
        assert!(custom_emoji("<:pog:7x7>").is_none());
        assert!(custom_emoji("<:po g:77>").is_none());
    }

    #[test]
    fn only_emoji_messages_are_drawn_big() {
        let pieces = jumbo(" <:pog:77> 🔥<a:dance:88>👍🏽 🇷🇺 1️⃣ 👨‍👩‍👧 ").unwrap();
        let keys: Vec<&str> = pieces.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["pog:77", "🔥", "dance:88", "👍🏽", "🇷🇺", "1️⃣", "👨‍👩‍👧"]);
        assert_eq!(pieces[1].image, None);
        assert!(jumbo("hi <:pog:77>").is_none());
        assert!(jumbo("<:pog:77> 1").is_none(), "a bare digit is text");
        assert!(jumbo("").is_none());
        assert!(jumbo(&"🔥".repeat(JUMBO_MAX)).is_some());
        assert!(jumbo(&"🔥".repeat(JUMBO_MAX + 1)).is_none());
    }
}
