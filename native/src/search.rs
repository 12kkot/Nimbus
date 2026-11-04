//! "Find or start a conversation" (Ctrl+K): one list to search DMs, friends,
//! servers and their text channels by name, best matches first.

/// What a result opens.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A DM or group DM, by channel id.
    Dm(String),
    /// A friend without a DM yet, by user id.
    Friend(String),
    Guild(String),
    /// A text channel: (guild id, channel id).
    Channel(String, String),
}

impl Target {
    /// A stable name for remembering what was picked.
    pub fn key(&self) -> String {
        match self {
            Target::Dm(id) => format!("dm:{id}"),
            Target::Friend(id) => format!("friend:{id}"),
            Target::Guild(id) => format!("guild:{id}"),
            Target::Channel(g, c) => format!("channel:{g}:{c}"),
        }
    }

    /// `SearchItem.kind` in app.slint: 0 DM, 1 friend, 2 server, 3 channel.
    pub fn kind(&self) -> i32 {
        match self {
            Target::Dm(_) => 0,
            Target::Friend(_) => 1,
            Target::Guild(_) => 2,
            Target::Channel(..) => 3,
        }
    }
}

/// Something that can be found.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub target: Target,
    pub name: String,
    /// Shown dimmer after the name: a username, a server's name for a channel.
    pub detail: String,
    /// Other names to match (usernames).
    pub aliases: Vec<String>,
    /// Higher first when matches are equally good (recent DMs, open server).
    pub weight: u64,
}

/// How well `query` matches: 3 whole name, 2 start of the name, 1 start of
/// a word, 0 anywhere inside; None if it doesn't.
fn score(query: &str, name: &str) -> Option<u8> {
    let name = name.to_lowercase();
    if name == query {
        return Some(3);
    }
    if name.starts_with(query) {
        return Some(2);
    }
    if name.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(query)) {
        return Some(1);
    }
    name.contains(query).then_some(0)
}

/// The best matches for `query`, at most `limit`. An empty query lists recent DMs.
pub fn find(entries: &[Entry], query: &str, limit: usize) -> Vec<Entry> {
    find_recent(entries, query, limit, &[])
}

/// Like `find`; with an empty query, what was picked lately (newest first, `recents` are keys) comes first.
pub fn find_recent(entries: &[Entry], query: &str, limit: usize, recents: &[String]) -> Vec<Entry> {
    if query.trim().is_empty() && !recents.is_empty() {
        let mut out: Vec<Entry> = recents.iter().filter_map(|k| entries.iter().find(|e| &e.target.key() == k)).cloned().collect();
        for e in find(entries, "", limit) {
            if !out.contains(&e) {
                out.push(e);
            }
        }
        out.truncate(limit);
        return out;
    }
    let query = query.trim().trim_start_matches(['@', '#']).to_lowercase();
    let mut found: Vec<(u8, &Entry)> = entries
        .iter()
        .filter_map(|e| {
            if query.is_empty() {
                return matches!(e.target, Target::Dm(_)).then_some((0, e));
            }
            std::iter::once(&e.name).chain(&e.aliases).filter_map(|n| score(&query, n)).max().map(|s| (s, e))
        })
        .collect();
    // Better match first, then DMs and friends before servers and channels, then the weight.
    found.sort_by(|(sa, a), (sb, b)| {
        sb.cmp(sa).then_with(|| (a.target.kind() >= 2).cmp(&(b.target.kind() >= 2))).then_with(|| b.weight.cmp(&a.weight))
    });
    found.into_iter().take(limit).map(|(_, e)| e.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_picks_come_first() {
        let item = |target, name: &str, weight| Entry { target, name: name.into(), detail: String::new(), aliases: vec![], weight };
        let list = vec![
            item(Target::Dm("1".into()), "Аня", 9),
            item(Target::Guild("5".into()), "Сервер", 0),
            item(Target::Dm("2".into()), "Борис", 5),
        ];
        let names: Vec<String> = find_recent(&list, "", 10, &["guild:5".to_string(), "dm:2".to_string()]).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["Сервер", "Борис", "Аня"]);
        assert_eq!(find_recent(&list, "ан", 10, &["guild:5".to_string()]).len(), 1, "a query ignores recents");
    }

    fn entry(target: Target, name: &str, aliases: &[&str], weight: u64) -> Entry {
        Entry { target, name: name.into(), detail: String::new(), aliases: aliases.iter().map(|s| s.to_string()).collect(), weight }
    }

    fn entries() -> Vec<Entry> {
        vec![
            entry(Target::Dm("d1".into()), "demo_carol", &["demo_carol_dma"], 50),
            entry(Target::Dm("d2".into()), "demo_bob", &["goldie_rust"], 90),
            entry(Target::Friend("u3".into()), "Золотой", &["gold"], 0),
            entry(Target::Guild("g1".into()), "Тестовый сервер", &[], 0),
            entry(Target::Channel("g1".into(), "c1".into()), "main-chat", &[], 0),
            entry(Target::Channel("g1".into(), "c2".into()), "oh-shiting", &[], 0),
        ]
    }

    #[test]
    fn best_matches_first() {
        let names: Vec<String> = find(&entries(), "gold", 10).into_iter().map(|e| e.name).collect();
        // "gold" is a whole alias of the friend, the start of demo_bob.
        assert_eq!(names, ["Золотой", "demo_bob"]);
    }

    #[test]
    fn words_inside_names_and_channels() {
        let found = find(&entries(), "#chat", 10);
        assert_eq!(found[0].target, Target::Channel("g1".into(), "c1".into()));
        let found = find(&entries(), "сервер", 10);
        assert_eq!(found[0].target, Target::Guild("g1".into()));
    }

    #[test]
    fn empty_query_lists_recent_dms() {
        let names: Vec<String> = find(&entries(), "", 10).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["demo_bob", "demo_carol"]);
    }

    #[test]
    fn case_and_at_sign_do_not_matter() {
        assert_eq!(find(&entries(), "@DEMO_CAROL", 10)[0].target, Target::Dm("d1".into()));
        assert!(find(&entries(), "zzz", 10).is_empty());
    }
}
