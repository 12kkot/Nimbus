//! @ and # in the message box: finding the word being typed, matching names, and
//! turning the picked names into real mentions (`<@id>`, `<#id>`) when sending.

/// A thing the list offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// The shown name.
    pub label: String,
    pub detail: String,
    /// What goes into the box ("@Алекс").
    pub insert: String,
    /// What it becomes when sent ("<@123>").
    pub token: String,
}

/// The @ or # word at the end of the text: where it starts (in characters), the
/// sign, and what follows it. It must begin the text or follow a space.
pub fn token(text: &str) -> Option<(usize, char, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = chars.len();
    while i > 0 {
        let c = chars[i - 1];
        if c == '@' || c == '#' {
            let before_ok = i == 1 || chars[i - 2].is_whitespace();
            let query: String = chars[i..].iter().collect();
            return (before_ok && query.chars().count() <= 24 && !query.contains('\n')).then(|| (i - 1, c, query));
        }
        // A space ends the word: the sign isn't being typed any more.
        if c.is_whitespace() {
            return None;
        }
        i -= 1;
    }
    None
}

/// Whether the (lowercase) query starts any word of any of the names.
pub fn matches(query_lower: &str, names: &[&str]) -> bool {
    query_lower.is_empty()
        || names.iter().any(|n| {
            let n = n.to_lowercase();
            n.starts_with(query_lower)
                || n.split(|c: char| !c.is_alphanumeric())
                    .any(|w| !w.is_empty() && w.starts_with(query_lower))
        })
}

/// The text with picked names replaced by their mentions. Longer names go first,
/// so "@Алекс Иванов" isn't cut short by "@Алекс".
pub fn resolve(text: &str, picked: &[(String, String)]) -> String {
    let mut ordered: Vec<&(String, String)> = picked.iter().collect();
    ordered.sort_by_key(|(insert, _)| std::cmp::Reverse(insert.chars().count()));
    let mut out = text.to_string();
    for (insert, token) in ordered {
        out = out.replace(insert.as_str(), token);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_word_being_typed() {
        assert_eq!(token("привет @ма"), Some((7, '@', "ма".into())));
        assert_eq!(token("@"), Some((0, '@', String::new())));
        assert_eq!(token("зайди в #общ"), Some((8, '#', "общ".into())));
        assert_eq!(token("почта a@b"), None, "a sign inside a word");
        assert_eq!(token("@Алекс привет"), None, "the word ended");
        assert_eq!(token("просто текст"), None);
        assert_eq!(token("@aaaaaaaaaaaaaaaaaaaaaaaaaaaa"), None, "too long to be a name");
    }

    #[test]
    fn names_match_by_the_start_of_a_word() {
        assert!(matches("ал", &["Алекс Иванов"]));
        assert!(matches("ив", &["Алекс Иванов"]));
        assert!(matches("gold", &["demo_bob", "goldie_x"]));
        assert!(matches("", &["любой"]));
        assert!(!matches("ек", &["Алекс"]), "the middle of a word does not count");
    }

    #[test]
    fn picked_names_become_mentions() {
        let picked = vec![
            ("@Алекс".to_string(), "<@1>".to_string()),
            ("@Алекс Иванов".to_string(), "<@2>".to_string()),
            ("#общий".to_string(), "<#9>".to_string()),
        ];
        assert_eq!(resolve("эй @Алекс Иванов и @Алекс в #общий", &picked), "эй <@2> и <@1> в <#9>");
        assert_eq!(resolve("без упоминаний", &picked), "без упоминаний");
    }
}
