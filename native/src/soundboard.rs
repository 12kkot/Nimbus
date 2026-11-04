//! The soundboard: Discord's own sounds and the server's, the panel that lists them, playing one in the
//! call, and the sounds others play there (fetched once, decoded, kept for a while).

use std::collections::VecDeque;

use serde_json::{json, Value};

/// Discord's own sounds, the same for everyone.
pub const DEFAULTS_PATH: &str = "/soundboard-default-sounds";
/// Sound files weigh well under this (Discord takes up to 512 KB).
pub const MAX_BYTES: u64 = 1024 * 1024;
/// Sounds are at most 5.2 s long; anything past this is cut.
const MAX_SAMPLES: usize = 48_000 * 8;
/// How many decoded sounds are kept.
pub const KEPT: usize = 30;
/// How many sounds the recent section keeps.
pub const RECENT: usize = 6;

/// One sound, as the panel shows it and the play request names it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub id: String,
    pub name: String,
    /// 0..1, how loud Discord plays it.
    pub volume: f64,
    /// A plain emoji, or a custom one's name.
    pub emoji: String,
    pub emoji_id: Option<String>,
    /// The server it belongs to; None for Discord's own.
    pub guild: Option<String>,
}

impl Sound {
    /// `name:id` of its custom emoji, as the emoji pictures are kept by.
    pub fn emoji_key(&self) -> Option<String> {
        self.emoji_id.as_ref().map(|id| format!("{}:{id}", self.emoji))
    }
}

fn digits(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

fn id_of(v: &Value) -> Option<String> {
    v.as_str().map(String::from).or_else(|| v.as_u64().map(|n| n.to_string())).filter(|id| digits(id))
}

fn sound_of(v: &Value, guild: Option<&str>) -> Option<Sound> {
    if v["available"].as_bool() == Some(false) {
        return None;
    }
    Some(Sound {
        id: id_of(&v["sound_id"])?,
        name: v["name"].as_str().filter(|n| !n.trim().is_empty())?.to_string(),
        volume: v["volume"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0),
        emoji: v["emoji_name"].as_str().unwrap_or("").to_string(),
        emoji_id: id_of(&v["emoji_id"]),
        guild: id_of(&v["guild_id"]).or_else(|| guild.map(String::from)),
    })
}

/// A list of sounds: a bare array (Discord's own) or `{ items: [...] }` (a server's). Sounds that can't be
/// used now (`available: false`) and entries without an id or a name are left out. `guild` is the server
/// asked about, for entries that don't name it.
pub fn parse(v: &Value, guild: Option<&str>) -> Vec<Sound> {
    v.as_array().or_else(|| v["items"].as_array()).into_iter().flatten().filter_map(|s| sound_of(s, guild)).collect()
}

/// Where a server's sounds are asked for.
pub fn guild_path(guild: &str) -> Result<String, String> {
    Ok(format!("/guilds/{}/soundboard-sounds", crate::snowflake(guild)?))
}

/// The request that plays a sound for everyone in the voice channel. Discord's own sounds name no server.
pub fn play_request(channel: &str, sound: &Sound) -> Result<(String, Value), String> {
    let mut body = json!({ "sound_id": sound.id });
    if let Some(guild) = &sound.guild {
        body["source_guild_id"] = json!(guild);
    }
    Ok((format!("/channels/{}/send-soundboard-sound", crate::snowflake(channel)?), body))
}

/// The sound's file on Discord's CDN.
pub fn file_url(id: &str) -> Option<String> {
    digits(id).then(|| format!("https://cdn.discordapp.com/soundboard-sounds/{id}"))
}

/// A sound someone played in `voice_channel` (VOICE_CHANNEL_EFFECT_SEND): its id and loudness. Effects
/// in other channels, and emoji reactions without a sound, are None.
pub fn effect(v: &Value, voice_channel: &str) -> Option<(String, f64)> {
    if v["channel_id"].as_str() != Some(voice_channel) {
        return None;
    }
    Some((id_of(&v["sound_id"])?, v["sound_volume"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0)))
}

/// Whether a sound matches what's typed in the panel's search (by any word of its name, or its emoji).
pub fn matches(sound: &Sound, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    let name = sound.name.to_lowercase();
    query.is_empty() || name.starts_with(&query) || name.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(&query)) || sound.emoji == query
}

