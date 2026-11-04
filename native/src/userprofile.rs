//! A person's profile card, read from what `/users/{id}/profile` answers (or only from what we know of them).

use serde_json::Value;

/// How the app names a user's pictures and names (these live in main.rs).
pub struct Look<'a> {
    pub name: &'a dyn Fn(&Value) -> String,
    pub avatar: &'a dyn Fn(&Value) -> String,
    pub banner: &'a dyn Fn(&Value) -> Option<String>,
    pub decoration: &'a dyn Fn(&Value) -> Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub username: String,
    pub pronouns: String,
    pub bio: String,
    /// The server tag worn beside the username.
    pub tag: String,
    pub color: Option<u32>,
    pub avatar: String,
    pub banner: Option<String>,
    pub decoration: Option<String>,
    pub badges: Vec<String>,
    /// What each of `badges` says, in the same order.
    pub badge_info: Vec<Badge>,
    pub mutual_friends: Option<u64>,
    pub mutual_servers: Option<u64>,
}

/// What Discord says about a badge.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Badge {
    pub id: String,
    pub text: String,
    /// Where it leads, when that is a page of the web.
    pub link: String,
}

/// A step of a badge that grows: its name, how long it takes, whether it is reached.
#[derive(Clone, Debug, PartialEq)]
pub struct Tier {
    pub name: String,
    pub after: String,
    pub reached: bool,
}

/// The heading and steps of the badges that grow with time; others have none.
pub fn badge_tiers(id: &str) -> Option<(String, Vec<Tier>)> {
    let t = crate::i18n::t;
    let (heading, steps, at): (String, Vec<(String, String)>, usize) = if id == "premium" || id.starts_with("premium_tenure_") {
        let at = match id.strip_prefix("premium_tenure_").and_then(|r| r.split('_').next()) {
            None => 0,
            Some("1") => 1,
            Some("3") => 2,
            Some("6") => 3,
            Some("12") => 4,
            Some("24") => 5,
            Some("36") => 6,
            Some("60") => 7,
            Some("72") => 8,
            Some(_) => return None,
        };
        let steps = vec![
            (t("Начальный"), String::new()),
            (t("Бронза"), t("1 месяц")),
            (t("Серебро"), t("3 месяца")),
            (t("Золото"), t("6 месяцев")),
            (t("Платина"), t("1 год")),
            (t("Алмаз"), t("2 года")),
            (t("Изумруд"), t("3 года")),
            (t("Рубин"), t("5 лет")),
            (t("Опал"), t("От 6 лет")),
        ];
        ("Nitro".to_string(), steps, at)
    } else if let Some(level) = id.strip_prefix("guild_booster_lvl") {
        let level: usize = level.parse().ok().filter(|l| (1..=9).contains(l))?;
        let months = [t("1 месяц"), t("2 месяца"), t("3 месяца"), t("6 месяцев"), t("9 месяцев"), t("12 месяцев"), t("15 месяцев"), t("18 месяцев"), t("От 24 месяцев")];
        (t("Буст сервера"), months.into_iter().map(|after| (String::new(), after)).collect(), level - 1)
    } else {
        return None;
    };
    let tiers = steps.into_iter().enumerate().map(|(n, (name, after))| Tier { name, after, reached: n <= at }).collect();
    Some((heading, tiers))
}

const MAX_BADGES: usize = 40;
const CDN: &str = "https://cdn.discordapp.com";

