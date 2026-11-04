//! Slash commands: the bots' commands Discord lists for a chat, the panel that offers them when the
//! message box starts with `/`, and turning `/name arguments` into the interaction that runs one.

use serde_json::{json, Value};

/// How many commands the "recent" section keeps.
pub const RECENT: usize = 5;

/// A bot (Discord calls it an application) whose commands the chat has.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub id: String,
    pub name: String,
    /// The icon's address on Discord's CDN, if it has one.
    pub icon: Option<String>,
}

/// One option of a command (what follows its name).
#[derive(Debug, Clone, PartialEq)]
pub struct Opt {
    /// 3 string, 4 integer, 5 boolean, 6 user, 7 channel, 8 role, 9 mentionable, 10 number, 11 attachment.
    pub kind: u64,
    pub name: String,
    pub description: String,
    pub required: bool,
    /// The values it may take, if limited: (shown name, value).
    pub choices: Vec<(String, Value)>,
}

/// What the panel offers: a command, or one of its subcommands ("music play").
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub app: String,
    /// The command as Discord gave it (sent back with the interaction).
    pub command: Value,
    /// The subcommand group and subcommand under it, if any.
    pub path: Vec<String>,
    /// The whole name typed after `/`, with spaces between the parts.
    pub name: String,
    pub description: String,
    pub options: Vec<Opt>,
}

impl Entry {
    /// Stays the same between fetches: for the recent section.
    pub fn key(&self) -> String {
        format!("{}:{}", self.command["id"].as_str().unwrap_or(""), self.path.join(" "))
    }

    /// The options as the panel's pills show them: required ones first.
    pub fn pills(&self) -> Vec<String> {
        let mut options: Vec<&Opt> = self.options.iter().collect();
        options.sort_by_key(|o| !o.required);
        options.into_iter().map(|o| o.name.clone()).collect()
    }
}

/// The commands of a chat.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Index {
    pub apps: Vec<App>,
    pub entries: Vec<Entry>,
    /// Commands done on a person (type 2), each with its app's name: the "Приложения" of their menu.
    pub on_user: Vec<(String, Value)>,
}

impl Index {
    pub fn app(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.id == id)
    }
}

/// Where a chat's commands are asked for: the server's, or the DM's.
pub fn index_path(guild: Option<&str>, channel: &str) -> Result<String, String> {
    Ok(match guild {
        Some(g) => format!("/guilds/{}/application-command-index", crate::snowflake(g)?),
        None => format!("/channels/{}/application-command-index", crate::snowflake(channel)?),
    })
}

fn icon_url(app_id: &str, icon: Option<&str>) -> Option<String> {
    let icon = icon.filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))?;
    app_id.chars().all(|c| c.is_ascii_digit()).then(|| format!("https://cdn.discordapp.com/app-icons/{app_id}/{icon}.png?size=64"))
}

