//! Polls under messages: what they ask, how the votes stand, and what our click sends.

use serde_json::{json, Value};

use crate::chat;

/// One answer of a poll.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub id: u64,
    pub text: String,
    /// The emoji before the text: the emoji itself, or `:name:` for a custom one.
    pub emoji: String,
    /// A custom emoji's key (`name:id`) and picture.
    pub emoji_key: String,
    pub emoji_image: Option<String>,
    pub count: u64,
    /// We voted for it.
    pub me: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Poll {
    pub question: String,
    pub answers: Vec<Answer>,
    /// More than one answer may be chosen.
    pub multi: bool,
    /// Discord sent the counts (it leaves them out while it hasn't counted).
    pub has_results: bool,
    /// The votes are final: the poll is over.
    pub finalized: bool,
    /// When the poll ends, in ms since 1970.
    pub expiry_ms: Option<i64>,
}

/// The poll of a message, if it has one.
pub fn parse(m: &Value) -> Option<Poll> {
    let p = m["poll"].as_object()?;
    let results = &p.get("results").cloned().unwrap_or(Value::Null);
    let counts = results["answer_counts"].as_array();
    let answers = p.get("answers")?.as_array()?.iter().filter_map(|a| {
        let id = a["answer_id"].as_u64()?;
        let media = &a["poll_media"];
        let emoji = &media["emoji"];
        let custom = emoji["id"].as_str().filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()));
        let name = emoji["name"].as_str().unwrap_or("");
        let (glyph, key, image) = match custom {
            Some(eid) => (format!(":{name}:"), format!("{name}:{eid}"), Some(chat::emoji_picture(eid, emoji["animated"].as_bool().unwrap_or(false)))),
            None => (name.to_string(), String::new(), None),
        };
        let count = counts.and_then(|c| c.iter().find(|c| c["id"].as_u64() == Some(id)));
        Some(Answer {
            id,
            text: media["text"].as_str().unwrap_or("").to_string(),
            emoji: glyph,
            emoji_key: key,
            emoji_image: image,
            count: count.and_then(|c| c["count"].as_u64()).unwrap_or(0),
            me: count.and_then(|c| c["me_voted"].as_bool()).unwrap_or(false),
        })
    });
    let expiry_ms = p.get("expiry").and_then(|e| e.as_str()).and_then(|e| chrono::DateTime::parse_from_rfc3339(e).ok()).map(|t| t.timestamp_millis());
    Some(Poll {
        question: p.get("question").and_then(|q| q["text"].as_str()).unwrap_or("").to_string(),
        answers: answers.collect(),
        multi: p.get("allow_multiselect").and_then(Value::as_bool).unwrap_or(false),
        has_results: results.is_object(),
        finalized: results["is_finalized"].as_bool().unwrap_or(false),
        expiry_ms,
    })
}

impl Poll {
    /// No more votes: counted as final, or past its end.
    pub fn ended(&self, now_ms: i64) -> bool {
        self.finalized || self.expiry_ms.is_some_and(|e| e <= now_ms)
    }

    pub fn total(&self) -> u64 {
        self.answers.iter().map(|a| a.count).sum()
    }

    /// The answers we voted for.
    pub fn mine(&self) -> Vec<u64> {
        self.answers.iter().filter(|a| a.me).map(|a| a.id).collect()
    }

    /// What we vote for after clicking `answer`: with several allowed it's switched on or off; with one,
    /// it replaces our vote, and clicking our only vote again takes it back (an empty list).
    pub fn choice(&self, answer: u64) -> Vec<u64> {
        let mut mine = self.mine();
        if self.multi {
            match mine.iter().position(|&a| a == answer) {
                Some(i) => {
                    mine.remove(i);
                }
                None => mine.push(answer),
            }
            mine.sort_unstable();
            mine
        } else if mine == [answer] {
            Vec::new()
        } else {
            vec![answer]
        }
    }

    /// Our vote as Discord took it: shown at once, before the gateway tells us.
    pub fn set_mine(&mut self, ids: &[u64]) {
        for a in &mut self.answers {
            let now = ids.contains(&a.id);
            match (a.me, now) {
                (false, true) => a.count += 1,
                (true, false) => a.count = a.count.saturating_sub(1),
                _ => {}
            }
            a.me = now;
        }
        self.has_results = true;
    }

    /// MESSAGE_POLL_VOTE_ADD / _REMOVE. Our own votes were already counted by `set_mine`.
    /// Returns whether anything changed.
    pub fn apply_vote(&mut self, kind: &str, event: &Value, me: &str) -> bool {
        let Some(answer) = event["answer_id"].as_u64() else { return false };
        let mine = event["user_id"].as_str() == Some(me);
        let Some(a) = self.answers.iter_mut().find(|a| a.id == answer) else { return false };
        match kind {
            "MESSAGE_POLL_VOTE_ADD" if !(mine && a.me) => {
                a.count += 1;
                a.me |= mine;
            }
            "MESSAGE_POLL_VOTE_REMOVE" if !(mine && !a.me) => {
                a.count = a.count.saturating_sub(1);
                if mine {
                    a.me = false;
                }
            }
            _ => return false,
        }
        self.has_results = true;
        true
    }
}