/// A part of the panel: 0 recent, 1 the server's sounds, 2 Discord's own.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub kind: i32,
    pub sounds: Vec<Sound>,
}

/// The panel for what's typed: the recently played (of those usable here), the server's, Discord's own;
/// empty sections are left out. Only this server's sounds and Discord's own can be played without Nitro,
/// so others' never come here.
pub fn sections(server: &[Sound], defaults: &[Sound], recent: &[String], query: &str) -> Vec<Section> {
    let usable = || server.iter().chain(defaults);
    let recent_sounds: Vec<Sound> = recent.iter().filter_map(|id| usable().find(|s| s.id == *id)).filter(|s| matches(s, query)).cloned().collect();
    let pick = |list: &[Sound]| list.iter().filter(|s| matches(s, query)).cloned().collect::<Vec<_>>();
    [(0, recent_sounds), (1, pick(server)), (2, pick(defaults))]
        .into_iter()
        .filter(|(_, sounds)| !sounds.is_empty())
        .map(|(kind, sounds)| Section { kind, sounds })
        .collect()
}

/// The recent list with `id` played just now: it goes first, the oldest drops off.
pub fn remember(recent: &mut Vec<String>, id: &str) {
    recent.retain(|r| r != id);
    recent.insert(0, id.to_string());
    recent.truncate(RECENT);
}

/// Where the panel's headers and tiles go: tiles `per_row` to a row under each section's header.
/// Returns, in order, (section index, sound index or None for the header, x, y), and the whole height.
pub fn layout(sections: &[Section], per_row: usize, tile: (f32, f32), header: f32, gap: f32) -> (Vec<(usize, Option<usize>, f32, f32)>, f32) {
    let (mut out, mut y) = (Vec::new(), 0.0);
    for (si, section) in sections.iter().enumerate() {
        out.push((si, None, 0.0, y));
        y += header;
        for (i, _) in section.sounds.iter().enumerate() {
            let (col, row) = (i % per_row, i / per_row);
            out.push((si, Some(i), col as f32 * (tile.0 + gap), y + row as f32 * (tile.1 + gap)));
        }
        let rows = section.sounds.len().div_ceil(per_row);
        y += rows as f32 * (tile.1 + gap) + gap;
    }
    (out, y)
}

/// A sound file (MP3, or Ogg Opus) as 48 kHz mono samples, at most a few seconds.
pub fn decode(bytes: &[u8]) -> Result<Vec<i16>, String> {
    let mut samples = if bytes.starts_with(b"OggS") { crate::voicemsg::decode(bytes)? } else { decode_mp3(bytes)? };
    samples.truncate(MAX_SAMPLES);
    Ok(samples)
}

fn decode_mp3(bytes: &[u8]) -> Result<Vec<i16>, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let source = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe().format(&hint, source, &FormatOptions::default(), &MetadataOptions::default()).map_err(|e| e.to_string())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| e.to_string())?;
    let (mut mono, mut rate) = (Vec::new(), 0u32);
    // Read past the cut-off at 48 kHz, whatever the file's own rate.
    while mono.len() < MAX_SAMPLES * 2 {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(_)) | Err(Error::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A damaged frame is skipped, as players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let channels = spec.channels.count().max(1);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        mono.extend(buffer.samples().chunks_exact(channels).map(|frame| frame.iter().sum::<f32>() / channels as f32));
    }
    if rate == 0 {
        return Err("no sound in the file".into());
    }
    Ok(resample(&mono, rate))
}

