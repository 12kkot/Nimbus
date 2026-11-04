//! Our private notes about other people (only we see them), as Discord keeps them.

use serde_json::{json, Value};

use crate::discord::snowflake;

/// The longest note Discord keeps.
pub const MAX_CHARS: usize = 256;

pub fn path(user: &str) -> Result<String, String> {
    Ok(format!("/users/@me/notes/{}", snowflake(user)?))
}

/// The note in what Discord answers (none yet: empty).
pub fn of(answer: &Value) -> String {
    answer["note"].as_str().unwrap_or("").to_string()
}

/// What to save when the field is left: the typed text, trimmed and cut to the limit, or none when
/// it says what was already saved.
pub fn to_save(saved: &str, typed: &str) -> Option<String> {
    let text: String = typed.trim().chars().take(MAX_CHARS).collect();
    (text != saved.trim()).then_some(text)
}

pub fn body(text: &str) -> Value {
    json!({ "note": text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_changed_note_is_saved_and_never_too_long() {
        assert_eq!(to_save("друг с работы", "друг с работы "), None);
        assert_eq!(to_save("", "  "), None);
        assert_eq!(to_save("старое", "новое"), Some("новое".into()));
        assert_eq!(to_save("старое", ""), Some(String::new()), "clearing is a change");
        let long = to_save("", &"я".repeat(300)).unwrap();
        assert_eq!(long.chars().count(), MAX_CHARS);
        assert_eq!(body("x"), json!({ "note": "x" }));
    }

    #[test]
    fn note_paths_check_the_id() {
        assert_eq!(path("123").unwrap(), "/users/@me/notes/123");
        assert!(path("12/../3").is_err());
        assert!(path("@me").is_err());
        assert_eq!(of(&json!({ "note": "привет", "note_user_id": "1" })), "привет");
        assert_eq!(of(&Value::Null), "");
    }
}
