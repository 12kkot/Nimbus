//! Which channel categories are folded, kept in the settings file; and the way back and forward through chats.

use std::collections::HashSet;

use serde_json::{json, Value};

const KEY: &str = "folded_categories";
/// How far back the arrows reach.
const TRAIL_LEN: usize = 50;

pub fn load() -> HashSet<String> {
    parse(&crate::prefs::read()[KEY])
}

fn parse(v: &Value) -> HashSet<String> {
    v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect()
}

/// Flips a category and saves the choice. True when it is folded now.
pub fn toggle(id: &str) -> bool {
    let mut set = load();
    let folded = flip(&mut set, id);
    let mut list: Vec<&String> = set.iter().collect();
    list.sort();
    crate::prefs::write(KEY, json!(list));
    folded
}

fn flip(set: &mut HashSet<String>, id: &str) -> bool {
    if set.remove(id) {
        return false;
    }
    set.insert(id.to_string());
    true
}

/// For each row in display order, given its kind and id: does it sit in (or is it) a folded category?
/// 2 is a category header; rows before the first category are never folded.
pub fn marks<'a>(rows: impl Iterator<Item = (i32, &'a str)>, folded: &HashSet<String>) -> Vec<bool> {
    let mut now = false;
    rows.map(|(kind, id)| {
        if kind == 2 {
            now = folded.contains(id);
        }
        now
    })
    .collect()
}

/// A place the arrows can return to: server and channel.
pub type Place = (String, String);

#[derive(Debug, Default)]
pub struct Trail {
    back: Vec<Place>,
    ahead: Vec<Place>,
    at: Option<Place>,
}

impl Trail {
    /// A chat opened by hand: the way ahead is gone.
    pub fn visit(&mut self, place: Place) {
        if self.at.as_ref() == Some(&place) {
            return;
        }
        if let Some(old) = self.at.replace(place) {
            self.back.push(old);
            if self.back.len() > TRAIL_LEN {
                self.back.remove(0);
            }
        }
        self.ahead.clear();
    }

    pub fn go_back(&mut self) -> Option<Place> {
        let to = self.back.pop()?;
        if let Some(old) = self.at.replace(to.clone()) {
            self.ahead.push(old);
        }
        Some(to)
    }

    pub fn go_ahead(&mut self) -> Option<Place> {
        let to = self.ahead.pop()?;
        if let Some(old) = self.at.replace(to.clone()) {
            self.back.push(old);
        }
        Some(to)
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_ahead(&self) -> bool {
        !self.ahead.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(c: &str) -> Place {
        ("g".into(), c.into())
    }

    #[test]
    fn rows_follow_their_category() {
        let folded: HashSet<String> = ["b".to_string()].into();
        let rows = [(0, "x"), (2, "a"), (0, "y"), (2, "b"), (0, "z"), (1, "v"), (2, "c"), (0, "w")];
        let got = marks(rows.iter().copied(), &folded);
        assert_eq!(got, [false, false, false, true, true, true, false, false]);
    }

    #[test]
    fn flip_goes_both_ways() {
        let mut set = HashSet::new();
        assert!(flip(&mut set, "a"));
        assert!(!flip(&mut set, "a"));
        assert!(set.is_empty());
    }

    #[test]
    fn saved_list_is_read_back_and_junk_is_ignored() {
        let got = parse(&json!(["1", 2, "3"]));
        assert_eq!(got.len(), 2);
        assert!(parse(&Value::Null).is_empty());
    }

    #[test]
    fn trail_walks_back_and_ahead() {
        let mut t = Trail::default();
        t.visit(p("a"));
        assert!(!t.can_back());
        t.visit(p("b"));
        t.visit(p("b"));
        t.visit(p("c"));
        assert_eq!(t.go_back(), Some(p("b")));
        assert_eq!(t.go_back(), Some(p("a")));
        assert_eq!(t.go_back(), None);
        assert_eq!(t.go_ahead(), Some(p("b")));
        t.visit(p("d"));
        assert!(!t.can_ahead());
        assert_eq!(t.go_back(), Some(p("b")));
    }

    #[test]
    fn trail_forgets_the_oldest() {
        let mut t = Trail::default();
        for i in 0..TRAIL_LEN + 10 {
            t.visit(p(&i.to_string()));
        }
        assert_eq!(t.back.len(), TRAIL_LEN);
    }
}