/// `profile` may be `Null`: then the card has only what `known` (the user as we saw them) says.
pub fn card(id: &str, profile: &Value, known: &Value, look: &Look) -> Card {
    let user = if profile["user"].is_object() { &profile["user"] } else { known };
    let about = &profile["user_profile"];
    let text = |v: &Value| v.as_str().unwrap_or("").to_string();
    let color = about["theme_colors"][0]
        .as_u64()
        .or(user["accent_color"].as_u64())
        .or_else(|| user["banner_color"].as_str().and_then(|c| u64::from_str_radix(c.trim_start_matches('#'), 16).ok()))
        .map(|c| c as u32 & 0xff_ffff);
    let (badges, badge_info): (Vec<String>, Vec<Badge>) = profile["badges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b["icon"].as_str().map(|icon| (b, icon)))
        .filter(|(_, icon)| !icon.is_empty() && icon.bytes().all(|b| b.is_ascii_alphanumeric()))
        .take(MAX_BADGES)
        .map(|(b, icon)| {
            let link = text(&b["link"]);
            let info = Badge { id: text(&b["id"]), text: text(&b["description"]), link: if link.starts_with("https://") { link } else { String::new() } };
            (format!("{CDN}/badge-icons/{icon}.png"), info)
        })
        .unzip();
    Card {
        id: id.to_string(),
        name: (look.name)(user),
        username: text(&user["username"]),
        pronouns: text(&about["pronouns"]),
        bio: text(&about["bio"]),
        tag: text(if user["primary_guild"]["tag"].is_string() { &user["primary_guild"]["tag"] } else { &user["clan"]["tag"] }),
        color,
        avatar: (look.avatar)(user),
        banner: (look.banner)(user),
        decoration: (look.decoration)(user),
        badges,
        badge_info,
        mutual_friends: profile["mutual_friends_count"].as_u64(),
        mutual_servers: profile["mutual_guilds"].as_array().map(|g| g.len() as u64),
    }
}

/// An account someone linked to their profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub kind: String,
    pub name: String,
    pub url: Option<String>,
}

/// Where a linked account can be seen; only plain names go into an address.
fn account_url(kind: &str, id: &str, name: &str) -> Option<String> {
    let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
    let (base, key) = match kind {
        "steam" => ("https://steamcommunity.com/profiles/", id),
        "github" => ("https://github.com/", name),
        "twitch" => ("https://www.twitch.tv/", name),
        "twitter" => ("https://x.com/", name),
        "youtube" => ("https://www.youtube.com/channel/", id),
        "spotify" => ("https://open.spotify.com/user/", id),
        "reddit" => ("https://www.reddit.com/u/", name),
        "tiktok" => ("https://www.tiktok.com/@", name),
        _ => return None,
    };
    plain(key).then(|| format!("{base}{key}"))
}

fn account_kind(kind: &str) -> String {
    let known = [("steam", "Steam"), ("github", "GitHub"), ("twitch", "Twitch"), ("twitter", "X"), ("youtube", "YouTube"), ("spotify", "Spotify"), ("reddit", "Reddit"), ("tiktok", "TikTok"), ("battlenet", "Battle.net"), ("epicgames", "Epic Games"), ("xbox", "Xbox"), ("playstation", "PlayStation"), ("riotgames", "Riot Games"), ("leagueoflegends", "League of Legends"), ("roblox", "Roblox"), ("instagram", "Instagram"), ("facebook", "Facebook"), ("ebay", "eBay"), ("paypal", "PayPal"), ("domain", "Web")];
    known.iter().find(|(k, _)| *k == kind).map_or_else(|| kind.to_string(), |(_, shown)| shown.to_string())
}

/// The accounts a profile shows.
pub fn accounts(profile: &Value) -> Vec<Account> {
    let list = profile["connected_accounts"].as_array().into_iter().flatten();
    list.filter_map(|a| {
        let kind = a["type"].as_str()?;
        let name = a["name"].as_str().unwrap_or("").to_string();
        Some(Account { kind: account_kind(kind), url: account_url(kind, a["id"].as_str().unwrap_or(""), &name), name })
    })
    .collect()
}

/// The address a mention carries in message text.
pub const LINK_PREFIX: &str = "nimbus-user:";

