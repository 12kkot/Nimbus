//! Voice messages: the player row's waveform and times, and the .ogg file turned into sound.
//! The file is Opus in an Ogg container; the pages are read here, the packets decoded by
//! the Opus decoder Songbird already brings.

use base64::Engine;
use serde_json::Value;

/// Discord marks a message holding a voice message with this flag.
pub const IS_VOICE_MESSAGE: u64 = 1 << 13;
/// Bars drawn in the player row.
pub const BARS: usize = 48;
/// The biggest voice message file fetched.
pub const MAX_BYTES: u64 = 10 * 1024 * 1024;
/// Longest sound decoded, 48 kHz mono: fifteen minutes.
const MAX_SAMPLES: usize = 48_000 * 60 * 15;

/// A voice message attachment.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub url: String,
    pub secs: f64,
    /// Bar heights, 0..1.
    pub bars: Vec<f32>,
}

/// Whether an attachment is a voice message (it has a waveform and a length).
pub fn is_voice(attachment: &Value) -> bool {
    attachment["waveform"].is_string() && attachment["duration_secs"].is_number()
}

/// The voice message of a message, if it is one.
pub fn clip_of(m: &Value) -> Option<Clip> {
    if m["flags"].as_u64().unwrap_or(0) & IS_VOICE_MESSAGE == 0 {
        return None;
    }
    let a = m["attachments"].as_array()?.iter().find(|a| is_voice(a))?;
    Some(Clip { url: a["url"].as_str()?.to_string(), secs: a["duration_secs"].as_f64()?.max(0.0), bars: bars(a["waveform"].as_str()?, BARS) })
}

/// The waveform Discord keeps (base64, one byte of loudness per step) squeezed or stretched to `n` bars.
pub fn bars(waveform: &str, n: usize) -> Vec<f32> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(waveform.trim()).unwrap_or_default();
    if bytes.is_empty() || n == 0 {
        return vec![0.0; n];
    }
    (0..n)
        .map(|i| {
            let (from, to) = (i * bytes.len() / n, ((i + 1) * bytes.len() / n).max(i * bytes.len() / n + 1));
            let part = &bytes[from..to.min(bytes.len())];
            let top = part.iter().copied().max().unwrap_or(0);
            top as f32 / 255.0
        })
        .collect()
}

