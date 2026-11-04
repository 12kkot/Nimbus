//! What changed, built into the app: shown once after an update, and any time from the settings.

const TEXT: &str = include_str!("../CHANGELOG.md");
/// The version that last ran here.
const SEEN_KEY: &str = "last_version";

/// A line of the changelog as shown.
#[derive(Debug, PartialEq)]
pub enum Line {
    Version(String),
    Section(String),
    Item(String),
    Plain(String),
}

/// The changelog's markdown, in lines to draw: only what it really uses of it.
pub fn parse(text: &str) -> Vec<Line> {
    text.lines()
        .map(|line| line.trim().replace('`', ""))
        .filter(|line| !line.is_empty())
        .map(|line| {
            if let Some(version) = line.strip_prefix("## ") {
                Line::Version(version.to_string())
            } else if let Some(section) = line.strip_prefix("### ") {
                Line::Section(section.to_string())
            } else if let Some(item) = line.strip_prefix("- ") {
                Line::Item(item.to_string())
            } else {
                Line::Plain(line)
            }
        })
        .collect()
}

pub fn lines() -> Vec<Line> {
    parse(TEXT)
}

/// Whether to tell about an update: another version ran here before. A new install has nothing saved at all,
/// and a version from before this was kept has other settings saved.
fn updated(seen: Option<&str>, has_settings: bool, version: &str) -> bool {
    match seen {
        Some(seen) => seen != version,
        None => has_settings,
    }
}

/// Remembers this version; true when it is new here.
pub fn note_version(version: &str) -> bool {
    let saved = crate::prefs::read();
    let has_settings = saved.as_object().is_some_and(|all| !all.is_empty());
    let is_new = updated(saved[SEEN_KEY].as_str(), has_settings, version);
    if saved[SEEN_KEY].as_str() != Some(version) {
        crate::prefs::write(SEEN_KEY, serde_json::json!(version));
    }
    is_new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_changelog_is_read_line_by_line() {
        let text = "## Nimbus 1.0\n\nHello `there`.\n\n### Speed\n- Faster `chats`.\n";
        assert_eq!(
            parse(text),
            vec![Line::Version("Nimbus 1.0".into()), Line::Plain("Hello there.".into()), Line::Section("Speed".into()), Line::Item("Faster chats.".into())]
        );
    }

    #[test]
    fn the_built_in_changelog_starts_with_this_version() {
        let first = lines().into_iter().next();
        assert_eq!(first, Some(Line::Version(format!("Nimbus {}", env!("CARGO_PKG_VERSION")))));
    }

    #[test]
    fn only_an_update_is_told_about() {
        assert!(updated(Some("0.1.9"), true, "0.2.0"));
        assert!(!updated(Some("0.2.0"), true, "0.2.0"));
        assert!(updated(None, true, "0.2.0"), "came from a version that kept no such note");
        assert!(!updated(None, false, "0.2.0"), "a new install");
    }
}
