//! Server scheduled events: the list Discord keeps for a server, how its row and cards word them, our
//! "interested" mark, and the body of a new event.

use std::collections::HashSet;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat, TimeZone, Timelike, Utc};
use serde_json::{json, Value};

use crate::i18n;

/// Where an event happens: a stage channel, a voice channel, or somewhere else (a text).
pub const STAGE: u64 = 1;
pub const VOICE: u64 = 2;
pub const EXTERNAL: u64 = 3;

/// Event statuses.
const ACTIVE: u64 = 2;
const COMPLETED: u64 = 3;
const CANCELED: u64 = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledEvent {
    pub id: String,
    pub guild: String,
    pub name: String,
    pub description: String,
    /// When it starts and (if known) ends, in ms since 1970.
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    /// 1 scheduled, 2 going on now, 3 over, 4 cancelled.
    pub status: u64,
    pub entity: u64,
    pub channel: Option<String>,
    /// Where an event elsewhere happens, as its creator wrote it.
    pub location: String,
    pub creator_id: String,
    /// The creator's avatar on Discord's CDN, if they have one.
    pub creator_avatar: Option<String>,
    pub creator_name: String,
    /// How many said they're interested.
    pub users: u64,
}

impl ScheduledEvent {
    pub fn active(&self) -> bool {
        self.status == ACTIVE
    }
}

fn time_ms(v: &Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|t| t.timestamp_millis())
}

fn avatar_url(user: &Value) -> Option<String> {
    let id = user["id"].as_str().filter(|id| crate::snowflake(id).is_ok())?;
    let hash = user["avatar"].as_str().filter(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))?;
    Some(format!("https://cdn.discordapp.com/avatars/{id}/{hash}.png?size=64"))
}

/// One event as Discord sends it (the list, or the gateway).
pub fn parse(v: &Value) -> Option<ScheduledEvent> {
    let creator = &v["creator"];
    Some(ScheduledEvent {
        id: v["id"].as_str()?.to_string(),
        guild: v["guild_id"].as_str().unwrap_or("").to_string(),
        name: v["name"].as_str().unwrap_or("").to_string(),
        description: v["description"].as_str().unwrap_or("").to_string(),
        start_ms: time_ms(&v["scheduled_start_time"])?,
        end_ms: time_ms(&v["scheduled_end_time"]),
        status: v["status"].as_u64().unwrap_or(1),
        entity: v["entity_type"].as_u64().unwrap_or(EXTERNAL),
        channel: v["channel_id"].as_str().map(String::from),
        location: v["entity_metadata"]["location"].as_str().unwrap_or("").to_string(),
        creator_id: v["creator_id"].as_str().or(creator["id"].as_str()).unwrap_or("").to_string(),
        creator_avatar: avatar_url(creator),
        creator_name: creator["global_name"].as_str().or(creator["username"].as_str()).unwrap_or("").to_string(),
        users: v["user_count"].as_u64().unwrap_or(0),
    })
}

/// The events still to come or going on, those going on first, then by start.
fn tidy(list: &mut Vec<ScheduledEvent>) {
    list.retain(|e| !matches!(e.status, COMPLETED | CANCELED));
    list.sort_by_key(|e| (!e.active(), e.start_ms));
}

pub fn parse_list(v: &Value) -> Vec<ScheduledEvent> {
    let mut list: Vec<ScheduledEvent> = v.as_array().into_iter().flatten().filter_map(parse).collect();
    tidy(&mut list);
    list
}

/// Which Russian form a count takes: 1 событие, 2 события, 5 событий.
fn ru_form(n: usize) -> usize {
    match (n % 10, n % 100) {
        (1, r) if r != 11 => 0,
        (2..=4, r) if !(12..=14).contains(&r) => 1,
        _ => 2,
    }
}

/// The row above a server's channels: "Мероприятия" with none, else how many there are.
pub fn count_title(n: usize) -> String {
    if n == 0 {
        return i18n::t("Мероприятия");
    }
    let form = if i18n::english() { if n == 1 { 0 } else { 2 } } else { ru_form(n) };
    match form {
        0 => i18n::tf("{} событие", &[&n]),
        1 => i18n::tf("{} события", &[&n]),
        _ => i18n::tf("{} событий", &[&n]),
    }
}

/// When an event starts, as its card says: "Сегодня, в 18:00", "Завтра, в 18:00", or the date.
pub fn when(start: NaiveDateTime, today: NaiveDate) -> String {
    let hm = crate::clock::hm(&start);
    if start.date() == today {
        i18n::tf("Сегодня, в {}", &[&hm])
    } else if Some(start.date()) == today.succ_opt() {
        i18n::tf("Завтра, в {}", &[&hm])
    } else {
        i18n::tf("{}, в {}", &[&crate::clock::date(start.date()), &hm])
    }
}

