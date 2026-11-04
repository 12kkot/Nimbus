//! Notifications about new messages: when to show one, what it says, and its own sound.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// What the person chose in Settings → Уведомления.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prefs {
    pub enabled: bool,
    pub sound: bool,
    /// The text of the message is shown too (otherwise only who wrote).
    pub show_text: bool,
    /// @everyone and @here count as a mention.
    pub everyone: bool,
}

impl Prefs {
    pub const DEFAULT: Prefs = Prefs {
        enabled: true,
        sound: true,
        show_text: true,
        everyone: true,
    };

    pub fn from_json(v: &Value) -> Self {
        let d = Self::DEFAULT;
        let flag = |k: &str, def: bool| v[k].as_bool().unwrap_or(def);
        Self {
            enabled: flag("enabled", d.enabled),
            sound: flag("sound", d.sound),
            show_text: flag("show_text", d.show_text),
            everyone: flag("everyone", d.everyone),
        }
    }

    pub fn to_json(self) -> Value {
        json!({ "enabled": self.enabled, "sound": self.sound, "show_text": self.show_text, "everyone": self.everyone })
    }
}

/// The current choices, read by the network side when a message arrives.
static CURRENT: Mutex<Prefs> = Mutex::new(Prefs::DEFAULT);
/// Streamer mode: nothing pops up and nothing sounds.
static STREAMER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_streamer(on: bool) {
    STREAMER.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn current() -> Prefs {
    *CURRENT.lock().unwrap()
}

pub fn set_current(p: Prefs) {
    *CURRENT.lock().unwrap() = p;
}

/// What is known about a message that just arrived.
#[derive(Debug, Clone, Copy, Default)]
pub struct Arrival {
    pub mine: bool,
    pub dm: bool,
    /// It names us.
    pub mentions_me: bool,
    /// @everyone or @here, where the server doesn't have them silenced for us.
    pub mentions_everyone: bool,
    /// The channel or its server is muted.
    pub muted: bool,
    /// What the server or channel is set to tell about.
    pub level: Level,
    /// The Nimbus window is in front.
    pub focused: bool,
}

/// Only what's meant for us (or everything, where asked), only when we aren't looking, never what's muted.
pub fn should_notify(p: &Prefs, a: &Arrival) -> bool {
    if !p.enabled || a.mine || a.focused || a.muted || STREAMER.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    match a.level {
        Level::Nothing => false,
        Level::All => true,
        Level::Mentions => a.dm || a.mentions_me || (p.everyone && a.mentions_everyone),
    }
}

/// What a server or channel tells about, as Discord numbers it (0, 1, 2).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Level {
    All,
    /// Also what a server or channel with no choice of its own gets.
    #[default]
    Mentions,
    Nothing,
}

impl Level {
    /// None for 3 (or anything else): "as the server is".
    pub fn from_code(code: u64) -> Option<Level> {
        match code {
            0 => Some(Level::All),
            1 => Some(Level::Mentions),
            2 => Some(Level::Nothing),
            _ => None,
        }
    }

    pub fn code(self) -> u64 {
        match self {
            Level::All => 0,
            Level::Mentions => 1,
            Level::Nothing => 2,
        }
    }
}

/// A server's or a channel's own choice.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Place {
    pub level: Option<Level>,
    pub muted: bool,
}

/// Muted, unless the mute had an end and it has passed.
fn mute_on(entry: &Value, now: DateTime<Utc>) -> bool {
    if entry["muted"].as_bool() != Some(true) {
        return false;
    }
    let end = entry["mute_config"]["end_time"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok());
    end.is_none_or(|end| end.with_timezone(&Utc) > now)
}

fn place_of(entry: &Value, now: DateTime<Utc>) -> Place {
    Place { level: entry["message_notifications"].as_u64().and_then(Level::from_code), muted: mute_on(entry, now) }
}

/// The notification choices of every server and channel (READY's user_guild_settings, then its updates).
/// DMs sit under the entry without a server, by channel.
#[derive(Debug, Default)]
pub struct Settings {
    /// By server id ("" for the DM side): its choice, and whether @everyone is silenced there.
    guilds: HashMap<String, (Place, bool)>,
    channels: HashMap<String, Place>,
    /// The channels each server has choices for, so an update can drop the ones it no longer lists.
    channels_of: HashMap<String, Vec<String>>,
}

impl Settings {
    pub fn from_ready(ready: &Value, now: DateTime<Utc>) -> Self {
        let mut s = Settings::default();
        let list = &ready["user_guild_settings"];
        for entry in list.as_array().or_else(|| list["entries"].as_array()).into_iter().flatten() {
            s.apply(entry, now);
        }
        s
    }