fn opt_of(v: &Value) -> Option<Opt> {
    let choices = v["choices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((c["name"].as_str()?.to_string(), c["value"].clone())))
        .filter(|(_, value)| !value.is_null())
        .collect();
    Some(Opt {
        kind: v["type"].as_u64()?,
        name: v["name"].as_str()?.to_string(),
        description: v["description"].as_str().unwrap_or("").to_string(),
        required: v["required"].as_bool().unwrap_or(false),
        choices,
    })
}

/// A command, or one entry per subcommand when it has them (type 1 subcommand, 2 group of them).
fn entries_of(app: &str, command: &Value, out: &mut Vec<Entry>) {
    let Some(name) = command["name"].as_str() else { return };
    let options = command["options"].as_array().cloned().unwrap_or_default();
    let entry = |path: Vec<String>, description: &str, options: &[Value]| Entry {
        app: app.to_string(),
        command: command.clone(),
        name: std::iter::once(name.to_string()).chain(path.iter().cloned()).collect::<Vec<_>>().join(" "),
        path,
        description: description.to_string(),
        options: options.iter().filter_map(opt_of).collect(),
    };
    let subs: Vec<&Value> = options.iter().filter(|o| matches!(o["type"].as_u64(), Some(1 | 2))).collect();
    if subs.is_empty() {
        out.push(entry(Vec::new(), command["description"].as_str().unwrap_or(""), &options));
        return;
    }
    for sub in subs {
        let Some(sub_name) = sub["name"].as_str() else { continue };
        let inner = sub["options"].as_array().cloned().unwrap_or_default();
        if sub["type"].as_u64() == Some(2) {
            for leaf in inner.iter().filter(|o| o["type"].as_u64() == Some(1)) {
                let Some(leaf_name) = leaf["name"].as_str() else { continue };
                let leaf_options = leaf["options"].as_array().cloned().unwrap_or_default();
                out.push(entry(vec![sub_name.to_string(), leaf_name.to_string()], leaf["description"].as_str().unwrap_or(""), &leaf_options));
            }
        } else {
            out.push(entry(vec![sub_name.to_string()], sub["description"].as_str().unwrap_or(""), &inner));
        }
    }
}

/// What `application-command-index` brings. Only chat commands (type 1) are kept, and only those of a
/// listed bot; each bot's come sorted by name.
pub fn parse_index(v: &Value) -> Index {
    let apps: Vec<App> = v["applications"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let id = a["id"].as_str()?.to_string();
            let name = a["name"].as_str().or(a["bot"]["username"].as_str()).unwrap_or("").to_string();
            let icon = icon_url(&id, a["icon"].as_str());
            Some(App { id, name, icon })
        })
        .collect();
    let mut entries = Vec::new();
    for command in v["application_commands"].as_array().into_iter().flatten() {
        let chat = command["type"].as_u64().unwrap_or(1) == 1;
        let Some(app) = command["application_id"].as_str().filter(|a| chat && apps.iter().any(|x| x.id == *a)) else { continue };
        entries_of(app, command, &mut entries);
    }
    let order = |e: &Entry| apps.iter().position(|a| a.id == e.app).unwrap_or(usize::MAX);
    entries.sort_by(|a, b| order(a).cmp(&order(b)).then_with(|| a.name.cmp(&b.name)));
    let on_user = v["application_commands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["type"].as_u64() == Some(2))
        .filter_map(|c| Some((apps.iter().find(|a| Some(a.id.as_str()) == c["application_id"].as_str())?.name.clone(), c.clone())))
        .collect();
    Index { apps, entries, on_user }
}

/// What's typed after `/` while the command's name is still being typed (nothing else yet).
pub fn query(text: &str) -> Option<String> {
    let rest = text.strip_prefix('/')?;
    (!rest.contains(char::is_whitespace) && rest.chars().count() <= 32).then(|| rest.to_lowercase())
}

/// Whether the command matches what's typed: the whole name, or any of its parts, starts with it.
fn matches(entry: &Entry, query: &str) -> bool {
    let name = entry.name.to_lowercase();
    query.is_empty() || name.starts_with(query) || name.split(' ').any(|part| part.starts_with(query))
}

/// A line of the panel: a section's header (a bot's, or the recent section's: None), or a command.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Header(Option<String>),
    Command(usize),
}

/// The panel for what's typed: the recent commands that match, then every bot's that match.
pub fn rows(index: &Index, query: &str, recent: &[String]) -> Vec<Row> {
    let mut out = Vec::new();
    let recent_rows: Vec<usize> = recent.iter().filter_map(|key| index.entries.iter().position(|e| e.key() == *key)).filter(|&i| matches(&index.entries[i], query)).collect();
    if !recent_rows.is_empty() {
        out.push(Row::Header(None));
        out.extend(recent_rows.into_iter().map(Row::Command));
    }
    for app in &index.apps {
        let found: Vec<usize> = (0..index.entries.len()).filter(|&i| index.entries[i].app == app.id && matches(&index.entries[i], query)).collect();
        if !found.is_empty() {
            out.push(Row::Header(Some(app.id.clone())));
            out.extend(found.into_iter().map(Row::Command));
        }
    }
    out
}