/// Mono samples (-1..1) at `rate` as 48 kHz 16-bit, by straight lines between neighbours.
pub fn resample(mono: &[f32], rate: u32) -> Vec<i16> {
    if mono.is_empty() || rate == 0 {
        return Vec::new();
    }
    let step = rate as f64 / 48_000.0;
    let n = ((mono.len() as f64 / step) as usize).min(MAX_SAMPLES);
    (0..n)
        .map(|i| {
            let at = i as f64 * step;
            let j = at as usize;
            let frac = (at - j as f64) as f32;
            let a = mono[j.min(mono.len() - 1)];
            let b = mono[(j + 1).min(mono.len() - 1)];
            ((a + (b - a) * frac).clamp(-1.0, 1.0) * i16::MAX as f32) as i16
        })
        .collect()
}

/// The sound at `volume` (0..1).
pub fn scaled(samples: &[i16], volume: f64) -> Vec<i16> {
    let v = volume.clamp(0.0, 1.0) as f32;
    samples.iter().map(|&s| (s as f32 * v) as i16).collect()
}

/// The last few things used, by key: a lookup moves one to the front, the oldest goes when full.
pub struct Kept<V> {
    items: VecDeque<(String, V)>,
    size: usize,
}

impl<V: Clone> Kept<V> {
    pub fn new(size: usize) -> Self {
        Self { items: VecDeque::new(), size }
    }

    pub fn get(&mut self, key: &str) -> Option<V> {
        let at = self.items.iter().position(|(k, _)| k == key)?;
        let item = self.items.remove(at)?;
        let value = item.1.clone();
        self.items.push_front(item);
        Some(value)
    }

