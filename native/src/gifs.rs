//! The GIF panel: Discord's own GIF search (Tenor behind it), read into what the panel shows.

use serde_json::Value;

/// How many tiles the panel shows at first, and how many more each time it is scrolled to the end.
pub const PAGE: usize = 12;
/// A tile's picture, in pixels (shown at the same size).
pub const TILE_W: u32 = 152;
pub const TILE_H: u32 = 112;
/// Previews are small loops: more frames than this are not kept.
pub const MAX_FRAMES: usize = 40;

/// One tile: a GIF to send, or a category that searches for its name.
#[derive(Debug, Clone, PartialEq)]
pub struct Gif {
    /// What goes into the message (the Tenor page, which Discord shows as the moving picture).
    pub url: String,
    /// The animated picture shown on the tile, only from a known media host.
    pub preview: Option<String>,
    /// A still picture for when the animated one can't be read.
    pub still: Option<String>,
    /// A category's name (empty for a GIF).
    pub category: String,
}

/// The tenor `.gif` of a Tenor file in another format: the id's last five letters name the format.
pub fn tenor_gif(url: &str) -> Option<String> {
    let rest = url.split('?').next()?.strip_prefix("https://media.tenor.com/")?;
    let (id, file) = rest.split_once('/')?;
    if id.len() <= 5 || !id.is_ascii() {
        return None;
    }
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    Some(format!("https://media.tenor.com/{}AAAAC/{stem}.gif", &id[..id.len() - 5]))
}

/// The address of a picture that may be fetched: https on a known media host only.
fn trusted(url: Option<&str>) -> Option<String> {
    url.filter(|u| crate::is_trusted_media_host(u)).map(String::from)
}

/// A file that the picture decoder plays (GIF or WebP), from the fields Discord gives.
fn moving(v: &Value) -> Option<String> {
    let src = v["src"].as_str();
    let playable = |u: &&str| {
        let path = u.split('?').next().unwrap_or(u);
        path.ends_with(".gif") || path.ends_with(".webp")
    };
    trusted(v["gif_src"].as_str())
        .or_else(|| trusted(src.filter(playable)))
        .or_else(|| src.and_then(tenor_gif).and_then(|u| trusted(Some(&u))))
}

fn gif_of(v: &Value) -> Option<Gif> {
    // Only what Discord's GIF search itself names goes into a message: a Tenor or Giphy page or file.
    let url = trusted(v["url"].as_str().or(v["gif_src"].as_str()).or(v["src"].as_str()))?;
    Some(Gif { url, preview: moving(v), still: trusted(v["preview"].as_str()), category: String::new() })
}

fn category_of(v: &Value) -> Option<Gif> {
    let name = v["name"].as_str().filter(|n| !n.trim().is_empty())?;
    Some(Gif { url: String::new(), preview: moving(v), still: None, category: name.to_string() })
}

/// What a GIF request brings: a bare list (search), or `{ gifs, categories }` (trending). Entries without
/// what's needed are left out; GIFs come first, then categories.
pub fn parse(v: &Value) -> Vec<Gif> {
    let gifs = v.as_array().or_else(|| v["gifs"].as_array()).into_iter().flatten().filter_map(gif_of);
    let categories = v["categories"].as_array().into_iter().flatten().filter_map(category_of);
    gifs.chain(categories).collect()
}

/// Discord's locale for the request: English or Russian, as the app is.
pub fn locale(english: bool) -> &'static str {
    if english {
        "en-US"
    } else {
        "ru"
    }
}

/// The request for what the panel shows: trending when nothing is typed, else a search.
pub fn path(query: &str, locale: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        format!("/gifs/trending?provider=tenor&locale={locale}&media_format=mp4")
    } else {
        format!("/gifs/search?q={}&provider=tenor&media_format=mp4&locale={locale}", crate::percent_encode(query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn search_results_become_tiles() {
        let found = json!([
            { "url": "https://tenor.com/view/cat-123", "src": "https://media.tenor.com/abcdefAAAPo/cat.mp4",
              "gif_src": "https://media.tenor.com/abcdefAAAAC/cat.gif", "preview": "https://media.tenor.com/abcdefAAAAe/cat.png", "width": 498, "height": 280 },
            { "url": "https://tenor.com/view/dog-1", "src": "https://media.tenor.com/xyz12AAAPo/dog.mp4" },
            { "url": "http://example.com/plain" },
            { "src": "https://evil.example/x.gif" },
        ]);
        let tiles = parse(&found);
        assert_eq!(tiles.len(), 2, "plain http and unknown hosts are left out: {tiles:?}");
        assert_eq!(tiles[0].url, "https://tenor.com/view/cat-123");
        assert_eq!(tiles[0].preview.as_deref(), Some("https://media.tenor.com/abcdefAAAAC/cat.gif"));
        assert_eq!(tiles[0].still.as_deref(), Some("https://media.tenor.com/abcdefAAAAe/cat.png"));
        // No gif_src: the gif beside the mp4.
        assert_eq!(tiles[1].preview.as_deref(), Some("https://media.tenor.com/xyz12AAAAC/dog.gif"));
    }

    #[test]
    fn previews_come_only_from_known_hosts() {
        let tiles = parse(&json!([{ "url": "https://tenor.com/view/a", "gif_src": "https://evil.example/a.gif", "src": "https://evil.example/a.webp" }]));
        assert_eq!(tiles[0].preview, None);
        assert_eq!(tiles[0].still, None);
    }

    #[test]
    fn trending_brings_gifs_and_categories() {
        let trending = json!({
            "gifs": [{ "url": "https://tenor.com/view/t", "gif_src": "https://media.tenor.com/ttttttAAAAC/t.gif" }],
            "categories": [{ "name": "привет", "src": "https://media.tenor.com/hiiiiiAAAPo/hi.mp4" }, { "name": "" }],
        });
        let tiles = parse(&trending);
        assert_eq!(tiles.len(), 2);
        assert!(tiles[0].category.is_empty());
        assert_eq!(tiles[1].category, "привет");
        assert_eq!(tiles[1].preview.as_deref(), Some("https://media.tenor.com/hiiiiiAAAAC/hi.gif"));
        assert!(parse(&json!(null)).is_empty());
        assert!(parse(&json!({ "message": "rate limited" })).is_empty());
    }

    #[test]
    fn requests_are_trending_or_a_search() {
        assert_eq!(path("  ", "ru"), "/gifs/trending?provider=tenor&locale=ru&media_format=mp4");
        assert_eq!(path("кот & dog", "en-US"), "/gifs/search?q=%D0%BA%D0%BE%D1%82%20%26%20dog&provider=tenor&media_format=mp4&locale=en-US");
        assert_eq!(tenor_gif("https://example.com/abcdefAAAPo/x.mp4"), None);
        assert_eq!(tenor_gif("https://media.tenor.com/abc/x.mp4"), None);
    }
}