/// The command the text starts with (`/name`, then a space or the end), with the text after it.
/// The longest name wins, so "/music play x" is the subcommand, not "/music".
pub fn find<'a>(index: &'a Index, text: &'a str) -> Option<(&'a Entry, &'a str)> {
    let body = text.strip_prefix('/')?;
    let words: Vec<&str> = body.split_whitespace().collect();
    index
        .entries
        .iter()
        .filter(|e| {
            let parts: Vec<&str> = e.name.split(' ').collect();
            words.len() >= parts.len() && parts.iter().zip(&words).all(|(p, w)| p.to_lowercase() == w.to_lowercase())
        })
        .max_by_key(|e| e.path.len())
        .map(|e| (e, skip_words(body, e.path.len() + 1)))
}

/// The text after its first `n` words.
fn skip_words(text: &str, n: usize) -> &str {
    let mut rest = text;
    for _ in 0..n {
        rest = rest.trim_start();
        rest = &rest[rest.find(char::is_whitespace).unwrap_or(rest.len())..];
    }
    rest.trim_start()
}

/// Why the typed arguments can't run the command.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgError {
    /// A required option was not given.
    Missing(String),
    /// The value doesn't fit the option (not a number, not one of its choices, …).
    Bad(String),
    /// Files can't be given by typing.
    Unsupported(String),
    /// Text left over after every option got its value.
    Extra(String),
}

/// The words for the status line.
pub fn error_text(e: &ArgError) -> String {
    match e {
        ArgError::Missing(name) => crate::i18n::tf("Не хватает параметра «{}»", &[name]),
        ArgError::Bad(name) => crate::i18n::tf("Неверное значение параметра «{}»", &[name]),
        ArgError::Unsupported(name) => crate::i18n::tf("Параметр «{}» здесь не задать", &[name]),
        ArgError::Extra(text) => crate::i18n::tf("Лишний текст: {}", &[text]),
    }
}

/// Where each `name:` of the command's options starts in the text: (byte offset, option index), in order.
fn named_marks(text: &str, options: &[Opt]) -> Vec<(usize, usize)> {
    let mut marks = Vec::new();
    for (at, _) in text.char_indices() {
        if at > 0 && !text[..at].ends_with(char::is_whitespace) {
            continue;
        }
        let rest = &text[at..];
        let best = options
            .iter()
            .enumerate()
            .filter(|(_, o)| rest.len() > o.name.len() && rest.is_char_boundary(o.name.len()) && rest[..o.name.len()].to_lowercase() == o.name.to_lowercase() && rest[o.name.len()..].starts_with(':'))
            .max_by_key(|(_, o)| o.name.len());
        if let Some((i, _)) = best {
            marks.push((at, i));
        }
    }
    marks
}

fn unquote(text: &str) -> &str {
    let t = text.trim();
    t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(t)
}

/// Values for options in order from plain text. A string option takes the rest of the line when every
/// option still unfilled after it is optional (those can still be given as `name:value`); others take
/// one word (or one "quoted text").
fn positional(text: &str, options: &[Opt], filled: &mut [Option<String>]) -> Result<(), ArgError> {
    let mut rest = text.trim();
    let open: Vec<usize> = (0..options.len()).filter(|&i| filled[i].is_none()).collect();
    for (n, &i) in open.iter().enumerate() {
        if rest.is_empty() {
            break;
        }
        let takes_rest = options[i].kind == 3 && open[n + 1..].iter().all(|&j| !options[j].required);
        let (value, after) = if takes_rest {
            (unquote(rest), "")
        } else if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            (&quoted[..end], quoted.get(end + 1..).unwrap_or(""))
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            (&rest[..end], &rest[end..])
        };
        filled[i] = Some(value.to_string());
        rest = after.trim();
    }
    if rest.is_empty() {
        Ok(())
    } else {
        Err(ArgError::Extra(rest.to_string()))
    }
}