/// The user a clicked link in a message means, when it is a mention.
pub fn user_of_link(url: &str) -> Option<&str> {
    let id = url.strip_prefix(LINK_PREFIX)?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

/// A file name for a saved picture, from its address: only plain characters, always with an extension.
pub fn file_name_of(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let name: String = path.rsplit('/').next().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
    let name = name.trim_matches('.');
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") => name.to_string(),
        _ => "image.png".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn look() -> Look<'static> {
        Look {
            name: &|u| u["global_name"].as_str().or(u["username"].as_str()).unwrap_or("?").to_string(),
            avatar: &|u| format!("avatar-of-{}", u["id"].as_str().unwrap_or("")),
            banner: &|u| u["banner"].as_str().map(String::from),
            decoration: &|_| None,
        }
    }

    #[test]
    fn reads_the_whole_profile() {
        let profile = json!({
            "user": { "id": "5", "username": "pat", "global_name": "Pat", "banner": "b1", "accent_color": 255, "primary_guild": { "tag": "RUST" } },
            "user_profile": { "bio": "hello", "pronouns": "they" },
            "badges": [{ "icon": "abc123" }, { "icon": "../evil" }, { "icon": "def" }],
            "mutual_friends_count": 3,
            "mutual_guilds": [{ "id": "1" }, { "id": "2" }],
        });
        let c = card("5", &profile, &Value::Null, &look());
        assert_eq!((c.name.as_str(), c.username.as_str(), c.bio.as_str(), c.tag.as_str()), ("Pat", "pat", "hello", "RUST"));
        assert_eq!(c.color, Some(255));
        assert_eq!(c.banner.as_deref(), Some("b1"));
        assert_eq!(c.badges, vec!["https://cdn.discordapp.com/badge-icons/abc123.png", "https://cdn.discordapp.com/badge-icons/def.png"]);
        assert_eq!((c.mutual_friends, c.mutual_servers), (Some(3), Some(2)));
    }

    #[test]
    fn without_a_profile_uses_what_we_know() {
        let c = card("7", &Value::Null, &json!({ "id": "7", "username": "sam" }), &look());
        assert_eq!((c.name.as_str(), c.avatar.as_str()), ("sam", "avatar-of-7"));
        assert!(c.badges.is_empty() && c.mutual_friends.is_none() && c.mutual_servers.is_none());
    }

    #[test]
    fn a_badge_says_what_discord_said_and_links_only_to_the_web() {
        let profile = json!({ "badges": [
            { "id": "premium", "icon": "abc", "description": "Подписчик", "link": "https://discord.com/nitro" },
            { "id": "odd", "icon": "def", "description": "Что-то", "link": "file:///c:/evil" },
        ] });
        let c = card("5", &profile, &Value::Null, &look());
        assert_eq!(c.badge_info[0], Badge { id: "premium".into(), text: "Подписчик".into(), link: "https://discord.com/nitro".into() });
        assert_eq!(c.badge_info[1].link, "");
    }

    #[test]
    fn growing_badges_know_how_far_they_are() {
        let (heading, tiers) = badge_tiers("premium_tenure_12_month_v2").unwrap();
        assert_eq!(heading, "Nitro");
        assert_eq!(tiers.len(), 9);
        assert_eq!(tiers.iter().filter(|t| t.reached).count(), 5, "up to Platinum");
        let (_, boost) = badge_tiers("guild_booster_lvl4").unwrap();
        assert_eq!(boost.iter().filter(|t| t.reached).count(), 4);
        assert_eq!(boost.len(), 9);
        assert!(badge_tiers("guild_booster_lvl12").is_none());
        assert!(badge_tiers("hypesquad_house_1").is_none());
        assert_eq!(badge_tiers("premium").unwrap().1.iter().filter(|t| t.reached).count(), 1);
    }

    #[test]
    fn linked_accounts_get_addresses_only_from_plain_names() {
        let profile = json!({ "connected_accounts": [
            { "type": "steam", "id": "7656", "name": "Gold Man" },
            { "type": "github", "id": "1", "name": "../evil" },
            { "type": "battlenet", "id": "2", "name": "gold#1" },
        ] });
        let list = accounts(&profile);
        assert_eq!(list[0], Account { kind: "Steam".into(), name: "Gold Man".into(), url: Some("https://steamcommunity.com/profiles/7656".into()) });
        assert_eq!(list[1].url, None);
        assert_eq!((list[2].kind.as_str(), list[2].url.clone()), ("Battle.net", None));
        assert!(accounts(&Value::Null).is_empty());
    }

    #[test]
    fn only_digits_make_a_mention_link() {
        assert_eq!(user_of_link("nimbus-user:123"), Some("123"));
        assert_eq!(user_of_link("nimbus-user:12a"), None);
        assert_eq!(user_of_link("nimbus-user:"), None);
        assert_eq!(user_of_link("https://example.com"), None);
    }

    #[test]
    fn saved_names_are_plain() {
        assert_eq!(file_name_of("https://cdn.discordapp.com/attachments/1/2/cat.PNG?ex=1"), "cat.PNG");
        assert_eq!(file_name_of("https://cdn.discordapp.com/a/..%2F..%2Fevil.exe"), "image.png");
        assert_eq!(file_name_of("https://x.com/"), "image.png");
    }
}