    /// One server's entry (from READY, USER_GUILD_SETTINGS_UPDATE, or our own change's answer).
    pub fn apply(&mut self, entry: &Value, now: DateTime<Utc>) {
        let guild = entry["guild_id"].as_str().unwrap_or("").to_string();
        for old in self.channels_of.remove(&guild).unwrap_or_default() {
            self.channels.remove(&old);
        }
        let mut ids = Vec::new();
        for o in entry["channel_overrides"].as_array().into_iter().flatten() {
            if let Some(cid) = o["channel_id"].as_str() {
                self.channels.insert(cid.to_string(), place_of(o, now));
                ids.push(cid.to_string());
            }
        }
        self.channels_of.insert(guild.clone(), ids);
        self.guilds.insert(guild, (place_of(entry, now), entry["suppress_everyone"].as_bool() == Some(true)));
    }

    pub fn guild(&self, guild: &str) -> Place {
        self.guilds.get(guild).map(|g| g.0).unwrap_or_default()
    }

    pub fn channel(&self, channel: &str) -> Place {
        self.channels.get(channel).copied().unwrap_or_default()
    }

    pub fn muted(&self, channel: &str, guild: Option<&str>) -> bool {
        self.channel(channel).muted || guild.is_some_and(|g| self.guild(g).muted)
    }

    /// The channel's own choice, else its server's, else only mentions.
    pub fn level(&self, channel: &str, guild: Option<&str>) -> Level {
        self.channel(channel).level.or_else(|| guild.and_then(|g| self.guild(g).level)).unwrap_or_default()
    }

    pub fn everyone_silenced(&self, guild: Option<&str>) -> bool {
        guild.is_some_and(|g| self.guilds.get(g).is_some_and(|s| s.1))
    }
}

/// A change picked in a server's or channel's menu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    /// None: the channel goes back to what its server has.
    Level(Option<Level>),
    Mute(bool),
    /// Muted for this many minutes.
    MuteFor(u64),
}

impl Change {
    /// "level:0".."level:3", "mute", "unmute", as the menu names them.
    pub fn parse(what: &str) -> Option<Change> {
        match what {
            "mute" => Some(Change::Mute(true)),
            "unmute" => Some(Change::Mute(false)),
            _ if what.starts_with("mute:") => what.strip_prefix("mute:")?.parse::<u64>().ok().filter(|m| *m > 0).map(Change::MuteFor),
            _ => what.strip_prefix("level:")?.parse::<u64>().ok().filter(|c| *c <= 3).map(|c| Change::Level(Level::from_code(c))),
        }
    }
}

/// The path and body that make the change, for the server or (with `channel`) one of its channels.
pub fn settings_request(guild: &str, channel: Option<&str>, change: Change) -> Result<(String, Value), String> {
    // The DM side has settings for its chats only; as a whole it is not a server.
    let place = if guild == "@me" && channel.is_some() { guild.to_string() } else { crate::discord::snowflake(guild)?.to_string() };
    let path = format!("/users/@me/guilds/{place}/settings");
    let fields = match change {
        Change::Level(level) => json!({ "message_notifications": level.map_or(3, Level::code) }),
        // Muted until turned off again.
        Change::Mute(true) => json!({ "muted": true, "mute_config": { "selected_time_window": -1, "end_time": null } }),
        Change::Mute(false) => json!({ "muted": false }),
        Change::MuteFor(minutes) => {
            let end = Utc::now() + chrono::Duration::minutes(minutes as i64);
            json!({ "muted": true, "mute_config": { "selected_time_window": minutes, "end_time": end.to_rfc3339() } })
        }
    };
    let body = match channel {
        Some(channel) => {
            let mut overrides = serde_json::Map::new();
            overrides.insert(crate::discord::snowflake(channel)?.to_string(), fields);
            json!({ "channel_overrides": overrides })
        }
        None if change == Change::Level(None) => return Err("a server has no level to go back to".into()),
        None => fields,
    };
    Ok((path, body))
}

/// What a notification says. `anonymous`: the settings hide the message, so also who wrote it and where.
#[derive(Debug, Clone, PartialEq)]
pub struct ToastText {
    pub heading: String,
    pub place: String,
    pub body: String,
    pub anonymous: bool,
}