/// An id from a mention (`<@1>`, `<@!1>`, `<@&1>`, `<#1>`, by `kinds` of sign) or a bare number.
fn mention_id(text: &str, signs: &[&str]) -> Option<String> {
    let id = signs
        .iter()
        .find_map(|sign| text.strip_prefix('<').and_then(|t| t.strip_prefix(sign)).and_then(|t| t.strip_suffix('>')))
        .unwrap_or(text);
    (!id.is_empty() && id.len() <= 20 && id.chars().all(|c| c.is_ascii_digit())).then(|| id.to_string())
}

/// A typed value as the option's type wants it.
fn convert(option: &Opt, text: &str) -> Result<Value, ArgError> {
    let bad = || ArgError::Bad(option.name.clone());
    if !option.choices.is_empty() {
        let typed = text.to_lowercase();
        return option
            .choices
            .iter()
            .find(|(name, value)| name.to_lowercase() == typed || value.as_str().map_or_else(|| value.to_string() == text, |v| v.to_lowercase() == typed))
            .map(|(_, value)| value.clone())
            .ok_or_else(bad);
    }
    match option.kind {
        3 => Ok(Value::String(text.to_string())),
        4 => text.parse::<i64>().map(Value::from).map_err(|_| bad()),
        10 => text.replace(',', ".").parse::<f64>().ok().filter(|f| f.is_finite()).map(Value::from).ok_or_else(bad),
        5 => match text.to_lowercase().as_str() {
            "true" | "yes" | "1" | "on" => Ok(Value::Bool(true)),
            "false" | "no" | "0" | "off" => Ok(Value::Bool(false)),
            _ => Err(bad()),
        },
        6 => mention_id(text, &["@!", "@"]).map(Value::String).ok_or_else(bad),
        7 => mention_id(text, &["#"]).map(Value::String).ok_or_else(bad),
        8 => mention_id(text, &["@&"]).map(Value::String).ok_or_else(bad),
        9 => mention_id(text, &["@&", "@!", "@"]).map(Value::String).ok_or_else(bad),
        _ => Err(ArgError::Unsupported(option.name.clone())),
    }
}

/// The typed text after the command's name as its options: `name:value` pairs, and plain values for
/// the rest in order. Every required one must be there.
pub fn arguments(entry: &Entry, text: &str) -> Result<Vec<Value>, ArgError> {
    let options = &entry.options;
    let mut filled: Vec<Option<String>> = vec![None; options.len()];
    let marks = named_marks(text, options);
    let plain_end = marks.first().map_or(text.len(), |m| m.0);
    for (k, &(at, i)) in marks.iter().enumerate() {
        let end = marks.get(k + 1).map_or(text.len(), |m| m.0);
        filled[i] = Some(unquote(&text[at + options[i].name.len() + 1..end]).to_string());
    }
    positional(&text[..plain_end], options, &mut filled)?;
    let mut out = Vec::new();
    for (option, value) in options.iter().zip(filled) {
        match value.filter(|v| !v.is_empty()) {
            Some(v) => out.push(json!({ "type": option.kind, "name": option.name, "value": convert(option, &v)? })),
            None if option.kind == 11 && option.required => return Err(ArgError::Unsupported(option.name.clone())),
            None if option.required => return Err(ArgError::Missing(option.name.clone())),
            None => {}
        }
    }
    Ok(out)
}

/// The options inside the subcommand (and its group) they belong to.
pub fn wrap(entry: &Entry, options: Vec<Value>) -> Vec<Value> {
    match entry.path.as_slice() {
        [sub] => vec![json!({ "type": 1, "name": sub, "options": options })],
        [group, sub] => vec![json!({ "type": 2, "name": group, "options": [{ "type": 1, "name": sub, "options": options }] })],
        _ => options,
    }
}

/// A fresh id in Discord's form (milliseconds since 2015 in the high bits), for the interaction's nonce.
pub fn nonce(unix_ms: u64) -> String {
    (unix_ms.saturating_sub(1_420_070_400_000) << 22).to_string()
}