/// "0:07", "12:34".
pub fn clock(secs: f64) -> String {
    let secs = secs.max(0.0).round() as u64;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// The packets of an Ogg stream, in order (pages of other streams are skipped).
pub fn ogg_packets(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut packets = Vec::new();
    let mut partial: Vec<u8> = Vec::new();
    let mut serial = None;
    let mut at = 0;
    while at < bytes.len() {
        let head = bytes.get(at..at + 27).ok_or("cut-off Ogg page")?;
        if &head[0..4] != b"OggS" || head[4] != 0 {
            return Err("not an Ogg page".into());
        }
        let page_serial = u32::from_le_bytes([head[14], head[15], head[16], head[17]]);
        let count = head[26] as usize;
        let lacing = bytes.get(at + 27..at + 27 + count).ok_or("cut-off Ogg page")?;
        let size: usize = lacing.iter().map(|&l| l as usize).sum();
        let mut body = bytes.get(at + 27 + count..at + 27 + count + size).ok_or("cut-off Ogg page")?;
        at += 27 + count + size;
        if *serial.get_or_insert(page_serial) != page_serial {
            continue;
        }
        // A page that doesn't continue a packet drops whatever was left unfinished.
        if head[5] & 1 == 0 {
            partial.clear();
        }
        for &l in lacing {
            partial.extend_from_slice(&body[..l as usize]);
            body = &body[l as usize..];
            // A lacing value below 255 ends the packet.
            if l < 255 {
                packets.push(std::mem::take(&mut partial));
            }
        }
    }
    Ok(packets)
}

/// What the Opus header says: channels and the samples to drop at the start.
fn opus_head(packet: &[u8]) -> Option<(u8, usize)> {
    if packet.len() < 19 || &packet[0..8] != b"OpusHead" {
        return None;
    }
    Some((packet[9], u16::from_le_bytes([packet[10], packet[11]]) as usize))
}

/// A voice message file as 48 kHz mono samples.
pub fn decode(bytes: &[u8]) -> Result<Vec<i16>, String> {
    use songbird::driver::opus::{Channels, Decoder};
    let packets = ogg_packets(bytes)?;
    let (channels, skip) = packets.first().and_then(|p| opus_head(p)).ok_or("no Opus header")?;
    let stereo = channels == 2;
    let mut decoder = Decoder::new(48_000, if stereo { Channels::Stereo } else { Channels::Mono }).map_err(|e| format!("Opus {e:?}"))?;
    let mut out = Vec::new();
    let mut pcm = vec![0i16; 5760 * 2]; // Opus's longest frame, 120 ms, two channels
    // The first packet is the header, the second the tags.
    for packet in packets.iter().skip(2).filter(|p| !p.is_empty()) {
        let n = decoder.decode(packet, &mut pcm, false).map_err(|e| format!("Opus {e:?}"))?;
        if stereo {
            out.extend(pcm[..n * 2].chunks_exact(2).map(|s| ((s[0] as i32 + s[1] as i32) / 2) as i16));
        } else {
            out.extend_from_slice(&pcm[..n]);
        }
        if out.len() >= MAX_SAMPLES + skip {
            break;
        }
    }
    out.drain(..skip.min(out.len()));
    out.truncate(MAX_SAMPLES);
    Ok(out)
}

// ---------- Recording one ----------

/// The name Discord gives voice message files.
pub const FILENAME: &str = "voice-message.ogg";
/// The longest voice message recorded: twenty minutes.
pub const MAX_RECORD_SECS: usize = 20 * 60;
/// Steps of loudness in the waveform sent along.
pub const WAVEFORM_STEPS: usize = 256;
/// One Opus packet: 20 ms at 48 kHz.
const PACKET_SAMPLES: usize = 960;
/// Samples the Opus encoder runs ahead; the player drops them at the start.
const PRE_SKIP: u16 = 312;

/// Ogg's page checksum: CRC-32 with the polynomial 0x04c11db7, not reflected, starting at zero.
pub fn ogg_crc(bytes: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04c1_1db7 } else { r << 1 };
            }
            *slot = r;
        }
        t
    });
    bytes.iter().fold(0u32, |crc, &b| (crc << 8) ^ table[((crc >> 24) as u8 ^ b) as usize])
}

