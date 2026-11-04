//! Server invites: reading a code out of what was typed or written in a chat, the requests for
//! a preview, for joining and for a new invite, and what a preview says.

use base64::Engine;
use serde_json::{json, Value};

use crate::discord::snowflake;

/// The most invite cards drawn under one message.
pub const MAX_PER_MESSAGE: usize = 3;
/// What a new invite is: a day long, any number of uses, a full member (not temporary).
const NEW_INVITE_SECS: u64 = 86_400;

/// Whether a code is one Discord could have made: 2 to 32 letters, digits or dashes.
pub fn valid_code(code: &str) -> bool {
    (2..=32).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The code of "discord.gg/CODE", "https://discord.com/invite/CODE" or a bare "CODE"; none for anything else.
pub fn code_of(text: &str) -> Option<String> {
    // Addresses are read whatever their case; the code keeps its own.
    let strip = |s: &str, prefix: &str| s.get(..prefix.len()).filter(|head| head.eq_ignore_ascii_case(prefix)).map(|_| s[prefix.len()..].to_string());
    let text = text.trim();
    let rest = strip(text, "https://").or_else(|| strip(text, "http://")).unwrap_or_else(|| text.to_string());
    let rest = strip(&rest, "www.").unwrap_or(rest);
    let lower = rest.to_ascii_lowercase();
    let after = ["discord.gg/", "discord.com/invite/", "discordapp.com/invite/", "ptb.discord.com/invite/", "canary.discord.com/invite/"]
        .iter()
        .find(|p| lower.starts_with(*p))
        .map(|p| &rest[p.len()..]);
    let code = match after {
        Some(code) => code.split(['?', '#']).next().unwrap_or("").trim_end_matches('/'),
        // Something with a slash or a dot that isn't one of those addresses is not a code.
        None if rest.contains(['/', '.']) => return None,
        None => rest.as_str(),
    };
    valid_code(code).then(|| code.to_string())
}

/// The invite links written in a message, each once, in order.
pub fn codes_in(text: &str) -> Vec<String> {
    static FIND: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let find = crate::regex(&FIND, r"(?i)(?:https?://)?(?:www\.)?(?:discord\.gg|(?:ptb\.|canary\.)?discord(?:app)?\.com/invite)/([A-Za-z0-9-]{2,32})\b");
    let mut out: Vec<String> = Vec::new();
    for c in find.captures_iter(text) {
        let code = c[1].to_string();
        if !out.contains(&code) && out.len() < MAX_PER_MESSAGE {
            out.push(code);
        }
    }
    out
}

/// The invite's address to share.
pub fn link(code: &str) -> String {
    format!("https://discord.gg/{code}")
}

fn checked(code: &str) -> crate::discord::Result<&str> {
    if valid_code(code) {
        Ok(code)
    } else {
        Err(format!("invalid invite code: {code}"))
    }
}

/// Where a preview is asked: the server, its icon and how many are in it.
pub fn preview_path(code: &str) -> crate::discord::Result<String> {
    Ok(format!("/invites/{}?with_counts=true", checked(code)?))
}

/// The path and body of joining by an invite.
pub fn join_request(code: &str) -> crate::discord::Result<(String, Value)> {
    Ok((format!("/invites/{}", checked(code)?), json!({})))
}

/// The path and body of a new invite to a channel.
pub fn create_request(channel: &str) -> crate::discord::Result<(String, Value)> {
    Ok((format!("/channels/{}/invites", snowflake(channel)?), json!({ "max_age": NEW_INVITE_SECS, "max_uses": 0, "temporary": false })))
}

/// Where in the client a join came from, as Discord's own client says it (X-Context-Properties).
pub fn join_context(p: &Preview) -> String {
    let body = json!({
        "location": "Join Guild",
        "location_guild_id": p.guild_id,
        "location_channel_id": p.channel_id,
        "location_channel_type": p.channel_type,
    });
    base64::engine::general_purpose::STANDARD.encode(body.to_string())
}

/// Discord wants a captcha solved before it lets this join happen: that is left to its own client.
pub fn needs_captcha(error: &str) -> bool {
    error.contains("captcha_key") || error.contains("captcha_sitekey")
}

/// What an invite leads to.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub code: String,
    pub guild_id: String,
    pub name: String,
    pub icon: Option<String>,
    pub members: Option<u64>,
    pub online: Option<u64>,
    pub channel_id: String,
    pub channel_type: u64,
}