/// The body of `POST /interactions` that runs the command (with `options` already wrapped).
pub fn interaction(entry: &Entry, options: Vec<Value>, guild: Option<&str>, channel: &str, session: &str, nonce: &str) -> Value {
    let command = &entry.command;
    let mut body = json!({
        "type": 2,
        "application_id": entry.app,
        "channel_id": channel,
        "session_id": session,
        "nonce": nonce,
        "data": {
            "version": command["version"],
            "id": command["id"],
            "name": command["name"],
            "type": 1,
            "options": options,
            "application_command": command,
            "attachments": [],
        },
    });
    if let Some(g) = guild {
        body["guild_id"] = json!(g);
    }
    body
}

/// The body of `POST /interactions` that runs a command on a person.
pub fn user_interaction(command: &Value, target: &str, guild: Option<&str>, channel: &str, session: &str, nonce: &str) -> Value {
    let mut body = json!({
        "type": 2,
        "application_id": command["application_id"],
        "channel_id": channel,
        "session_id": session,
        "nonce": nonce,
        "data": {
            "version": command["version"],
            "id": command["id"],
            "name": command["name"],
            "type": 2,
            "target_id": target,
            "options": [],
            "application_command": command,
            "attachments": [],
        },
    });
    if let Some(g) = guild {
        body["guild_id"] = json!(g);
    }
    body
}

/// The hint above the box once a command is picked: its options (required first), and which of them is
/// being typed now (the last `name:` typed, else the one the words typed so far reach, as `positional`
/// hands them out), if any.
pub fn hint(entry: &Entry, text: &str) -> (Vec<String>, i32) {
    let pills = entry.pills();
    let options = &entry.options;
    let active = if let Some(&(_, i)) = named_marks(text, options).last() {
        Some(options[i].name.clone())
    } else {
        let mut rest = text.trim_start();
        let mut typing = None;
        for (n, o) in options.iter().enumerate() {
            let takes_rest = o.kind == 3 && options[n + 1..].iter().all(|o| !o.required);
            match rest.find(char::is_whitespace) {
                Some(end) if !takes_rest => rest = rest[end..].trim_start(),
                _ => {
                    typing = Some(o.name.clone());
                    break;
                }
            }
        }
        typing
    };
    let active = active.and_then(|name| pills.iter().position(|p| *p == name)).map_or(-1, |i| i as i32);
    (pills, active)
}