/// One Ogg page: `flags` (1 continued, 2 first, 4 last), the granule position, the packets' lacing and bodies.
fn ogg_page(flags: u8, granule: u64, serial: u32, seq: u32, packets: &[&[u8]]) -> Vec<u8> {
    let mut lacing = Vec::new();
    for p in packets {
        lacing.extend(std::iter::repeat_n(255u8, p.len() / 255));
        lacing.push((p.len() % 255) as u8);
    }
    let mut page = b"OggS".to_vec();
    page.push(0);
    page.push(flags);
    page.extend_from_slice(&granule.to_le_bytes());
    page.extend_from_slice(&serial.to_le_bytes());
    page.extend_from_slice(&seq.to_le_bytes());
    page.extend_from_slice(&[0; 4]);
    page.push(lacing.len() as u8);
    page.extend_from_slice(&lacing);
    for p in packets {
        page.extend_from_slice(p);
    }
    let crc = ogg_crc(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

/// Lacing values a packet takes on a page.
fn lacing_len(packet: &[u8]) -> usize {
    packet.len() / 255 + 1
}

/// An Ogg Opus file (48 kHz mono) of Opus packets of 20 ms each; `samples` is the sound's real length,
/// so the player cuts the silence padding the last packet.
pub fn ogg_opus(packets: &[Vec<u8>], samples: u64) -> Vec<u8> {
    const SERIAL: u32 = 0x4e42_5653;
    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[1, 1]);
    head.extend_from_slice(&PRE_SKIP.to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes());
    head.extend_from_slice(&[0, 0, 0]);
    let vendor = b"Nimbus";
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    tags.extend_from_slice(vendor);
    tags.extend_from_slice(&0u32.to_le_bytes());

    let mut out = ogg_page(2, 0, SERIAL, 0, &[&head]);
    out.extend(ogg_page(0, 0, SERIAL, 1, &[&tags]));
    let end = PRE_SKIP as u64 + samples;
    let (mut seq, mut done, mut page): (u32, u64, Vec<&[u8]>) = (2, 0, Vec::new());
    let mut segments = 0;
    for (i, packet) in packets.iter().enumerate() {
        page.push(packet);
        segments += lacing_len(packet);
        done += PACKET_SAMPLES as u64;
        let next = packets.get(i + 1).map_or(0, |p| lacing_len(p));
        // About a second to a page, never more than its 255 lacing values.
        if page.len() == 50 || segments + next > 255 || i + 1 == packets.len() {
            let last = i + 1 == packets.len();
            let granule = if last { end } else { (PRE_SKIP as u64 + done).min(end) };
            out.extend(ogg_page(if last { 4 } else { 0 }, granule, SERIAL, seq, &page));
            seq += 1;
            page.clear();
            segments = 0;
        }
    }
    if packets.is_empty() {
        out.extend(ogg_page(4, end, SERIAL, seq, &[]));
    }
    out
}

/// 48 kHz mono sound as an Ogg Opus file.
pub fn encode(samples: &[i16]) -> Result<Vec<u8>, String> {
    use songbird::driver::opus::{Application, Bitrate, Channels, Encoder};
    let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Voip).map_err(|e| format!("Opus {e:?}"))?;
    let _ = encoder.set_bitrate(Bitrate::Bits(32_000));
    let mut packets = Vec::with_capacity(samples.len() / PACKET_SAMPLES + 1);
    let mut frame = [0i16; PACKET_SAMPLES];
    for chunk in samples.chunks(PACKET_SAMPLES) {
        frame.fill(0);
        frame[..chunk.len()].copy_from_slice(chunk);
        let mut buf = vec![0u8; 1500];
        let len = encoder.encode(&frame, &mut buf).map_err(|e| format!("Opus {e:?}"))?;
        buf.truncate(len);
        packets.push(buf);
    }
    Ok(ogg_opus(&packets, samples.len() as u64))
}

/// The sound's loudness in `WAVEFORM_STEPS` steps, the loudest at 255, as Discord keeps it (base64).
pub fn waveform(samples: &[i16]) -> String {
    let steps: Vec<f64> = (0..WAVEFORM_STEPS)
        .map(|i| {
            let part = &samples[i * samples.len() / WAVEFORM_STEPS..(i + 1) * samples.len() / WAVEFORM_STEPS];
            if part.is_empty() {
                return 0.0;
            }
            (part.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / part.len() as f64).sqrt()
        })
        .collect();
    let top = steps.iter().copied().fold(0.0, f64::max);
    let bytes: Vec<u8> = steps.iter().map(|&s| if top > 0.0 { (s / top * 255.0).round() as u8 } else { 0 }).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The length of 48 kHz sound, in seconds, to a hundredth.
pub fn duration_secs(samples: usize) -> f64 {
    (samples as f64 / 480.0).round() / 100.0
}

/// The request for a place to put the file.
pub fn slot_request(channel: &str, filename: &str, size: u64) -> Result<(String, Value), String> {
    let body = serde_json::json!({ "files": [{ "filename": filename, "file_size": size, "id": "0", "is_clip": false }] });
    Ok((format!("/channels/{}/attachments", crate::discord::snowflake(channel)?), body))
}

/// Discord's answer to that: where to put the file, and its name there.
pub fn slot_of(answer: &Value) -> Option<(String, String)> {
    let a = answer["attachments"].as_array()?.first()?;
    Some((a["upload_url"].as_str()?.to_string(), a["upload_filename"].as_str().filter(|f| !f.is_empty())?.to_string()))
}

/// The address to put the file at, if it is Discord's own storage (or Google's, which keeps it for Discord) over https.
pub fn upload_url(url: &str) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "the upload address is not an address".to_string())?;
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    let trusted = ["storage.googleapis.com", "discord.com", "discordapp.com", "discordapp.net"].iter().any(|d| under(d));
    if parsed.scheme() != "https" || !trusted || crate::discord::is_internal_url(&parsed) || parsed.port().is_some_and(|p| p != 443) {
        return Err(format!("the upload address is not Discord's storage: {host}"));
    }
    Ok(parsed)
}

