//! Bots' message components: rows of buttons and select menus under a message, and the interaction a
//! click on one of them sends (`POST /interactions`, type 3).

use serde_json::{json, Value};

use crate::chat;

/// Discord's component types.
pub const ACTION_ROW: u64 = 1;
pub const BUTTON: u64 = 2;
pub const STRING_SELECT: u64 = 3;
const SECTION: u64 = 9;
const TEXT_DISPLAY: u64 = 10;
const CONTAINER: u64 = 17;

/// Button styles. Premium buttons (6) are not shown.
pub const LINK: u64 = 5;
const PREMIUM: u64 = 6;

/// The message is only shown to us (an ephemeral answer of a bot).
pub const EPHEMERAL: u64 = 1 << 6;
/// The message is laid out with components only ("components v2").
const COMPONENTS_V2: u64 = 1 << 15;

/// One option of a select menu.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    pub description: String,
    pub emoji: String,
    pub emoji_key: String,
    pub emoji_image: Option<String>,
    /// Chosen when the menu is first shown.
    pub default: bool,
}

/// A button or a select menu.
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    /// Discord's type: 2 a button; 3 a select of the bot's options; 5 users, 6 roles, 7 mentionables, 8 channels.
    pub kind: u64,
    /// A button's style: 1 primary, 2 secondary, 3 success, 4 danger, 5 link.
    pub style: u64,
    pub label: String,
    /// The emoji before the label: the emoji itself, or `:name:` for a custom one; its key (`name:id`) and picture.
    pub emoji: String,
    pub emoji_key: String,
    pub emoji_image: Option<String>,
    pub custom_id: String,
    /// Where a link button leads.
    pub url: String,
    pub disabled: bool,
    pub placeholder: String,
    pub options: Vec<SelectOption>,
    pub min: u64,
    pub max: u64,
}

impl Component {
    pub fn is_select(&self) -> bool {
        matches!(self.kind, 3 | 5..=8)
    }

    /// The labels of the options chosen to begin with, as the closed menu shows them.
    pub fn summary(&self) -> String {
        self.options.iter().filter(|o| o.default).map(|o| o.label.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// The values to send for the options picked, if they make a valid choice (known options, within the
    /// menu's minimum and maximum).
    pub fn choice(&self, picked: &[String]) -> Option<Vec<String>> {
        if self.kind != STRING_SELECT || self.disabled {
            return None;
        }
        let values: Vec<String> = self.options.iter().filter(|o| picked.contains(&o.value)).map(|o| o.value.clone()).collect();
        let n = values.len() as u64;
        (n == picked.len() as u64 && n >= self.min.max(1) && n <= self.max).then_some(values)
    }
}

/// What a message has in components: rows of buttons and menus, and (components v2) texts to show as its text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub rows: Vec<Vec<Component>>,
    pub texts: Vec<String>,
}

/// The emoji of a button or an option, as polls write it.
fn emoji_of(e: &Value) -> (String, String, Option<String>) {
    let name = e["name"].as_str().unwrap_or("");
    match e["id"].as_str().filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) {
        Some(id) => (format!(":{name}:"), format!("{name}:{id}"), Some(chat::emoji_picture(id, e["animated"].as_bool().unwrap_or(false)))),
        None => (name.to_string(), String::new(), None),
    }
}

fn component_of(c: &Value) -> Option<Component> {
    let kind = c["type"].as_u64()?;
    let (emoji, emoji_key, emoji_image) = emoji_of(&c["emoji"]);
    let text = |key: &str| c[key].as_str().unwrap_or("").to_string();
    let mut out = Component {
        kind,
        style: c["style"].as_u64().unwrap_or(0),
        label: text("label"),
        emoji,
        emoji_key,
        emoji_image,
        custom_id: text("custom_id"),
        url: text("url"),
        disabled: c["disabled"].as_bool().unwrap_or(false),
        placeholder: text("placeholder"),
        options: Vec::new(),
        min: c["min_values"].as_u64().unwrap_or(1),
        max: c["max_values"].as_u64().unwrap_or(1).max(1),
    };
    match kind {
        BUTTON => {
            let usable = match out.style {
                LINK => out.url.starts_with("https://") || out.url.starts_with("http://"),
                PREMIUM => false,
                1..=4 => !out.custom_id.is_empty(),
                _ => false,
            };
            (usable && (!out.label.is_empty() || !out.emoji.is_empty())).then_some(out)
        }
        3 | 5..=8 if !out.custom_id.is_empty() => {
            out.options = c["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|o| {
                    let (emoji, emoji_key, emoji_image) = emoji_of(&o["emoji"]);
                    Some(SelectOption {
                        label: o["label"].as_str()?.to_string(),
                        value: o["value"].as_str()?.to_string(),
                        description: o["description"].as_str().unwrap_or("").to_string(),
                        emoji,
                        emoji_key,
                        emoji_image,
                        default: o["default"].as_bool().unwrap_or(false),
                    })
                })
                .collect();
            out.max = out.max.min(out.options.len().max(1) as u64);
            Some(out)
        }
        _ => None,
    }
}