    pub fn put(&mut self, key: &str, value: V) {
        self.items.retain(|(k, _)| k != key);
        self.items.push_front((key.to_string(), value));
        self.items.truncate(self.size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_of_both_kinds_are_read() {
        let defaults = parse(&json!([
            { "sound_id": "1", "name": "quack", "volume": 1.0, "emoji_name": "🦆", "emoji_id": null, "available": true },
            { "sound_id": 2, "name": "airhorn", "volume": 0.5, "emoji_name": "🔊" },
            { "sound_id": "3", "name": "  " },
            { "name": "no id" },
        ]), None);
        assert_eq!(defaults.len(), 2);
        assert_eq!(defaults[1].id, "2");
        assert_eq!(defaults[1].volume, 0.5);
        assert_eq!(defaults[0].guild, None);
        let server = parse(&json!({ "items": [
            { "sound_id": "10", "name": "мем", "volume": 2.0, "emoji_name": "pog", "emoji_id": "77" },
            { "sound_id": "11", "name": "старый", "available": false },
        ] }), Some("5"));
        assert_eq!(server.len(), 1, "unavailable sounds are left out");
        assert_eq!(server[0].guild.as_deref(), Some("5"));
        assert_eq!(server[0].volume, 1.0);
        assert_eq!(server[0].emoji_key().as_deref(), Some("pog:77"));
        assert!(parse(&json!({ "message": "Missing Access" }), None).is_empty());
    }

    #[test]
    fn playing_names_the_server_only_for_its_sounds() {
        let own = Sound { id: "10".into(), name: "a".into(), volume: 1.0, emoji: String::new(), emoji_id: None, guild: Some("5".into()) };
        let (path, body) = play_request("99", &own).unwrap();
        assert_eq!(path, "/channels/99/send-soundboard-sound");
        assert_eq!(body, json!({ "sound_id": "10", "source_guild_id": "5" }));
        let discords = Sound { guild: None, ..own };
        assert_eq!(play_request("99", &discords).unwrap().1, json!({ "sound_id": "10" }));
        assert!(play_request("../x", &discords).is_err());
        assert_eq!(guild_path("5").unwrap(), "/guilds/5/soundboard-sounds");
        assert_eq!(file_url("10").as_deref(), Some("https://cdn.discordapp.com/soundboard-sounds/10"));
        assert_eq!(file_url("10/../x"), None);
    }

    #[test]
    fn effects_count_only_in_our_channel() {
        let e = json!({ "channel_id": "7", "guild_id": "5", "user_id": "1", "sound_id": "10", "sound_volume": 0.4 });
        assert_eq!(effect(&e, "7"), Some(("10".into(), 0.4)));
        assert_eq!(effect(&e, "8"), None);
        assert_eq!(effect(&json!({ "channel_id": "7", "emoji": { "name": "🔥" } }), "7"), None);
    }

    fn sound(id: &str, name: &str, guild: Option<&str>) -> Sound {
        Sound { id: id.into(), name: name.into(), volume: 1.0, emoji: String::new(), emoji_id: None, guild: guild.map(String::from) }
    }

    #[test]
    fn the_panel_has_recent_server_and_discord_sections() {
        let server = vec![sound("10", "Bruh moment", Some("5")), sound("11", "вжух", Some("5"))];
        let defaults = vec![sound("1", "quack", None), sound("2", "airhorn", None)];
        let recent = vec!["2".to_string(), "999".to_string()];
        let all = sections(&server, &defaults, &recent, "");
        assert_eq!(all.iter().map(|s| (s.kind, s.sounds.len())).collect::<Vec<_>>(), [(0, 1), (1, 2), (2, 2)]);
        assert_eq!(all[0].sounds[0].name, "airhorn", "a recent sound not usable here is left out");
        let found = sections(&server, &defaults, &recent, "MOM");
        assert_eq!(found.iter().map(|s| s.kind).collect::<Vec<_>>(), [1]);
        assert!(sections(&[], &defaults, &[], "zzz").is_empty());
        let mut recent = Vec::new();
        for id in ["1", "2", "3", "4", "5", "6", "7", "2"] {
            remember(&mut recent, id);
        }
        assert_eq!(recent, ["2", "7", "6", "5", "4", "3"]);
    }

    #[test]
    fn tiles_go_three_to_a_row_under_headers() {
        let s = |n: usize| Section { kind: 1, sounds: (0..n).map(|i| sound(&i.to_string(), "x", None)).collect() };
        let (placed, height) = layout(&[s(4), s(1)], 3, (90.0, 40.0), 30.0, 6.0);
        assert_eq!(placed[0], (0, None, 0.0, 0.0));
        assert_eq!(placed[1], (0, Some(0), 0.0, 30.0));
        assert_eq!(placed[3], (0, Some(2), 192.0, 30.0));
        assert_eq!(placed[4], (0, Some(3), 0.0, 76.0));
        // Two rows of the first, a gap, then the second's header.
        assert_eq!(placed[5], (1, None, 0.0, 128.0));
        assert_eq!(height, 128.0 + 30.0 + 46.0 + 6.0);
    }

    #[test]
    fn sound_is_made_48k_and_scaled() {
        let half_rate: Vec<f32> = vec![0.0, 1.0, 0.0, -1.0];
        let up = resample(&half_rate, 24_000);
        assert_eq!(up.len(), 8);
        assert_eq!(up[1], i16::MAX / 2);
        assert_eq!(up[2], i16::MAX);
        assert!(resample(&[], 44_100).is_empty());
        assert_eq!(scaled(&[1000, -1000], 0.5), [500, -500]);
        assert!(decode(b"not a sound").is_err());
        assert!(decode(b"OggS broken").is_err());
    }

    #[test]
    fn kept_sounds_drop_the_least_used() {
        let mut kept = Kept::new(2);
        kept.put("a", 1);
        kept.put("b", 2);
        assert_eq!(kept.get("a"), Some(1));
        kept.put("c", 3);
        assert_eq!(kept.get("b"), None, "b was used least lately");
        assert_eq!(kept.get("a"), Some(1));
        assert_eq!(kept.get("c"), Some(3));
    }
}
