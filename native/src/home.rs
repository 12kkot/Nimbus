//! The DM side of the client: the DM list (with presence dots, what people
//! play and group sizes), friends and friend requests, and message requests.
//! Plain functions over Discord's JSON, so they can be tested without a UI.

use std::collections::HashMap;

use serde_json::Value;

/// Someone's presence as the list shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presence {
    /// 1 online, 2 idle, 3 do not disturb, 4 offline (as `ChannelItem.status`).
    pub status: i32,
    /// What they're doing ("ROBLOX", "Слушает Spotify", a custom status), or empty.
    pub activity: String,
}

pub const OFFLINE: i32 = 4;

pub fn status_code(status: &str) -> i32 {
    match status {
        "online" => 1,
        "idle" => 2,
        "dnd" => 3,
        _ => OFFLINE, // offline, invisible
    }
}

pub fn status_text(status: i32) -> String {
    match status {
        1 => crate::i18n::t("В сети"),
        2 => crate::i18n::t("Не активен"),
        3 => crate::i18n::t("Не беспокоить"),
        _ => crate::i18n::t("Не в сети"),
    }
}

/// The line Discord shows under a name: a game first, then other activities,
/// then a custom status.
pub fn activity_text(activities: &Value) -> String {
    let list = activities.as_array().map(Vec::as_slice).unwrap_or_default();
    let name = |a: &Value| a["name"].as_str().unwrap_or("").to_string();
    let by_type = |t: u64| list.iter().find(|a| a["type"].as_u64() == Some(t));
    if let Some(a) = by_type(0) {
        return name(a);
    }
    if let Some(a) = by_type(1) {
        return crate::i18n::tf("Стримит {}", &[&a["details"].as_str().unwrap_or(&name(a))]);
    }
    if let Some(a) = by_type(2) {
        return crate::i18n::tf("Слушает {}", &[&name(a)]);
    }
    if let Some(a) = by_type(3) {
        return crate::i18n::tf("Смотрит {}", &[&name(a)]);
    }
    if let Some(a) = by_type(5) {
        return crate::i18n::tf("Соревнуется в {}", &[&name(a)]);
    }
    by_type(4).and_then(|a| a["state"].as_str()).unwrap_or("").to_string()
}

/// A presence entry: READY's `presences` ({user: {id}}), READY_SUPPLEMENTAL's
/// friends ({user_id}) and PRESENCE_UPDATE all fit.
pub fn presence_of(entry: &Value) -> Option<(String, Presence)> {
    let id = entry["user"]["id"].as_str().or(entry["user_id"].as_str())?;
    let presence = Presence { status: status_code(entry["status"].as_str().unwrap_or("offline")), activity: activity_text(&entry["activities"]) };
    Some((id.to_string(), presence))
}

/// The Spotify track someone is listening to: its id and when it started (ms since 1970); none when not listening.
pub fn spotify_of(entry: &Value) -> (String, Option<(String, i64)>) {
    let id = entry["user"]["id"].as_str().or(entry["user_id"].as_str()).unwrap_or("").to_string();
    let track = entry["activities"].as_array().into_iter().flatten().find(|a| a["type"].as_u64() == Some(2) && a["name"].as_str() == Some("Spotify")).and_then(|a| {
        let track = a["sync_id"].as_str().filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric()))?;
        Some((track.to_string(), a["timestamps"]["start"].as_i64().unwrap_or(0)))
    });
    (id, track)
}

/// The same for everyone READY and READY_SUPPLEMENTAL tell about.
pub fn spotify_in(payload: &Value) -> Vec<(String, Option<(String, i64)>)> {
    let lists = [&payload["presences"], &payload["merged_presences"]["friends"]];
    lists.into_iter().filter_map(Value::as_array).flatten().map(spotify_of).collect()
}