/// Walks components v2: text displays become text, action rows rows, a section's button accessory a row of its own.
fn walk_v2(list: &[Value], out: &mut Parsed) {
    for c in list {
        match c["type"].as_u64() {
            Some(ACTION_ROW) => push_row(c, out),
            Some(TEXT_DISPLAY) => out.texts.extend(c["content"].as_str().filter(|t| !t.is_empty()).map(String::from)),
            Some(SECTION) => {
                walk_v2(c["components"].as_array().map(Vec::as_slice).unwrap_or(&[]), out);
                out.rows.extend(component_of(&c["accessory"]).map(|b| vec![b]));
            }
            Some(CONTAINER) => walk_v2(c["components"].as_array().map(Vec::as_slice).unwrap_or(&[]), out),
            // Media galleries, files, separators, thumbnails: not drawn.
            _ => {}
        }
    }
}

fn push_row(row: &Value, out: &mut Parsed) {
    let items: Vec<Component> = row["components"].as_array().into_iter().flatten().filter_map(component_of).collect();
    if !items.is_empty() {
        out.rows.push(items);
    }
}

/// The components of a message.
pub fn parse(m: &Value) -> Parsed {
    let mut out = Parsed::default();
    let list = m["components"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if flags(m) & COMPONENTS_V2 != 0 {
        walk_v2(list, &mut out);
    } else {
        for row in list.iter().filter(|r| r["type"].as_u64() == Some(ACTION_ROW)) {
            push_row(row, &mut out);
        }
    }
    out
}

pub fn flags(m: &Value) -> u64 {
    m["flags"].as_u64().unwrap_or(0)
}

/// The component with this custom id.
pub fn find<'a>(rows: &'a [Vec<Component>], custom_id: &str) -> Option<&'a Component> {
    rows.iter().flatten().find(|c| c.custom_id == custom_id)
}

/// The message a component belongs to, as the interaction names it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Target {
    pub guild: Option<String>,
    pub channel: String,
    pub message: String,
    pub flags: u64,
    /// The bot that made the message (its application; a bot's own messages carry it as the author).
    pub application: String,
}

pub fn target_of(m: &Value) -> Target {
    let text = |v: &Value| v.as_str().unwrap_or("").to_string();
    Target {
        guild: m["guild_id"].as_str().map(String::from),
        channel: text(&m["channel_id"]),
        message: text(&m["id"]),
        flags: flags(m),
        application: m["application_id"].as_str().or(m["author"]["id"].as_str()).unwrap_or("").to_string(),
    }
}