/// The message that carries the uploaded file as a voice message.
pub fn message_request(channel: &str, uploaded: &str, secs: f64, waveform: &str) -> Result<(String, Value), String> {
    let body = serde_json::json!({
        "flags": IS_VOICE_MESSAGE,
        "attachments": [{ "id": "0", "filename": FILENAME, "uploaded_filename": uploaded, "duration_secs": secs, "waveform": waveform }],
    });
    Ok((format!("/channels/{}/messages", crate::discord::snowflake(channel)?), body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One Ogg page holding `packets` (the last may go on in the next page when `open`).
    fn page(serial: u32, seq: u32, continued: bool, packets: &[&[u8]], open: bool) -> Vec<u8> {
        let mut lacing = Vec::new();
        for (i, p) in packets.iter().enumerate() {
            lacing.extend(std::iter::repeat_n(255u8, p.len() / 255));
            if !(open && i == packets.len() - 1) {
                lacing.push((p.len() % 255) as u8);
            }
        }
        let mut out = b"OggS".to_vec();
        out.push(0);
        out.push(if continued { 1 } else { 0 });
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&serial.to_le_bytes());
        out.extend_from_slice(&seq.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // checksum, not checked
        out.push(lacing.len() as u8);
        out.extend_from_slice(&lacing);
        for p in packets {
            out.extend_from_slice(p);
        }
        out
    }

    fn head(channels: u8, skip: u16) -> Vec<u8> {
        let mut h = b"OpusHead".to_vec();
        h.push(1);
        h.push(channels);
        h.extend_from_slice(&skip.to_le_bytes());
        h.extend_from_slice(&48_000u32.to_le_bytes());
        h.extend_from_slice(&[0, 0, 0]);
        h
    }

    #[test]
    fn ogg_pages_give_back_their_packets() {
        let long = vec![7u8; 600]; // three lacing values: 255, 255, 90
        let mut file = page(5, 0, false, &[b"first", &long], false);
        // A packet split over two pages: 255 bytes (open), then the other 45.
        let split = vec![9u8; 300];
        file.extend(page(5, 1, false, &[&split[..255]], true));
        file.extend(page(6, 0, false, &[b"other stream"], false));
        file.extend(page(5, 2, true, &[&split[255..], b"last"], false));
        let packets = ogg_packets(&file).unwrap();
        assert_eq!(packets.len(), 4);
        assert_eq!(packets[0], b"first");
        assert_eq!(packets[1], long);
        assert_eq!(packets[2], split);
        assert_eq!(packets[3], b"last");
        assert!(ogg_packets(b"RIFF....").is_err());
        assert!(ogg_packets(&file[..file.len() - 2]).is_err(), "a cut-off file says so");
        assert_eq!(opus_head(&head(1, 312)), Some((1, 312)));
        assert_eq!(opus_head(b"OpusTags........"), None);
    }

    #[test]
    fn an_opus_file_decodes_to_mono_sound() {
        use songbird::driver::opus::{Application, Channels, Encoder};
        let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Voip).unwrap();
        let tone: Vec<i16> = (0..960).map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16).collect();
        let mut frames = Vec::new();
        for _ in 0..10 {
            let mut buf = vec![0u8; 1500];
            let len = encoder.encode(&tone, &mut buf).unwrap();
            buf.truncate(len);
            frames.push(buf);
        }
        let mut file = page(1, 0, false, &[&head(1, 312)], false);
        file.extend(page(1, 1, false, &[b"OpusTags\0\0\0\0\0\0\0\0"], false));
        let refs: Vec<&[u8]> = frames.iter().map(|f| f.as_slice()).collect();
        file.extend(page(1, 2, false, &refs, false));
        let samples = decode(&file).unwrap();
        assert_eq!(samples.len(), 960 * 10 - 312, "20 ms a packet, the pre-skip dropped");
        assert!(samples.iter().any(|s| s.unsigned_abs() > 1000), "the tone is heard");
        assert!(decode(&page(1, 0, false, &[b"not a header"], false)).is_err());
    }

    #[test]
    fn waveforms_become_bars() {
        // 4 steps: 0, 255, 51, 102.
        let wave = base64::engine::general_purpose::STANDARD.encode([0u8, 255, 51, 102]);
        assert_eq!(bars(&wave, 2), vec![1.0, 0.4]);
        assert_eq!(bars(&wave, 4), vec![0.0, 1.0, 0.2, 0.4]);
        assert_eq!(bars(&wave, 8).len(), 8, "stretched when there are fewer steps");
        assert_eq!(bars("not base64!", 3), vec![0.0; 3]);
        assert_eq!(clock(7.4), "0:07");
        assert_eq!(clock(754.0), "12:34");
    }

    #[test]
    fn voice_messages_are_found_by_flag_and_waveform() {
        let m = json!({ "flags": 8192, "attachments": [{ "url": "https://cdn.discordapp.com/attachments/1/2/voice-message.ogg", "duration_secs": 7.2, "waveform": "AP8zZg==" }] });
        let clip = clip_of(&m).unwrap();
        assert_eq!(clip.url, "https://cdn.discordapp.com/attachments/1/2/voice-message.ogg");
        assert_eq!(clip.secs, 7.2);
        assert_eq!(clip.bars.len(), BARS);
        assert!(clip_of(&json!({ "flags": 0, "attachments": m["attachments"] })).is_none());
        assert!(clip_of(&json!({ "flags": 8192, "attachments": [{ "url": "x", "filename": "a.ogg" }] })).is_none());
    }

    #[test]
    fn the_ogg_checksum_is_crc32_without_reflection() {
        // CRC-32/POSIX checks "123456789" as 0x765e7680; Ogg's is the same without its final inversion.
        assert_eq!(ogg_crc(b"123456789"), !0x765e_7680);
        assert_eq!(ogg_crc(b""), 0);
    }

    /// Every page of a file has the checksum Ogg readers expect.
    fn pages_check_out(file: &[u8]) -> usize {
        let mut at = 0;
        let mut count = 0;
        while at < file.len() {
            let segments = file[at + 26] as usize;
            let size = 27 + segments + file[at + 27..at + 27 + segments].iter().map(|&l| l as usize).sum::<usize>();
            let mut page = file[at..at + size].to_vec();
            let said = u32::from_le_bytes([page[22], page[23], page[24], page[25]]);
            page[22..26].fill(0);
            assert_eq!(ogg_crc(&page), said, "page {count}");
            at += size;
            count += 1;
        }
        count
    }

    #[test]
    fn a_recorded_sound_goes_through_ogg_and_back() {
        // 2.5 s of a tone: 125 packets, so more than one page of sound.
        let tone: Vec<i16> = (0..120_000).map(|i| ((i as f32 * 0.05).sin() * 9000.0) as i16).collect();
        let file = encode(&tone).unwrap();
        assert!(pages_check_out(&file) >= 4, "head, tags and at least two pages of sound");
        assert_eq!(&file[5..6], &[2], "the first page starts the stream");
        let packets = ogg_packets(&file).unwrap();
        assert_eq!(opus_head(&packets[0]), Some((1, PRE_SKIP as usize)));
        assert!(packets[1].starts_with(b"OpusTags"));
        assert_eq!(packets.len(), 2 + 125);
        let back = decode(&file).unwrap();
        assert_eq!(back.len(), 125 * 960 - PRE_SKIP as usize);
        assert!(back.iter().any(|s| s.unsigned_abs() > 2000), "the tone is heard");
        // The last page ends the stream at the sound's real length.
        let last = file.windows(4).rposition(|w| w == b"OggS").unwrap();
        assert_eq!(file[last + 5], 4);
        assert_eq!(u64::from_le_bytes(file[last + 6..last + 14].try_into().unwrap()), PRE_SKIP as u64 + 120_000);
        // A packet bigger than a lacing value spans several, and the pages still check out.
        let big = ogg_opus(&[vec![1u8; 700], vec![2u8; 3]], 1920);
        pages_check_out(&big);
        assert_eq!(ogg_packets(&big).unwrap()[2], vec![1u8; 700]);
        assert_eq!(pages_check_out(&ogg_opus(&[], 0)), 3);
    }

    #[test]
    fn the_waveform_has_256_steps_up_to_255() {
        let decode = |w: &str| base64::engine::general_purpose::STANDARD.decode(w).unwrap();
        // Quiet first half, loud second half.
        let sound: Vec<i16> = (0..48_000).map(|i| if i < 24_000 { 1000 } else { -8000 }).collect();
        let steps = decode(&waveform(&sound));
        assert_eq!(steps.len(), WAVEFORM_STEPS);
        assert_eq!(steps[255], 255);
        assert!(steps[0] > 0 && steps[0] < 40, "{}", steps[0]);
        assert!(decode(&waveform(&[0; 4800])).iter().all(|&s| s == 0), "silence stays flat");
        assert_eq!(decode(&waveform(&[])).len(), WAVEFORM_STEPS);
        assert_eq!(decode(&waveform(&[5, -5, 5])).len(), WAVEFORM_STEPS, "shorter than the steps");
        assert_eq!(duration_secs(48_000 * 7 + 24_000), 7.5);
        assert_eq!(duration_secs(0), 0.0);
    }

    #[test]
    fn the_upload_requests_are_discords_voice_message_flow() {
        let (path, body) = slot_request("111", FILENAME, 5000).unwrap();
        assert_eq!(path, "/channels/111/attachments");
        assert_eq!(body, json!({ "files": [{ "filename": "voice-message.ogg", "file_size": 5000, "id": "0", "is_clip": false }] }));
        assert!(slot_request("1/../2", FILENAME, 1).is_err());
        let answer = json!({ "attachments": [{ "id": 0, "upload_url": "https://discord-attachments-uploads-prd.storage.googleapis.com/abc?sig=1", "upload_filename": "abc/voice-message.ogg" }] });
        assert_eq!(slot_of(&answer), Some(("https://discord-attachments-uploads-prd.storage.googleapis.com/abc?sig=1".into(), "abc/voice-message.ogg".into())));
        assert_eq!(slot_of(&json!({ "attachments": [] })), None);
        let (path, body) = message_request("111", "abc/voice-message.ogg", 7.5, "AP8=").unwrap();
        assert_eq!(path, "/channels/111/messages");
        assert_eq!(
            body,
            json!({ "flags": 8192, "attachments": [{ "id": "0", "filename": "voice-message.ogg", "uploaded_filename": "abc/voice-message.ogg", "duration_secs": 7.5, "waveform": "AP8=" }] })
        );
        assert!(message_request("x", "a", 1.0, "").is_err());
    }

    #[test]
    fn files_are_put_only_into_discords_storage() {
        for ok in ["https://discord-attachments-uploads-prd.storage.googleapis.com/a?b=c", "https://storage.googleapis.com/x", "https://cdn.discordapp.com/x", "https://discord.com/api/x"] {
            assert!(upload_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://discord-attachments-uploads-prd.storage.googleapis.com/a",
            "https://evil.com/storage.googleapis.com",
            "https://storage.googleapis.com.evil.com/a",
            "https://notdiscord.com/x",
            "https://127.0.0.1/x",
            "https://storage.googleapis.com:8443/x",
            "file:///C:/x",
            "not a url",
        ] {
            assert!(upload_url(bad).is_err(), "{bad}");
        }
    }
}