/// Everyone in a group, we too, as user objects. Discord gives the others as `recipients` or, in READY,
/// as `recipient_ids` with the users told apart.
pub fn group_people(channel: &Value, me: &Value, known: &HashMap<String, Value>) -> Vec<(String, Value)> {
    let listed = channel["recipients"].as_array().into_iter().flatten().filter_map(|u| Some((u["id"].as_str()?.to_string(), u.clone())));
    let by_id = channel["recipient_ids"].as_array().into_iter().flatten().filter_map(|id| {
        let id = id.as_str()?;
        Some((id.to_string(), known.get(id).cloned().unwrap_or_else(|| serde_json::json!({ "id": id }))))
    });
    let mut people: Vec<(String, Value)> = Vec::new();
    for (id, user) in me["id"].as_str().map(|id| (id.to_string(), me.clone())).into_iter().chain(listed).chain(by_id) {
        if !people.iter().any(|(seen, _)| *seen == id) {
            people.push((id, user));
        }
    }
    people
}

/// Someone joined or left a group: its lists as Discord would give them now.
pub fn change_recipient(channel: &mut Value, user: &Value, added: bool) {
    let Some(id) = user["id"].as_str() else { return };
    if let Some(list) = channel["recipients"].as_array_mut() {
        list.retain(|u| u["id"].as_str() != Some(id));
        if added {
            list.push(user.clone());
        }
    }
    if let Some(list) = channel["recipient_ids"].as_array_mut() {
        list.retain(|u| u.as_str() != Some(id));
        if added {
            list.push(Value::from(id));
        }
    }
}

/// Presences from READY and READY_SUPPLEMENTAL, whichever carries them.
pub fn presences_in(payload: &Value) -> Vec<(String, Presence)> {
    let lists = [&payload["presences"], &payload["merged_presences"]["friends"]];
    lists.into_iter().filter_map(Value::as_array).flatten().filter_map(presence_of).collect()
}

/// "1 участник", "3 участника", "5 участников".
pub fn members_label(n: usize) -> String {
    // English has one plural, Russian three; each form is its own entry in the table.
    match (n % 10, n % 100) {
        _ if crate::i18n::english() && n != 1 => crate::i18n::tf("{} участников", &[&n]),
        (1, x) if x != 11 => crate::i18n::tf("{} участник", &[&n]),
        (2..=4, x) if !(12..=14).contains(&x) => crate::i18n::tf("{} участника", &[&n]),
        _ => crate::i18n::tf("{} участников", &[&n]),
    }
}

/// Message requests and spam stay out of the DM list, on their own page.
pub fn is_request(channel: &Value) -> bool {
    channel["is_message_request"].as_bool() == Some(true) || channel["is_spam"].as_bool() == Some(true)
}

