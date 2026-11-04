//! How times and dates are written in messages: 24 or 12 hours, and the date's shape.

use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, Timelike};

static TIME12: AtomicBool = AtomicBool::new(false);
static DATE_ISO: AtomicBool = AtomicBool::new(false);

pub fn set(time12: bool, date_iso: bool) {
    TIME12.store(time12, Ordering::Relaxed);
    DATE_ISO.store(date_iso, Ordering::Relaxed);
}

fn hm_of(t: &NaiveDateTime, time12: bool) -> String {
    if time12 {
        let (pm, hour) = t.hour12();
        format!("{hour}:{:02} {}", t.minute(), if pm { "PM" } else { "AM" })
    } else {
        format!("{:02}:{:02}", t.hour(), t.minute())
    }
}

fn date_of(d: NaiveDate, iso: bool) -> String {
    if iso {
        d.format("%Y-%m-%d").to_string()
    } else {
        d.format("%d.%m.%Y").to_string()
    }
}

/// "Сегодня, 21:52", "Вчера, 9:52 PM", or the date and time.
fn stamp_of(t: NaiveDateTime, today: NaiveDate, time12: bool, iso: bool) -> String {
    let hm = hm_of(&t, time12);
    if t.date() == today {
        crate::i18n::tf("Сегодня, {}", &[&hm])
    } else if Some(t.date()) == today.pred_opt() {
        crate::i18n::tf("Вчера, {}", &[&hm])
    } else {
        format!("{}, {hm}", date_of(t.date(), iso))
    }
}

pub fn stamp(t: &DateTime<Local>) -> String {
    stamp_of(
        t.naive_local(),
        Local::now().date_naive(),
        TIME12.load(Ordering::Relaxed),
        DATE_ISO.load(Ordering::Relaxed),
    )
}

/// A time of day and a date as the settings write them (for scheduled events).
pub fn hm(t: &NaiveDateTime) -> String {
    hm_of(t, TIME12.load(Ordering::Relaxed))
}

pub fn date(d: NaiveDate) -> String {
    date_of(d, DATE_ISO.load(Ordering::Relaxed))
}

/// The day an account was made, read from its id (ids hold the moment they were made).
pub fn joined(id: &str) -> Option<String> {
    const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;
    let id: i64 = id.parse().ok()?;
    let at = DateTime::from_timestamp_millis((id >> 22) + DISCORD_EPOCH_MS)?;
    Some(date_of(at.with_timezone(&Local).date_naive(), DATE_ISO.load(Ordering::Relaxed)))
}

/// Just the time, for the small stamp of a grouped message.
pub fn short(t: &DateTime<Local>) -> String {
    hm_of(&t.naive_local(), TIME12.load(Ordering::Relaxed))
}

/// "через 2 ч", "5 мин назад": a moment against now, for Discord's relative stamps.
fn relative(secs: i64) -> String {
    let (ahead, n) = (secs > 0, secs.unsigned_abs());
    let amount = match n {
        0..=59 => return crate::i18n::t("сейчас"),
        60..=3599 => crate::i18n::tf("{} мин", &[&(n / 60)]),
        3600..=86_399 => crate::i18n::tf("{} ч", &[&(n / 3600)]),
        86_400..=2_591_999 => crate::i18n::tf("{} дн", &[&(n / 86_400)]),
        2_592_000..=31_535_999 => crate::i18n::tf("{} мес", &[&(n / 2_592_000)]),
        _ => crate::i18n::tf("{} г", &[&(n / 31_536_000)]),
    };
    if ahead { crate::i18n::tf("через {}", &[&amount]) } else { crate::i18n::tf("{} назад", &[&amount]) }
}

/// One Discord time stamp (`<t:1780383600:t>`) as text, by its style letter.
fn stamp_text(t: NaiveDateTime, style: &str, now: NaiveDateTime, time12: bool, iso: bool) -> String {
    let hm = hm_of(&t, time12);
    match style {
        "t" => hm,
        "T" if time12 => t.format("%-I:%M:%S %p").to_string(),
        "T" => t.format("%H:%M:%S").to_string(),
        "d" | "D" => date_of(t.date(), iso),
        "R" => relative((t - now).num_seconds()),
        _ => format!("{} {hm}", date_of(t.date(), iso)),
    }
}

/// Text with Discord's time stamps (`<t:SECONDS>` or `<t:SECONDS:STYLE>`) written out in local time,
/// in the chosen clock and date shape.
pub fn discord_stamps(text: &str) -> String {
    use std::sync::OnceLock;
    static STAMP: OnceLock<regex::Regex> = OnceLock::new();
    if !text.contains("<t:") {
        return text.to_string();
    }
    let re = STAMP.get_or_init(|| regex::Regex::new(r"<t:(-?\d{1,13})(?::([tTdDfFR]))?>").expect("valid regex"));
    let now = Local::now().naive_local();
    let (time12, iso) = (TIME12.load(Ordering::Relaxed), DATE_ISO.load(Ordering::Relaxed));
    re.replace_all(text, |c: &regex::Captures| {
        let Some(at) = c[1].parse::<i64>().ok().and_then(|s| DateTime::from_timestamp(s, 0)) else { return c[0].to_string() };
        stamp_text(at.with_timezone(&Local).naive_local(), c.get(2).map_or("f", |m| m.as_str()), now, time12, iso)
    })
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_stamps_by_style() {
        let t = at(2026, 6, 2, 9, 0);
        let now = at(2026, 6, 2, 7, 0);
        assert_eq!(stamp_text(t, "t", now, false, false), "09:00");
        assert_eq!(stamp_text(t, "t", now, true, false), "9:00 AM");
        assert_eq!(stamp_text(t, "d", now, false, true), "2026-06-02");
        assert_eq!(stamp_text(t, "R", now, false, false), "через 2 ч");
        assert_eq!(stamp_text(now, "R", t, false, false), "2 ч назад");
        assert_eq!(discord_stamps("no stamps here"), "no stamps here");
        assert_eq!(discord_stamps("bad <t:abc:t>"), "bad <t:abc:t>");
        assert!(!discord_stamps("x <t:1780383600:t> y").contains("<t:"));
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    #[test]
    fn times_in_both_clocks() {
        assert_eq!(hm_of(&at(2026, 10, 29, 21, 5), false), "21:05");
        assert_eq!(hm_of(&at(2026, 10, 29, 21, 5), true), "9:05 PM");
        assert_eq!(hm_of(&at(2026, 10, 29, 0, 30), true), "12:30 AM");
        assert_eq!(hm_of(&at(2026, 10, 29, 12, 0), true), "12:00 PM");
    }

    #[test]
    fn today_yesterday_and_older() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        assert_eq!(stamp_of(at(2026, 10, 29, 9, 7), today, false, false), "Сегодня, 09:07");
        assert_eq!(stamp_of(at(2026, 10, 28, 21, 52), today, true, false), "Вчера, 9:52 PM");
        assert_eq!(stamp_of(at(2026, 10, 1, 8, 0), today, false, false), "01.10.2026, 08:00");
        assert_eq!(stamp_of(at(2026, 10, 1, 8, 0), today, false, true), "2026-10-01, 08:00");
    }
}