/// The body of `PUT /channels/{c}/polls/{m}/answers/@me`: the answers chosen (none takes the vote back).
pub fn vote_body(ids: &[u64]) -> Value {
    json!({ "answer_ids": ids.iter().map(|id| id.to_string()).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(multi: bool) -> Value {
        json!({ "poll": {
            "question": { "text": "Куда идём?" },
            "answers": [
                { "answer_id": 1, "poll_media": { "text": "Раст", "emoji": { "id": null, "name": "🦀" } } },
                { "answer_id": 2, "poll_media": { "text": "Кино", "emoji": { "id": "55", "name": "popcorn", "animated": false } } },
                { "answer_id": 3, "poll_media": { "text": "Спать" } },
            ],
            "expiry": "2026-11-02T18:00:00+00:00",
            "allow_multiselect": multi,
            "results": { "is_finalized": false, "answer_counts": [
                { "id": 1, "count": 3, "me_voted": true },
                { "id": 2, "count": 1, "me_voted": false },
            ] },
        } })
    }

    #[test]
    fn a_poll_is_read_with_its_counts() {
        let p = parse(&sample(false)).unwrap();
        assert_eq!(p.question, "Куда идём?");
        assert_eq!(p.answers.len(), 3);
        assert_eq!((p.answers[0].emoji.as_str(), p.answers[0].count, p.answers[0].me), ("🦀", 3, true));
        assert_eq!(p.answers[1].emoji_key, "popcorn:55");
        assert_eq!(p.answers[1].emoji_image.as_deref(), Some("https://cdn.discordapp.com/emojis/55.webp?size=48"));
        assert_eq!((p.answers[2].count, p.answers[2].emoji.as_str()), (0, ""));
        assert_eq!(p.total(), 4);
        assert!(p.has_results && !p.finalized);
        let end = chrono::DateTime::parse_from_rfc3339("2026-11-02T18:00:00+00:00").unwrap().timestamp_millis();
        assert!(!p.ended(end - 1) && p.ended(end));
        assert!(parse(&json!({ "content": "no poll" })).is_none());
        let unseen = parse(&json!({ "poll": { "question": { "text": "?" }, "answers": [] } })).unwrap();
        assert!(!unseen.has_results && !unseen.ended(0));
    }

    #[test]
    fn one_answer_polls_switch_or_take_back() {
        let p = parse(&sample(false)).unwrap();
        assert_eq!(p.choice(2), vec![2], "another answer replaces ours");
        assert_eq!(p.choice(1), Vec::<u64>::new(), "our only vote clicked again goes");
    }

    #[test]
    fn several_answer_polls_toggle_each() {
        let mut p = parse(&sample(true)).unwrap();
        assert_eq!(p.choice(3), vec![1, 3]);
        assert_eq!(p.choice(1), Vec::<u64>::new());
        p.set_mine(&[1, 3]);
        assert_eq!(p.choice(1), vec![3]);
        assert_eq!((p.answers[2].count, p.answers[2].me), (1, true));
    }

    #[test]
    fn our_vote_is_not_counted_twice() {
        let mut p = parse(&sample(false)).unwrap();
        p.set_mine(&[2]);
        assert_eq!((p.answers[0].count, p.answers[1].count), (2, 2));
        // Then the gateway tells us about our own vote.
        assert!(!p.apply_vote("MESSAGE_POLL_VOTE_REMOVE", &json!({ "user_id": "me", "answer_id": 1 }), "me"));
        assert!(!p.apply_vote("MESSAGE_POLL_VOTE_ADD", &json!({ "user_id": "me", "answer_id": 2 }), "me"));
        assert_eq!((p.answers[0].count, p.answers[1].count), (2, 2));
        // Someone else's vote counts.
        assert!(p.apply_vote("MESSAGE_POLL_VOTE_ADD", &json!({ "user_id": "9", "answer_id": 3 }), "me"));
        assert!(p.apply_vote("MESSAGE_POLL_VOTE_REMOVE", &json!({ "user_id": "9", "answer_id": 1 }), "me"));
        assert_eq!((p.answers[0].count, p.answers[2].count), (1, 1));
        assert!(!p.apply_vote("MESSAGE_POLL_VOTE_ADD", &json!({ "user_id": "9", "answer_id": 77 }), "me"));
    }

    #[test]
    fn the_vote_body_names_the_answers() {
        assert_eq!(vote_body(&[2]), json!({ "answer_ids": ["2"] }));
        assert_eq!(vote_body(&[1, 3]), json!({ "answer_ids": ["1", "3"] }));
        assert_eq!(vote_body(&[]), json!({ "answer_ids": [] }));
    }
}