/// The preview from Discord's answer; none when it leads to no server (a group DM's invite, say).
pub fn parse(v: &Value) -> Option<Preview> {
    let code = v["code"].as_str().filter(|c| valid_code(c))?;
    let guild = &v["guild"];
    let guild_id = snowflake(guild["id"].as_str()?).ok()?;
    let icon = guild["icon"]
        .as_str()
        .filter(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .map(|h| format!("{}/icons/{guild_id}/{h}.png?size=128", crate::CDN));
    Some(Preview {
        code: code.to_string(),
        guild_id: guild_id.to_string(),
        name: crate::readable(guild["name"].as_str().unwrap_or("")),
        icon,
        members: v["approximate_member_count"].as_u64(),
        online: v["approximate_presence_count"].as_u64(),
        channel_id: v["channel"]["id"].as_str().and_then(|c| snowflake(c).ok()).unwrap_or("").to_string(),
        channel_type: v["channel"]["type"].as_u64().unwrap_or(0),
    })
}

/// "В сети: 12 · Участников: 340" (each part only when Discord said it).
pub fn counts_line(p: &Preview) -> String {
    let mut parts = Vec::new();
    if let Some(n) = p.online {
        parts.push(format!("{} {n}", crate::i18n::t("В сети:")));
    }
    if let Some(n) = p.members {
        parts.push(format!("{} {n}", crate::i18n::t("Участников:")));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_read_from_links_and_bare_text() {
        for (text, code) in [
            ("discord.gg/abc123", "abc123"),
            ("https://discord.gg/Nimbus-Dev", "Nimbus-Dev"),
            ("  https://discord.com/invite/XyZ42/  ", "XyZ42"),
            ("http://www.discord.com/invite/rust?event=1", "rust"),
            ("https://discordapp.com/invite/old0ne#top", "old0ne"),
            ("HTTPS://DISCORD.GG/CaseKept", "CaseKept"),
            ("hello-world", "hello-world"),
        ] {
            assert_eq!(code_of(text).as_deref(), Some(code), "{text}");
        }
        for text in ["", "a", "https://example.com/abc", "discord.gg/", "discord.gg/../../users/@me", "abc def", "code_with_underscore", &"x".repeat(33), "evil.com/abc"] {
            assert_eq!(code_of(text), None, "{text}");
        }
        assert!(valid_code("ab") && valid_code(&"z".repeat(32)) && !valid_code("a/b") && !valid_code("ab?c"));
    }

    #[test]
    fn invite_links_are_found_in_messages() {
        let text = "заходите https://discord.gg/abc123 и discord.com/invite/Rust-Ru, ещё раз discord.gg/abc123; не это https://example.com/invite/x";
        assert_eq!(codes_in(text), ["abc123", "Rust-Ru"]);
        assert!(codes_in("просто текст").is_empty());
        let many = (0..5).map(|i| format!("discord.gg/code{i} ")).collect::<String>();
        assert_eq!(codes_in(&many).len(), MAX_PER_MESSAGE);
    }

    #[test]
    fn requests_check_what_goes_into_the_path() {
        assert_eq!(preview_path("abc123").unwrap(), "/invites/abc123?with_counts=true");
        assert!(preview_path("../users/@me").is_err());
        let (path, body) = join_request("abc123").unwrap();
        assert_eq!((path.as_str(), body), ("/invites/abc123", json!({})));
        assert!(join_request("a").is_err());
        let (path, body) = create_request("111222333").unwrap();
        assert_eq!(path, "/channels/111222333/invites");
        assert_eq!(body, json!({ "max_age": 86400, "max_uses": 0, "temporary": false }));
        assert!(create_request("1/2").is_err());
        assert_eq!(link("abc123"), "https://discord.gg/abc123");
    }

    #[test]
    fn previews_are_read_and_checked() {
        let answer = json!({
            "code": "abc123",
            "guild": { "id": "81384788765712384", "name": "Nimbus Dev", "icon": "a_1b2c3d" },
            "channel": { "id": "81384788765712385", "type": 0 },
            "approximate_member_count": 340,
            "approximate_presence_count": 12,
        });
        let p = parse(&answer).unwrap();
        assert_eq!(p.name, "Nimbus Dev");
        assert_eq!(p.icon.as_deref(), Some("https://cdn.discordapp.com/icons/81384788765712384/a_1b2c3d.png?size=128"));
        assert_eq!((p.members, p.online), (Some(340), Some(12)));
        // The words follow the language; the numbers and their order don't.
        let line = counts_line(&p);
        assert!(line.ends_with(" 340") && line.contains(" 12 · "), "{line}");
        assert_eq!(counts_line(&Preview { online: None, ..p.clone() }).matches('·').count(), 0);
        let mut odd = answer.clone();
        odd["guild"]["icon"] = json!("../../x");
        assert_eq!(parse(&odd).unwrap().icon, None, "an odd icon hash is not fetched");
        odd["guild"]["id"] = json!("12/../3");
        assert!(parse(&odd).is_none());
        assert!(parse(&json!({ "code": "abc123", "channel": { "id": "1", "type": 3 } })).is_none(), "a group's invite has no server");
        let context: Value = serde_json::from_slice(&base64::engine::general_purpose::STANDARD.decode(join_context(&p)).unwrap()).unwrap();
        assert_eq!(context["location"], "Join Guild");
        assert_eq!(context["location_guild_id"], "81384788765712384");
        assert_eq!(context["location_channel_id"], "81384788765712385");
    }

    #[test]
    fn a_captcha_answer_is_recognised() {
        assert!(needs_captcha(r#"Discord API 400 Bad Request: {"captcha_key": ["captcha-required"], "captcha_sitekey": "x"}"#));
        assert!(!needs_captcha(r#"Discord API 404 Not Found: {"message": "Unknown Invite", "code": 10006}"#));
    }
}