fn last_message(channel: &Value) -> u64 {
    // Snowflakes grow with time; a channel without messages sorts by its own id.
    channel["last_message_id"].as_str().or(channel["id"].as_str()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// One DM or group DM as the list shows it.
#[derive(Debug, PartialEq)]
pub struct DmRow {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub status: i32,
    pub avatar_url: Option<String>,
    /// The other person in a 1:1 DM.
    pub recipient: Option<String>,
}

/// How main.rs names users and finds their avatars.
pub struct Look<'a> {
    pub name: &'a dyn Fn(&Value) -> String,
    pub avatar: &'a dyn Fn(&Value) -> String,
    pub cdn: &'a str,
}

/// The DM list, newest conversation first, without message requests.
pub fn dm_rows(private: &[Value], presences: &HashMap<String, Presence>, me_name: &str, look: &Look) -> Vec<DmRow> {
    let mut channels: Vec<&Value> = private.iter().filter(|c| !is_request(c)).collect();
    channels.sort_by_key(|c| std::cmp::Reverse(last_message(c)));
    channels.into_iter().map(|c| dm_row(c, presences, me_name, look)).collect()
}

pub fn dm_row(c: &Value, presences: &HashMap<String, Presence>, me_name: &str, look: &Look) -> DmRow {
    let id = c["id"].as_str().unwrap_or("").to_string();
    let recipients = c["recipients"].as_array().map(Vec::as_slice).unwrap_or_default();
    let group = c["type"].as_u64() == Some(3);
    let name = match c["name"].as_str() {
        Some(n) if !n.trim().is_empty() => n.to_string(),
        _ if !recipients.is_empty() => recipients.iter().map(|u| (look.name)(u)).collect::<Vec<_>>().join(", "),
        _ => crate::i18n::tf("Группа {}", &[&me_name]),
    };
    if group {
        let avatar_url = match c["icon"].as_str() {
            Some(icon) => Some(format!("{}/channel-icons/{id}/{icon}.png?size=64", look.cdn)),
            None => recipients.first().map(|u| (look.avatar)(u)),
        };
        return DmRow { id, name, subtitle: members_label(recipients.len() + 1), status: 0, avatar_url, recipient: None };
    }
    let other = recipients.first();
    let recipient = other.and_then(|u| u["id"].as_str()).map(String::from);
    let presence = recipient.as_ref().and_then(|r| presences.get(r)).cloned().unwrap_or(Presence { status: OFFLINE, activity: String::new() });
    DmRow { id, name, subtitle: presence.activity, status: presence.status, avatar_url: other.map(|u| (look.avatar)(u)), recipient }
}

/// What a failed friend request means, in words (Discord answers with codes).
pub fn add_friend_error(error: &str) -> String {
    let code = error.split("\"code\":").nth(1).and_then(|s| s.trim().split(|c: char| !c.is_ascii_digit()).next()).unwrap_or("");
    match code {
        "80004" => crate::i18n::t("Такого пользователя нет. Проверь имя пользователя (не отображаемое имя)."),
        "80007" => crate::i18n::t("Вы уже друзья."),
        "80000" | "80001" => crate::i18n::t("Этот пользователь не принимает заявки в друзья."),
        "80003" => crate::i18n::t("Нельзя отправить заявку самому себе."),
        "40002" | "40003" => crate::i18n::t("Discord просит подтвердить аккаунт: сделай это в официальном приложении."),
        _ if error.contains("captcha") => crate::i18n::t("Discord просит капчу: отправь заявку из официального приложения."),
        _ => crate::i18n::tf("Не получилось: {}", &[&error]),
    }
}

/// Someone on the friends page (`kind` as `PersonItem.kind`: 0 friend, 1 incoming, 2 outgoing).
#[derive(Debug, PartialEq)]
pub struct PersonRow {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub status: i32,
    pub kind: i32,
    pub avatar_url: String,
}

/// Friends for a tab: 0 online, 1 all, 2 pending requests (incoming first).
pub fn friend_rows(relationships: &[Value], users: &HashMap<String, Value>, presences: &HashMap<String, Presence>, tab: i32, look: &Look) -> Vec<PersonRow> {
    let mut rows: Vec<PersonRow> = relationships
        .iter()
        .filter_map(|r| {
            let id = r["id"].as_str().or(r["user"]["id"].as_str())?.to_string();
            let kind = match r["type"].as_u64()? {
                1 => 0,
                3 => 1,
                4 => 2,
                _ => return None, // blocked and the like
            };
            let user = if r["user"].is_object() { r["user"].clone() } else { users.get(&id).cloned().unwrap_or_default() };
            let presence = presences.get(&id).cloned().unwrap_or(Presence { status: OFFLINE, activity: String::new() });
            let subtitle = if presence.activity.is_empty() { status_text(presence.status) } else { presence.activity.clone() };
            let name = match r["nickname"].as_str() {
                Some(nick) if !nick.is_empty() => nick.to_string(),
                _ if user.is_object() => (look.name)(&user),
                _ => id.clone(),
            };
            Some(PersonRow { avatar_url: (look.avatar)(&user), id, name, subtitle, status: presence.status, kind })
        })
        .filter(|p| match tab {
            0 => p.kind == 0 && p.status != OFFLINE,
            1 => p.kind == 0,
            2 => p.kind != 0,
            _ => false, // the "add friend" tab is a form, not a list
        })
        .collect();
    rows.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_counts_everyone_once_and_us_too() {
        let me = json!({ "id": "1", "username": "я" });
        let mut known = HashMap::new();
        known.insert("3".to_string(), json!({ "id": "3", "username": "из READY" }));
        let group = json!({ "recipients": [{ "id": "2", "username": "два" }, { "id": "1" }], "recipient_ids": ["3", "2", "4"] });
        let people = group_people(&group, &me, &known);
        let ids: Vec<&str> = people.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["1", "2", "3", "4"]);
        assert_eq!(people[0].1["username"], "я");
        assert_eq!(people[2].1["username"], "из READY");
        assert_eq!(people[3].1["id"], "4", "someone unknown still has a row");
    }

    #[test]
    fn people_come_and_go_from_a_group() {
        let mut group = json!({ "recipients": [{ "id": "2" }], "recipient_ids": ["2"] });
        change_recipient(&mut group, &json!({ "id": "3" }), true);
        assert_eq!(group, json!({ "recipients": [{ "id": "2" }, { "id": "3" }], "recipient_ids": ["2", "3"] }));
        change_recipient(&mut group, &json!({ "id": "2" }), false);
        assert_eq!(group, json!({ "recipients": [{ "id": "3" }], "recipient_ids": ["3"] }));
    }

    #[test]
    fn only_a_plain_spotify_track_counts() {
        let listening = json!({ "user_id": "1", "activities": [{ "type": 2, "name": "Spotify", "sync_id": "abc123", "timestamps": { "start": 5 } }] });
        assert_eq!(spotify_of(&listening), ("1".to_string(), Some(("abc123".to_string(), 5))));
        let other = json!({ "user_id": "1", "activities": [{ "type": 2, "name": "Яндекс", "sync_id": "abc" }] });
        assert_eq!(spotify_of(&other).1, None);
        let odd = json!({ "user_id": "1", "activities": [{ "type": 2, "name": "Spotify", "sync_id": "../x" }] });
        assert_eq!(spotify_of(&odd).1, None);
    }
    use serde_json::json;

    fn look() -> (impl Fn(&Value) -> String, impl Fn(&Value) -> String) {
        (|u: &Value| u["global_name"].as_str().or(u["username"].as_str()).unwrap_or("?").to_string(), |u: &Value| format!("avatar:{}", u["id"].as_str().unwrap_or("")))
    }

    #[test]
    fn activities_read_like_discord() {
        assert_eq!(activity_text(&json!([{ "type": 4, "state": "сплю" }, { "type": 0, "name": "ROBLOX" }])), "ROBLOX");
        assert_eq!(activity_text(&json!([{ "type": 2, "name": "Spotify" }])), "Слушает Spotify");
        assert_eq!(activity_text(&json!([{ "type": 4, "state": "сплю" }])), "сплю");
        assert_eq!(activity_text(&json!(null)), "");
    }

    #[test]
    fn friend_request_errors_in_words() {
        let err = r#"Discord API 400 Bad Request: {"message": "Unknown User", "code": 80004}"#;
        assert!(add_friend_error(err).starts_with("Такого пользователя нет"));
        assert_eq!(add_friend_error(r#"x {"code":80007}"#), "Вы уже друзья.");
        assert!(add_friend_error("Discord API 400: captcha_key required").contains("капчу"));
        assert!(add_friend_error("timeout").starts_with("Не получилось"));
    }

    #[test]
    fn members_label_declines() {
        assert_eq!(members_label(1), "1 участник");
        assert_eq!(members_label(2), "2 участника");
        assert_eq!(members_label(5), "5 участников");
        assert_eq!(members_label(11), "11 участников");
        assert_eq!(members_label(22), "22 участника");
    }

    #[test]
    fn dm_list_sorts_names_groups_and_hides_requests() {
        let (name, avatar) = look();
        let look = Look { name: &name, avatar: &avatar, cdn: "cdn" };
        let private = vec![
            json!({ "id": "10", "type": 1, "last_message_id": "100", "recipients": [{ "id": "u1", "username": "demo_carol" }] }),
            json!({ "id": "11", "type": 1, "last_message_id": "300", "recipients": [{ "id": "u2", "username": "new" }] }),
            json!({ "id": "12", "type": 3, "last_message_id": "200", "recipients": [] }),
            json!({ "id": "13", "type": 3, "last_message_id": "50", "name": "demo_frank", "recipients": [{ "id": "u3", "username": "x" }] }),
            json!({ "id": "14", "type": 1, "last_message_id": "999", "is_message_request": true, "recipients": [{ "id": "u4", "username": "stranger" }] }),
        ];
        let mut presences = HashMap::new();
        presences.insert("u2".to_string(), Presence { status: 1, activity: "ROBLOX".into() });
        let rows = dm_rows(&private, &presences, "demo_user", &look);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["new", "Группа demo_user", "demo_carol", "demo_frank"]);
        assert_eq!((rows[0].status, rows[0].subtitle.as_str()), (1, "ROBLOX"));
        assert_eq!(rows[1].subtitle, "1 участник");
        assert_eq!(rows[3].subtitle, "2 участника");
        assert_eq!((rows[2].status, rows[2].recipient.as_deref()), (OFFLINE, Some("u1")));
    }

    #[test]
    fn presences_from_ready_supplemental_and_updates() {
        let ready = json!({ "presences": [{ "user": { "id": "a" }, "status": "dnd", "activities": [] }] });
        let sup = json!({ "merged_presences": { "friends": [{ "user_id": "b", "status": "idle", "activities": [{ "type": 0, "name": "Rust" }] }] } });
        assert_eq!(presences_in(&ready), vec![("a".into(), Presence { status: 3, activity: String::new() })]);
        assert_eq!(presences_in(&sup), vec![("b".into(), Presence { status: 2, activity: "Rust".into() })]);
    }

    #[test]
    fn friend_tabs() {
        let (name, avatar) = look();
        let look = Look { name: &name, avatar: &avatar, cdn: "cdn" };
        let relationships = vec![
            json!({ "id": "a", "type": 1, "user": { "id": "a", "username": "zed" } }),
            json!({ "id": "b", "type": 1, "user": { "id": "b", "username": "amy" } }),
            json!({ "id": "c", "type": 3, "user": { "id": "c", "username": "in" } }),
            json!({ "id": "d", "type": 4, "user": { "id": "d", "username": "out" } }),
            json!({ "id": "e", "type": 2, "user": { "id": "e", "username": "blocked" } }),
        ];
        let mut presences = HashMap::new();
        presences.insert("a".to_string(), Presence { status: 1, activity: String::new() });
        let users = HashMap::new();
        let online: Vec<String> = friend_rows(&relationships, &users, &presences, 0, &look).into_iter().map(|p| p.name).collect();
        let all: Vec<String> = friend_rows(&relationships, &users, &presences, 1, &look).into_iter().map(|p| p.name).collect();
        let pending: Vec<(String, i32)> = friend_rows(&relationships, &users, &presences, 2, &look).into_iter().map(|p| (p.name, p.kind)).collect();
        assert_eq!(online, ["zed"]);
        assert_eq!(all, ["amy", "zed"]);
        assert_eq!(pending, [("in".to_string(), 1), ("out".to_string(), 2)]);
        assert_eq!(friend_rows(&relationships, &users, &presences, 1, &look)[0].subtitle, "Не в сети");
    }
}