/// The words of a notification. `place` is the server and channel, or "Direct message".
pub fn toast_text(p: &Prefs, author: &str, place: &str, text: &str) -> ToastText {
    if !p.show_text {
        return ToastText { heading: "Nimbus".into(), place: String::new(), body: crate::i18n::t("Новое сообщение"), anonymous: true };
    }
    let body = if text.trim().is_empty() {
        crate::i18n::t("Вложение")
    } else {
        let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let short: String = flat.chars().take(140).collect();
        if flat.chars().count() > 140 {
            format!("{short}…")
        } else {
            short
        }
    };
    ToastText { heading: author.to_string(), place: place.to_string(), body, anonymous: false }
}

/// The first letter of a name, for when there is no picture yet.
pub fn initial_of(name: &str) -> String {
    name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

/// Nimbus's own notification sound (two soft notes), as the bytes of a WAV file.
pub fn chime_wav() -> Vec<u8> {
    const RATE: u32 = 44_100;
    let mut samples: Vec<i16> = Vec::new();
    for (hz, ms) in [(523.25f32, 150u32), (659.25, 240)] {
        let n = (RATE * ms / 1000) as usize;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            // Rises and dies away, so it neither clicks nor lingers.
            let envelope = (i as f32 / 900.0).min(1.0) * (1.0 - i as f32 / n as f32).powf(1.8);
            samples.push(((t * hz * std::f32::consts::TAU).sin() * envelope * 3600.0) as i16);
        }
    }
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    fn away() -> Arrival {
        Arrival {
            focused: false,
            ..Default::default()
        }
    }

    #[test]
    fn dms_and_mentions_notify_only_when_away() {
        let p = Prefs::DEFAULT;
        assert!(should_notify(&p, &Arrival { dm: true, ..away() }));
        assert!(should_notify(
            &p,
            &Arrival {
                mentions_me: true,
                ..away()
            }
        ));
        assert!(
            !should_notify(
                &p,
                &Arrival {
                    dm: true,
                    focused: true,
                    ..away()
                }
            ),
            "the window is in front"
        );
        assert!(!should_notify(&p, &away()), "an ordinary server message");
    }

    #[test]
    fn never_for_ours_muted_or_when_off() {
        let p = Prefs::DEFAULT;
        assert!(!should_notify(
            &p,
            &Arrival {
                dm: true,
                mine: true,
                ..away()
            }
        ));
        assert!(!should_notify(
            &p,
            &Arrival {
                mentions_me: true,
                muted: true,
                ..away()
            }
        ));
        assert!(!should_notify(&Prefs { enabled: false, ..p }, &Arrival { dm: true, ..away() }));
    }

    #[test]
    fn everyone_follows_its_switch() {
        let all = Arrival {
            mentions_everyone: true,
            ..away()
        };
        assert!(should_notify(&Prefs::DEFAULT, &all));
        assert!(!should_notify(
            &Prefs {
                everyone: false,
                ..Prefs::DEFAULT
            },
            &all
        ));
    }

    #[test]
    fn text_is_short_or_hidden() {
        let p = Prefs::DEFAULT;
        let full = toast_text(&p, "Алекс", "Сервер · #чат", "привет\nмир");
        assert_eq!((full.heading.as_str(), full.place.as_str(), full.body.as_str()), ("Алекс", "Сервер · #чат", "привет мир"));
        assert!(!full.anonymous);
        assert_eq!(toast_text(&p, "Алекс", "", "  ").body, "Вложение");
        // Hidden text hides who wrote it and where, too.
        let hidden = toast_text(&Prefs { show_text: false, ..p }, "Алекс", "Сервер · #чат", "секрет");
        assert!(hidden.anonymous);
        assert_eq!((hidden.heading.as_str(), hidden.place.as_str(), hidden.body.as_str()), ("Nimbus", "", "Новое сообщение"));
        let long = toast_text(&p, "Алекс", "", &"я".repeat(300)).body;
        assert_eq!(long.chars().count(), 141);
        assert!(long.ends_with('…'));
        assert_eq!(initial_of("марк"), "М");
        assert_eq!(initial_of(""), "");
    }

    #[test]
    fn chime_is_a_valid_short_wav() {
        let wav = chime_wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(wav.len(), 44 + data_len);
        let seconds = data_len as f32 / 2.0 / 44_100.0;
        assert!(seconds > 0.15 && seconds < 0.5, "{seconds}");
        let peak = wav[44..]
            .chunks(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
            .max()
            .unwrap();
        assert!(peak > 2000 && peak < 12000);
    }

    #[test]
    fn levels_decide_for_servers() {
        let p = Prefs::DEFAULT;
        let plain = |level: Level| Arrival { level, ..away() };
        assert!(should_notify(&p, &plain(Level::All)), "all messages");
        assert!(!should_notify(&p, &plain(Level::Mentions)), "only mentions, none here");
        assert!(should_notify(&p, &Arrival { mentions_me: true, ..plain(Level::Mentions) }));
        assert!(should_notify(&p, &Arrival { mentions_everyone: true, ..plain(Level::Mentions) }));
        assert!(!should_notify(&Prefs { everyone: false, ..p }, &Arrival { mentions_everyone: true, ..plain(Level::Mentions) }));
        assert!(!should_notify(&p, &Arrival { mentions_me: true, ..plain(Level::Nothing) }), "nothing");
        assert!(!should_notify(&p, &Arrival { muted: true, ..plain(Level::All) }), "muted beats all messages");
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-01T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    #[test]
    fn settings_come_from_ready_and_updates() {
        let ready = json!({ "user_guild_settings": { "entries": [
            { "guild_id": "g1", "message_notifications": 0, "muted": false, "suppress_everyone": true,
              "channel_overrides": [
                { "channel_id": "c1", "message_notifications": 2, "muted": false },
                { "channel_id": "c2", "message_notifications": 3, "muted": true },
              ] },
            { "guild_id": "g2", "message_notifications": 1, "muted": true, "mute_config": { "end_time": "2026-11-01T11:00:00Z" } },
            { "guild_id": "g3", "muted": true, "mute_config": { "end_time": null } },
            { "guild_id": null, "channel_overrides": [{ "channel_id": "dm1", "muted": true }] },
        ] } });
        let mut s = Settings::from_ready(&ready, now());
        assert_eq!(s.level("c1", Some("g1")), Level::Nothing, "the channel's own choice");
        assert_eq!(s.level("c2", Some("g1")), Level::All, "3: as the server");
        assert_eq!(s.level("c9", Some("g1")), Level::All);
        assert_eq!(s.level("x", Some("nowhere")), Level::Mentions, "no choice anywhere");
        assert!(s.muted("c2", Some("g1")));
        assert!(!s.muted("c1", Some("g1")));
        assert!(!s.muted("a", Some("g2")), "the mute ran out an hour ago");
        assert!(s.muted("a", Some("g3")), "muted until turned off");
        assert!(s.muted("dm1", None));
        assert!(s.everyone_silenced(Some("g1")) && !s.everyone_silenced(Some("g2")) && !s.everyone_silenced(None));
        // An update replaces the server's whole entry: c2's mute is gone with it.
        s.apply(&json!({ "guild_id": "g1", "message_notifications": 2, "muted": true, "channel_overrides": [] }), now());
        assert!(!s.channel("c2").muted);
        assert_eq!(s.level("c1", Some("g1")), Level::Nothing);
        assert!(s.muted("c1", Some("g1")));
        assert_eq!(s.guild("g1"), Place { level: Some(Level::Nothing), muted: true });
    }

    #[test]
    fn menu_changes_become_one_patch() {
        assert_eq!(Change::parse("level:1"), Some(Change::Level(Some(Level::Mentions))));
        assert_eq!(Change::parse("level:3"), Some(Change::Level(None)));
        assert_eq!(Change::parse("level:7"), None);
        assert_eq!(Change::parse("mute"), Some(Change::Mute(true)));
        assert_eq!(Change::parse("nonsense"), None);

        let (path, body) = settings_request("111", None, Change::Level(Some(Level::All))).unwrap();
        assert_eq!(path, "/users/@me/guilds/111/settings");
        assert_eq!(body, json!({ "message_notifications": 0 }));
        let (_, body) = settings_request("111", None, Change::Mute(true)).unwrap();
        assert_eq!(body, json!({ "muted": true, "mute_config": { "selected_time_window": -1, "end_time": null } }));
        let (_, body) = settings_request("111", Some("222"), Change::Mute(false)).unwrap();
        assert_eq!(body, json!({ "channel_overrides": { "222": { "muted": false } } }));
        let (_, body) = settings_request("111", Some("222"), Change::Level(None)).unwrap();
        assert_eq!(body, json!({ "channel_overrides": { "222": { "message_notifications": 3 } } }));
        assert!(settings_request("111", None, Change::Level(None)).is_err());
        assert!(settings_request("@me", None, Change::Mute(true)).is_err());
        assert!(settings_request("111", Some("2/2"), Change::Mute(true)).is_err());
    }

    #[test]
    fn prefs_round_trip() {
        let p = Prefs {
            sound: false,
            everyone: false,
            ..Prefs::DEFAULT
        };
        assert_eq!(Prefs::from_json(&p.to_json()), p);
        assert_eq!(Prefs::from_json(&json!(null)), Prefs::DEFAULT);
    }
}