/// The body of `POST /interactions` for a button click (`values` None) or a select menu's choice.
pub fn interaction(target: &Target, component: &Component, values: Option<&[String]>, session: &str, nonce: &str) -> Result<Value, String> {
    for id in [&target.channel, &target.message, &target.application].into_iter().chain(target.guild.as_ref()) {
        crate::snowflake(id)?;
    }
    let mut data = json!({ "component_type": component.kind, "custom_id": component.custom_id });
    if let Some(values) = values {
        data["type"] = json!(component.kind);
        data["values"] = json!(values);
    }
    let mut body = json!({
        "type": 3,
        "nonce": nonce,
        "channel_id": target.channel,
        "message_flags": target.flags,
        "message_id": target.message,
        "application_id": target.application,
        "session_id": session,
        "data": data,
    });
    if let Some(g) = &target.guild {
        body["guild_id"] = json!(g);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "id": "900", "channel_id": "800", "guild_id": "700", "flags": 0, "application_id": "600",
            "author": { "id": "601", "bot": true },
            "components": [
                { "type": 1, "components": [
                    { "type": 2, "style": 1, "label": "Да", "custom_id": "yes" },
                    { "type": 2, "style": 4, "label": "Нет", "custom_id": "no", "disabled": true },
                    { "type": 2, "style": 5, "label": "Сайт", "url": "https://example.com" },
                    { "type": 2, "style": 3, "emoji": { "id": "55", "name": "ok" }, "custom_id": "ok" },
                    { "type": 2, "style": 6, "sku_id": "1" },
                    { "type": 2, "style": 5, "label": "Плохая", "url": "javascript:alert(1)" },
                ] },
                { "type": 1, "components": [
                    { "type": 3, "custom_id": "pick", "placeholder": "Выбери", "min_values": 1, "max_values": 2, "options": [
                        { "label": "Красный", "value": "r", "description": "тёплый", "emoji": { "name": "🔴" }, "default": true },
                        { "label": "Синий", "value": "b" },
                        { "label": "Зелёный", "value": "g" },
                    ] },
                ] },
            ],
        })
    }

    #[test]
    fn rows_of_buttons_and_menus_are_read() {
        let p = parse(&sample());
        assert!(p.texts.is_empty());
        assert_eq!(p.rows.len(), 2);
        let buttons = &p.rows[0];
        assert_eq!(buttons.len(), 4, "premium and unsafe link buttons are dropped");
        assert_eq!((buttons[0].style, buttons[0].label.as_str(), buttons[0].disabled), (1, "Да", false));
        assert!(buttons[1].disabled);
        assert_eq!((buttons[2].style, buttons[2].url.as_str()), (LINK, "https://example.com"));
        assert_eq!(buttons[3].emoji_key, "ok:55");
        let menu = &p.rows[1][0];
        assert!(menu.is_select());
        assert_eq!((menu.options.len(), menu.min, menu.max), (3, 1, 2));
        assert_eq!(menu.summary(), "Красный");
        assert_eq!(menu.options[0].emoji, "🔴");
        assert_eq!(find(&p.rows, "pick").map(|c| c.kind), Some(STRING_SELECT));
        assert!(find(&p.rows, "nothing").is_none());
        assert_eq!(parse(&json!({ "content": "просто текст" })), Parsed::default());
    }

    #[test]
    fn a_menu_choice_must_fit() {
        let p = parse(&sample());
        let menu = &p.rows[1][0];
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(menu.choice(&s(&["b", "r"])), Some(s(&["r", "b"])), "in the menu's order");
        assert_eq!(menu.choice(&s(&["r", "b", "g"])), None, "more than the maximum");
        assert_eq!(menu.choice(&[]), None, "fewer than the minimum");
        assert_eq!(menu.choice(&s(&["x"])), None, "not an option");
    }

    #[test]
    fn components_v2_give_text_and_rows() {
        let m = json!({ "flags": 1 << 15, "components": [
            { "type": 17, "components": [
                { "type": 10, "content": "## Заголовок" },
                { "type": 9, "components": [{ "type": 10, "content": "в секции" }],
                  "accessory": { "type": 2, "style": 2, "label": "Ещё", "custom_id": "more" } },
                { "type": 14 },
                { "type": 12, "items": [] },
                { "type": 1, "components": [{ "type": 2, "style": 1, "label": "OK", "custom_id": "ok" }] },
            ] },
        ] });
        let p = parse(&m);
        assert_eq!(p.texts, vec!["## Заголовок", "в секции"]);
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rows[0][0].custom_id, "more");
        assert_eq!(p.rows[1][0].label, "OK");
    }

    #[test]
    fn a_click_and_a_choice_become_interactions() {
        let m = sample();
        let p = parse(&m);
        let target = target_of(&m);
        assert_eq!(target.application, "600");
        let click = interaction(&target, &p.rows[0][0], None, "sess", "123").unwrap();
        assert_eq!(click, json!({
            "type": 3, "nonce": "123", "guild_id": "700", "channel_id": "800", "message_flags": 0, "message_id": "900",
            "application_id": "600", "session_id": "sess", "data": { "component_type": 2, "custom_id": "yes" },
        }));
        let values = vec!["r".to_string(), "b".to_string()];
        let pick = interaction(&target, &p.rows[1][0], Some(&values), "sess", "124").unwrap();
        assert_eq!(pick["data"], json!({ "component_type": 3, "custom_id": "pick", "type": 3, "values": ["r", "b"] }));
        // In a DM there is no server, and the bot is the author when no application is named.
        let dm = json!({ "id": "1", "channel_id": "2", "flags": 64, "author": { "id": "3" } });
        let t = target_of(&dm);
        let body = interaction(&t, &p.rows[0][0], None, "s", "n").unwrap();
        assert!(body.get("guild_id").is_none());
        assert_eq!((body["application_id"].as_str(), body["message_flags"].as_u64()), (Some("3"), Some(64)));
        assert!(interaction(&Target { channel: "x".into(), ..t }, &p.rows[0][0], None, "s", "n").is_err());
    }
}