/// The link "Поделиться" copies.
pub fn link(guild: &str, event: &str) -> String {
    format!("https://discord.com/events/{guild}/{event}")
}

pub fn list_path(guild: &str) -> Result<String, String> {
    Ok(format!("/guilds/{}/scheduled-events?with_user_count=true", crate::snowflake(guild)?))
}

pub fn event_path(guild: &str, event: &str) -> Result<String, String> {
    Ok(format!("/guilds/{}/scheduled-events/{}", crate::snowflake(guild)?, crate::snowflake(event)?))
}

/// PUT to say we're interested, DELETE to take it back.
pub fn interest_path(guild: &str, event: &str) -> Result<String, String> {
    Ok(format!("{}/users/@me", event_path(guild, event)?))
}

/// Our own "interested" click, counted at once (the gateway's word on it later is not counted again).
pub fn set_interested(list: &mut [ScheduledEvent], interested: &mut HashSet<String>, event: &str, on: bool) {
    let Some(e) = list.iter_mut().find(|e| e.id == event) else { return };
    match (interested.contains(event), on) {
        (false, true) => e.users += 1,
        (true, false) => e.users = e.users.saturating_sub(1),
        _ => {}
    }
    if on {
        interested.insert(event.to_string());
    } else {
        interested.remove(event);
    }
}

/// GUILD_SCHEDULED_EVENT_CREATE / _UPDATE / _DELETE / _USER_ADD / _USER_REMOVE on a server's kept list.
/// Returns whether anything changed.
pub fn apply(list: &mut Vec<ScheduledEvent>, kind: &str, data: &Value, me: &str, interested: &mut HashSet<String>) -> bool {
    match kind {
        "GUILD_SCHEDULED_EVENT_CREATE" | "GUILD_SCHEDULED_EVENT_UPDATE" => {
            let Some(mut e) = parse(data) else { return false };
            if let Some(old) = list.iter().position(|x| x.id == e.id) {
                if data["user_count"].is_null() {
                    e.users = list[old].users;
                }
                list[old] = e;
            } else {
                list.push(e);
            }
            tidy(list);
            true
        }
        "GUILD_SCHEDULED_EVENT_DELETE" => {
            let before = list.len();
            list.retain(|e| Some(e.id.as_str()) != data["id"].as_str());
            list.len() != before
        }
        "GUILD_SCHEDULED_EVENT_USER_ADD" | "GUILD_SCHEDULED_EVENT_USER_REMOVE" => {
            let add = kind.ends_with("ADD");
            let Some(event) = data["guild_scheduled_event_id"].as_str() else { return false };
            if data["user_id"].as_str() == Some(me) {
                // Ours: already counted when we clicked (or clicked elsewhere: count it now).
                if interested.contains(event) == add {
                    return false;
                }
                set_interested(list, interested, event, add);
                return true;
            }
            let Some(e) = list.iter_mut().find(|e| e.id == event) else { return false };
            e.users = if add { e.users + 1 } else { e.users.saturating_sub(1) };
            true
        }
        _ => false,
    }
}

/// What the "create event" window gathered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub entity: u64,
    pub channel: String,
    pub location: String,
    pub name: String,
    /// As typed: dd.mm.yyyy (or yyyy-mm-dd) and hh:mm.
    pub date: String,
    pub time: String,
    pub end_date: String,
    pub end_time: String,
    pub description: String,
}

/// When a new event starts unless changed: an hour from now, rounded up to the next half hour.
pub fn default_start(now: NaiveDateTime) -> NaiveDateTime {
    let t = now + Duration::hours(1);
    let base = t.date().and_hms_opt(t.hour(), 0, 0).unwrap_or(t);
    let minutes = if t.minute() == 0 && t.second() == 0 { 0 } else if t.minute() < 30 || (t.minute() == 30 && t.second() == 0) { 30 } else { 60 };
    base + Duration::minutes(minutes)
}

/// The date and the time as the fields show them.
pub fn date_field(t: NaiveDateTime) -> (String, String) {
    (t.format("%d.%m.%Y").to_string(), t.format("%H:%M").to_string())
}

/// When the draft starts, if its fields can be read.
pub fn start_of(d: &Draft) -> Option<NaiveDateTime> {
    parse_local(&d.date, &d.time)
}

fn parse_local(date: &str, time: &str) -> Option<NaiveDateTime> {
    let date = date.trim();
    let d = NaiveDate::parse_from_str(date, "%d.%m.%Y").or_else(|_| NaiveDate::parse_from_str(date, "%Y-%m-%d")).ok()?;
    let t = NaiveTime::parse_from_str(time.trim(), "%H:%M").ok()?;
    Some(d.and_time(t))
}