/// The recent list with `key` used just now: it goes first, the oldest drops off.
pub fn remember(recent: &mut Vec<String>, key: String) {
    recent.retain(|k| *k != key);
    recent.insert(0, key);
    recent.truncate(RECENT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_on_a_person_are_kept_apart_and_name_their_target() {
        let v = json!({
            "applications": [{ "id": "1", "name": "Бот" }],
            "application_commands": [
                { "id": "10", "application_id": "1", "version": "5", "type": 1, "name": "play" },
                { "id": "11", "application_id": "1", "version": "6", "type": 2, "name": "Профиль" },
                { "id": "12", "application_id": "9", "version": "7", "type": 2, "name": "Чужой" },
            ],
        });
        let index = parse_index(&v);
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.on_user.len(), 1, "a command of an unlisted app stays out");
        assert_eq!(index.on_user[0].0, "Бот");
        let body = user_interaction(&index.on_user[0].1, "77", Some("5"), "6", "s", "n");
        assert_eq!(body["data"]["type"], 2);
        assert_eq!(body["data"]["target_id"], "77");
        assert_eq!(body["application_id"], "1");
        assert_eq!(body["guild_id"], "5");
    }

    fn sample() -> Value {
        json!({
            "applications": [
                { "id": "100", "name": "Jockie Music", "icon": "abc123" },
                { "id": "200", "name": "Модератор", "icon": null },
            ],
            "application_commands": [
                { "id": "1", "application_id": "100", "version": "11", "type": 1, "name": "play", "description": "Играть трек",
                  "options": [
                      { "type": 3, "name": "query", "description": "ссылка", "required": true },
                      { "type": 4, "name": "volume", "description": "громкость" },
                  ] },
                { "id": "2", "application_id": "100", "version": "12", "type": 1, "name": "loop", "description": "Повтор",
                  "options": [{ "type": 3, "name": "mode", "required": true, "choices": [{ "name": "Трек", "value": "track" }, { "name": "Off", "value": "off" }] }] },
                { "id": "3", "application_id": "200", "version": "13", "type": 1, "name": "ban", "description": "Забанить",
                  "options": [
                      { "type": 6, "name": "user", "required": true },
                      { "type": 5, "name": "silent" },
                      { "type": 3, "name": "reason" },
                  ] },
                { "id": "4", "application_id": "200", "version": "14", "type": 1, "name": "config", "description": "Настройки",
                  "options": [
                      { "type": 1, "name": "show", "description": "Показать" },
                      { "type": 2, "name": "role", "options": [{ "type": 1, "name": "set", "description": "Роль", "options": [{ "type": 8, "name": "role", "required": true }] }] },
                  ] },
                { "id": "5", "application_id": "200", "version": "1", "type": 2, "name": "Report user" },
                { "id": "6", "application_id": "999", "version": "1", "type": 1, "name": "orphan" },
            ],
        })
    }

    fn entry<'a>(index: &'a Index, name: &str) -> &'a Entry {
        index.entries.iter().find(|e| e.name == name).unwrap()
    }

    #[test]
    fn index_keeps_chat_commands_and_flattens_subcommands() {
        let index = parse_index(&sample());
        let names: Vec<&str> = index.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["loop", "play", "ban", "config role set", "config show"]);
        assert_eq!(index.apps[0].icon.as_deref(), Some("https://cdn.discordapp.com/app-icons/100/abc123.png?size=64"));
        assert_eq!(index.apps[1].icon, None);
        assert_eq!(entry(&index, "config role set").options[0].kind, 8);
        assert_eq!(entry(&index, "play").pills(), ["query", "volume"]);
        assert!(parse_index(&json!({ "message": "Missing Access" })).entries.is_empty());
    }

    #[test]
    fn the_panel_filters_and_groups_by_bot() {
        let index = parse_index(&sample());
        assert_eq!(query("/pl"), Some("pl".into()));
        assert_eq!(query("/play x"), None);
        assert_eq!(query("hi /pl"), None);
        let all = rows(&index, "", &[]);
        assert_eq!(all[0], Row::Header(Some("100".into())));
        assert_eq!(all.len(), 2 + 5);
        let recent = vec![entry(&index, "ban").key()];
        let found = rows(&index, "", &recent);
        assert_eq!(found[0], Row::Header(None));
        assert_eq!(found[1], Row::Command(2));
        // "set" finds a subcommand by its own part; only that bot's section is shown.
        assert_eq!(rows(&index, "set", &recent), vec![Row::Header(Some("200".into())), Row::Command(3)]);
        assert!(rows(&index, "zzz", &recent).is_empty());
    }

    #[test]
    fn recent_keeps_the_newest_few() {
        let mut recent = Vec::new();
        for k in ["a", "b", "c", "d", "e", "f", "b"] {
            remember(&mut recent, k.into());
        }
        assert_eq!(recent, ["b", "f", "e", "d", "c"]);
    }

    #[test]
    fn typed_text_finds_the_command() {
        let index = parse_index(&sample());
        let (e, rest) = find(&index, "/play  never gonna").unwrap();
        assert_eq!((e.name.as_str(), rest), ("play", "never gonna"));
        let (e, rest) = find(&index, "/config role set <@&5>").unwrap();
        assert_eq!((e.name.as_str(), rest), ("config role set", "<@&5>"));
        assert_eq!(find(&index, "/config show").unwrap().0.name, "config show");
        assert!(find(&index, "/playlist").is_none());
        assert!(find(&index, "play").is_none());
    }

    #[test]
    fn plain_values_fill_options_in_order() {
        let index = parse_index(&sample());
        let play = entry(&index, "play");
        assert_eq!(arguments(play, "never gonna give").unwrap(), vec![json!({ "type": 3, "name": "query", "value": "never gonna give" })]);
        assert_eq!(
            arguments(play, "lofi volume:40").unwrap(),
            vec![json!({ "type": 3, "name": "query", "value": "lofi" }), json!({ "type": 4, "name": "volume", "value": 40 })]
        );
        assert_eq!(arguments(play, "volume:40 query:\"a b\"").unwrap()[0]["value"], "a b");
        assert_eq!(arguments(play, ""), Err(ArgError::Missing("query".into())));
        assert_eq!(arguments(play, "x volume:громко"), Err(ArgError::Bad("volume".into())));
    }

    #[test]
    fn values_are_converted_by_type() {
        let index = parse_index(&sample());
        let ban = entry(&index, "ban");
        let got = arguments(ban, "<@!42> yes за спам").unwrap();
        assert_eq!(got[0], json!({ "type": 6, "name": "user", "value": "42" }));
        assert_eq!(got[1], json!({ "type": 5, "name": "silent", "value": true }));
        assert_eq!(got[2], json!({ "type": 3, "name": "reason", "value": "за спам" }));
        assert_eq!(arguments(ban, "123456 reason:флуд").unwrap()[0]["value"], "123456");
        assert_eq!(arguments(ban, "Алекс"), Err(ArgError::Bad("user".into())));
        assert_eq!(arguments(entry(&index, "loop"), "трек").unwrap()[0]["value"], "track");
        assert_eq!(arguments(entry(&index, "loop"), "OFF").unwrap()[0]["value"], "off");
        assert_eq!(arguments(entry(&index, "loop"), "queue"), Err(ArgError::Bad("mode".into())));
        assert_eq!(arguments(entry(&index, "config show"), "лишнее"), Err(ArgError::Extra("лишнее".into())));
    }

    #[test]
    fn the_interaction_names_the_command_and_its_subcommand() {
        let index = parse_index(&sample());
        let set = entry(&index, "config role set");
        let options = wrap(set, arguments(set, "<@&7>").unwrap());
        let body = interaction(set, options, Some("55"), "66", "sess", &nonce(1_420_070_400_000 + 1000));
        assert_eq!(body["type"], 2);
        assert_eq!(body["application_id"], "200");
        assert_eq!(body["guild_id"], "55");
        assert_eq!(body["nonce"], (1000u64 << 22).to_string());
        assert_eq!(body["data"]["name"], "config");
        assert_eq!(body["data"]["version"], "14");
        assert_eq!(body["data"]["options"], json!([{ "type": 2, "name": "role", "options": [{ "type": 1, "name": "set", "options": [{ "type": 8, "name": "role", "value": "7" }] }] }]));
        assert_eq!(body["data"]["application_command"]["id"], "4");
        let dm = interaction(entry(&index, "play"), vec![], None, "66", "sess", "1");
        assert!(dm.get("guild_id").is_none());
        assert_eq!(index_path(None, "66").unwrap(), "/channels/66/application-command-index");
        assert_eq!(index_path(Some("55"), "66").unwrap(), "/guilds/55/application-command-index");
        assert!(index_path(Some("../x"), "66").is_err());
    }

    #[test]
    fn the_hint_follows_what_is_typed() {
        let index = parse_index(&sample());
        let ban = entry(&index, "ban");
        assert_eq!(hint(ban, ""), (vec!["user".into(), "silent".into(), "reason".into()], 0));
        assert_eq!(hint(ban, "<@1> ").1, 1);
        assert_eq!(hint(ban, "<@1> reason:сп").1, 2);
        assert_eq!(hint(entry(&index, "config show"), "").1, -1);
        // A string that takes the rest of the line stays lit while more words come.
        assert_eq!(hint(entry(&index, "play"), "never gonna ").1, 0);
    }
}
