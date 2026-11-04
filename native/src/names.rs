//! Names of server roles and channels, for writing out `<@&id>` and `<#id>` in messages,
//! and the colour marks a mention carries until the styled text draws it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::Value;

/// Role id → (name, colour as 0xRRGGBB, 0 for none).
static ROLES: Mutex<Option<HashMap<String, (String, u32)>>> = Mutex::new(None);
/// Channel id → name.
static CHANNELS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// A server's role list, as READY or `/guilds/{id}/roles` gives it.
pub fn note_roles(roles: &[Value]) {
    let mut table = ROLES.lock().unwrap();
    let table = table.get_or_insert_with(HashMap::new);
    for r in roles {
        note_role_in(table, r);
    }
}

/// One role, as a GUILD_ROLE_CREATE or _UPDATE brings it.
pub fn note_role(r: &Value) {
    note_role_in(ROLES.lock().unwrap().get_or_insert_with(HashMap::new), r);
}

fn note_role_in(table: &mut HashMap<String, (String, u32)>, r: &Value) {
    if let (Some(id), Some(name)) = (r["id"].as_str(), r["name"].as_str()) {
        let color = r["colors"]["primary_color"].as_u64().or(r["color"].as_u64()).unwrap_or(0) as u32;
        table.insert(id.to_string(), (name.to_string(), color));
    }
}

/// A server's channels (or one channel).
pub fn note_channels(channels: &[Value]) {
    let mut table = CHANNELS.lock().unwrap();
    let table = table.get_or_insert_with(HashMap::new);
    for c in channels {
        if let (Some(id), Some(name)) = (c["id"].as_str(), c["name"].as_str()) {
            table.insert(id.to_string(), name.to_string());
        }
    }
}

pub fn role(id: &str) -> Option<(String, u32)> {
    ROLES.lock().unwrap().as_ref()?.get(id).cloned()
}

pub fn channel(id: &str) -> Option<String> {
    CHANNELS.lock().unwrap().as_ref()?.get(id).cloned()
}

const OPEN: char = '\u{E000}';
const MID: char = '\u{E001}';
const CLOSE: char = '\u{E002}';
const USER: char = '\u{E003}';

/// The colour of a user or channel mention, and of a role without its own colour: Discord's blurple, pale on dark themes.
const MENTION_COLOR: u32 = 0xffffff;
const MENTION_COLOR_LIGHT: u32 = 0x111111;
static LIGHT: AtomicBool = AtomicBool::new(false);

/// Whether the light theme is on (messages opened after this use its mention colour).
pub fn set_light(on: bool) {
    LIGHT.store(on, Ordering::Relaxed);
}

pub fn mention_color() -> u32 {
    if LIGHT.load(Ordering::Relaxed) { MENTION_COLOR_LIGHT } else { MENTION_COLOR }
}

/// `text` marked to be drawn in `color`.
pub fn mark(text: &str, color: u32) -> String {
    format!("{OPEN}{:06x}{MID}{text}{CLOSE}", color & 0xff_ffff)
}

/// A mention of a person, marked with whose it is: a click on it opens their card.
pub fn mark_user(text: &str, color: u32, id: &str) -> String {
    format!("{OPEN}{:06x}{USER}{id}{MID}{text}{CLOSE}", color & 0xff_ffff)
}

/// The marks dropped, keeping their text: for notifications, previews and plain text.
pub fn unmark(s: &str) -> String {
    if !s.contains(OPEN) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            OPEN => {
                for c in chars.by_ref() {
                    if c == MID {
                        break;
                    }
                }
            }
            CLOSE => {}
            c => out.push(c),
        }
    }
    out
}

/// The marks as styled-text colour tags (`<font color="#rrggbb">…</font>`), on text whose own `<` are escaped.
pub fn marks_to_font(s: &str) -> String {
    if !s.contains(OPEN) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 32);
    let mut link = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            OPEN => {
                let head: String = chars.by_ref().take_while(|&c| c != MID).collect();
                let (hex, user) = head.split_once(USER).unwrap_or((head.as_str(), ""));
                out.push_str(&format!("<font color=\"#{hex}\">"));
                if !user.is_empty() {
                    out.push('[');
                    link = Some(user.to_string());
                }
            }
            CLOSE => {
                if let Some(user) = link.take() {
                    out.push_str(&format!("]({}{user})", crate::userprofile::LINK_PREFIX));
                }
                out.push_str("</font>");
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn marks_round_trip() {
        let s = format!("hi {} and {}", mark("@Админ", 0x3498db), mark("@bob", 0xc9cdfb));
        assert_eq!(unmark(&s), "hi @Админ and @bob");
        assert_eq!(marks_to_font(&s), "hi <font color=\"#3498db\">@Админ</font> and <font color=\"#c9cdfb\">@bob</font>");
        assert_eq!(unmark("plain"), "plain");
        let m = mark_user("@sam", 0xc9cdfb, "42");
        assert_eq!(unmark(&m), "@sam");
        assert_eq!(marks_to_font(&m), "<font color=\"#c9cdfb\">[@sam](nimbus-user:42)</font>");
    }

    #[test]
    fn roles_and_channels_by_id() {
        note_roles(&[json!({"id": "11", "name": "Mod", "color": 0xe91e63}), json!({"id": "12", "name": "Plain", "color": 0})]);
        note_role(&json!({"id": "13", "name": "New", "colors": {"primary_color": 0x00ff00}}));
        note_channels(&[json!({"id": "21", "name": "general"})]);
        assert_eq!(role("11"), Some(("Mod".into(), 0xe91e63)));
        assert_eq!(role("12"), Some(("Plain".into(), 0)));
        assert_eq!(role("13"), Some(("New".into(), 0x00ff00)));
        assert_eq!(channel("21").as_deref(), Some("general"));
        assert_eq!(role("99"), None);
    }
}