fn utc_text<Tz: TimeZone>(t: NaiveDateTime, tz: &Tz) -> Option<String> {
    Some(tz.from_local_datetime(&t).earliest()?.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// The body of `POST /guilds/{id}/scheduled-events`; the times are read in `now`'s time zone and must be ahead.
pub fn create_body<Tz: TimeZone>(d: &Draft, now: &DateTime<Tz>) -> Result<Value, String> {
    let name = d.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(i18n::t("Укажите тему мероприятия (до 100 символов)"));
    }
    if d.description.chars().count() > 1000 {
        return Err(i18n::t("Описание не длиннее 1000 символов"));
    }
    let tz = now.timezone();
    let start = parse_local(&d.date, &d.time).ok_or_else(|| i18n::t("Дата и время начала: дд.мм.гггг и чч:мм"))?;
    if start <= now.naive_local() {
        return Err(i18n::t("Время начала уже прошло"));
    }
    let start_text = utc_text(start, &tz).ok_or_else(|| i18n::t("Дата и время начала: дд.мм.гггг и чч:мм"))?;
    let mut body = json!({
        "name": name,
        "privacy_level": 2,
        "scheduled_start_time": start_text,
        "entity_type": d.entity,
    });
    if !d.description.trim().is_empty() {
        body["description"] = json!(d.description.trim());
    }
    match d.entity {
        STAGE | VOICE => {
            let channel = crate::snowflake(&d.channel).map_err(|_| i18n::t("Выберите канал"))?;
            body["channel_id"] = json!(channel);
        }
        EXTERNAL => {
            let place = d.location.trim();
            if place.is_empty() || place.chars().count() > 100 {
                return Err(i18n::t("Укажите место (до 100 символов)"));
            }
            let end = parse_local(&d.end_date, &d.end_time).ok_or_else(|| i18n::t("Дата и время окончания: дд.мм.гггг и чч:мм"))?;
            if end <= start {
                return Err(i18n::t("Окончание должно быть позже начала"));
            }
            body["entity_metadata"] = json!({ "location": place });
            body["scheduled_end_time"] = json!(utc_text(end, &tz).ok_or_else(|| i18n::t("Дата и время окончания: дд.мм.гггг и чч:мм"))?);
        }
        _ => return Err(i18n::t("Выберите, где пройдёт мероприятие")),
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn sample() -> Value {
        json!([
            { "id": "11", "guild_id": "1", "name": "Завтрашний", "scheduled_start_time": "2026-11-02T15:00:00+00:00", "status": 1,
              "entity_type": 3, "entity_metadata": { "location": "Парк Горького" }, "scheduled_end_time": "2026-11-02T17:00:00+00:00",
              "creator_id": "5", "creator": { "id": "5", "username": "masha", "avatar": "abc" }, "user_count": 4 },
            { "id": "12", "guild_id": "1", "name": "Идёт", "description": "Сцена", "scheduled_start_time": "2026-11-01T18:00:00+00:00",
              "status": 2, "entity_type": 1, "channel_id": "77", "user_count": 9 },
            { "id": "13", "guild_id": "1", "name": "Отменён", "scheduled_start_time": "2026-11-01T10:00:00+00:00", "status": 4, "entity_type": 2 },
            { "id": "14", "guild_id": "1", "name": "Без времени" },
        ])
    }

    #[test]
    fn the_list_keeps_coming_events_active_first() {
        let list = parse_list(&sample());
        assert_eq!(list.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["12", "11"]);
        assert!(list[0].active());
        assert_eq!((list[0].entity, list[0].channel.as_deref(), list[0].users), (STAGE, Some("77"), 9));
        let e = &list[1];
        assert_eq!((e.location.as_str(), e.creator_id.as_str(), e.creator_name.as_str()), ("Парк Горького", "5", "masha"));
        assert_eq!(e.creator_avatar.as_deref(), Some("https://cdn.discordapp.com/avatars/5/abc.png?size=64"));
        assert!(e.end_ms.is_some());
    }

    #[test]
    fn counts_take_the_russian_plural() {
        let forms: Vec<String> = [0, 1, 2, 4, 5, 11, 12, 21, 22, 25, 111].iter().map(|&n| count_title(n)).collect();
        assert_eq!(forms, vec!["Мероприятия", "1 событие", "2 события", "4 события", "5 событий", "11 событий", "12 событий", "21 событие", "22 события", "25 событий", "111 событий"]);
    }

    #[test]
    fn start_times_are_worded() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let at = |d: u32, h: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap().and_hms_opt(h, 0, 0).unwrap();
        assert_eq!(when(at(1, 18), today), "Сегодня, в 18:00");
        assert_eq!(when(at(2, 9), today), "Завтра, в 09:00");
        assert_eq!(when(at(7, 20), today), "07.11.2026, в 20:00");
        assert_eq!(link("1", "2"), "https://discord.com/events/1/2");
        assert_eq!(interest_path("1", "2").unwrap(), "/guilds/1/scheduled-events/2/users/@me");
        assert!(list_path("x").is_err());
    }

    #[test]
    fn interest_is_counted_once() {
        let mut list = parse_list(&sample());
        let mut mine = HashSet::new();
        set_interested(&mut list, &mut mine, "11", true);
        assert_eq!(list[1].users, 5);
        let ours = json!({ "guild_scheduled_event_id": "11", "user_id": "me" });
        assert!(!apply(&mut list, "GUILD_SCHEDULED_EVENT_USER_ADD", &ours, "me", &mut mine), "our click is counted already");
        assert!(apply(&mut list, "GUILD_SCHEDULED_EVENT_USER_ADD", &json!({ "guild_scheduled_event_id": "11", "user_id": "9" }), "me", &mut mine));
        assert_eq!(list[1].users, 6);
        assert!(apply(&mut list, "GUILD_SCHEDULED_EVENT_USER_REMOVE", &ours, "me", &mut mine));
        assert_eq!(list[1].users, 5);
        assert!(!mine.contains("11"));
        // An update without a count keeps ours; a cancelled event goes; a deleted one too.
        let renamed = json!({ "id": "11", "guild_id": "1", "name": "Перенесли", "scheduled_start_time": "2026-11-03T15:00:00+00:00", "status": 1, "entity_type": 3 });
        assert!(apply(&mut list, "GUILD_SCHEDULED_EVENT_UPDATE", &renamed, "me", &mut mine));
        assert_eq!((list[1].name.as_str(), list[1].users), ("Перенесли", 5));
        let cancelled = json!({ "id": "12", "guild_id": "1", "scheduled_start_time": "2026-11-01T18:00:00+00:00", "status": 4 });
        assert!(apply(&mut list, "GUILD_SCHEDULED_EVENT_UPDATE", &cancelled, "me", &mut mine));
        assert_eq!(list.len(), 1);
        assert!(apply(&mut list, "GUILD_SCHEDULED_EVENT_DELETE", &json!({ "id": "11" }), "me", &mut mine));
        assert!(list.is_empty());
    }

    #[test]
    fn a_new_event_starts_on_the_next_half_hour() {
        let t = |h: u32, m: u32| NaiveDate::from_ymd_opt(2026, 11, 1).unwrap().and_hms_opt(h, m, 0).unwrap();
        assert_eq!(default_start(t(14, 10)), t(15, 30));
        assert_eq!(default_start(t(14, 40)), t(16, 0));
        assert_eq!(default_start(t(14, 0)), t(15, 0));
        assert_eq!(date_field(t(9, 5)), ("01.11.2026".to_string(), "09:05".to_string()));
    }

    #[test]
    fn create_bodies_by_place() {
        let msk = FixedOffset::east_opt(3 * 3600).unwrap();
        let now = msk.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap();
        let mut d = Draft { entity: VOICE, channel: "77".into(), name: " Созвон ".into(), date: "01.11.2026".into(), time: "18:30".into(), ..Default::default() };
        assert_eq!(create_body(&d, &now).unwrap(), json!({
            "name": "Созвон", "privacy_level": 2, "scheduled_start_time": "2026-11-01T15:30:00.000Z", "entity_type": 2, "channel_id": "77",
        }));
        d.channel = String::new();
        assert!(create_body(&d, &now).is_err(), "a voice event needs its channel");
        let mut out = Draft { entity: EXTERNAL, location: "Парк".into(), name: "Пикник".into(), date: "2026-11-02".into(), time: "10:00".into(), end_date: "02.11.2026".into(), end_time: "13:00".into(), description: "Берите пледы".into(), ..Default::default() };
        let body = create_body(&out, &now).unwrap();
        assert_eq!(body["entity_metadata"], json!({ "location": "Парк" }));
        assert_eq!(body["scheduled_end_time"], "2026-11-02T10:00:00.000Z");
        assert_eq!(body["description"], "Берите пледы");
        assert!(body.get("channel_id").is_none());
        out.end_time = "09:00".into();
        assert!(create_body(&out, &now).is_err(), "the end must come after the start");
        out.end_time = "13:00".into();
        out.time = "25:00".into();
        assert!(create_body(&out, &now).is_err());
        out.time = "10:00".into();
        out.date = "01.11.2026".into();
        out.end_date = "01.11.2026".into();
        assert!(create_body(&out, &now).is_err(), "10:00 today has passed");
        out.name = "   ".into();
        assert!(create_body(&out, &now).is_err());
    }
}
