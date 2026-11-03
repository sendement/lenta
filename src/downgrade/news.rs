//! Human version numbers ("1.16.1") for builds. Steam knows only build
//! ids; version numbers live in patch note titles, from Steam news (public,
//! no key), matched to builds by date.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub time: i64,
    pub version: String,
    pub title: String,
}

static KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:patch|update|version|ver|hotfix|release|notes?|v)\b\.?").unwrap()
});
static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)v?(\d{1,4}(?:\.\d{1,5}){1,3}[a-z]?)").unwrap());
// Things that look like x.y but aren't versions: "4.5 million", "2.0x".
static NOT_AFTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:million|billion|k\b|m\b|%|x\b|gb|mb|fps|hz)").unwrap()
});

fn wordish(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The version number a patch note title mentions.
pub fn extract_version(title: &str, game: &str) -> Option<String> {
    // "NieR Replicant ver.1.22474487139..." is a name, not a version.
    let t = if game.is_empty() {
        title.to_string()
    } else {
        title.replace(game, " ")
    };
    let mut best: Option<(u8, String)> = None;
    let mut from = 0;
    while let Some(c) = VERSION.captures_at(&t, from) {
        let whole = c.get(0).unwrap();
        let num = c.get(1).unwrap();
        from = whole.start() + whole.as_str().chars().next().map_or(1, char::len_utf8);
        // Not glued to a word or a dot before it…
        if t[..whole.start()]
            .chars()
            .next_back()
            .is_some_and(|ch| wordish(ch) || ch == '.')
        {
            continue;
        }
        // …nor running on into more digits after.
        let tail: String = t[num.end()..]
            .chars()
            .take_while(|&ch| wordish(ch) || ch == '.')
            .collect();
        if tail.chars().any(|ch| ch.is_ascii_digit()) {
            continue;
        }
        if NOT_AFTER.is_match(&t[num.end()..]) {
            continue;
        }
        // Prefer numbers right after "patch", "version"…
        let before: String = {
            let s = &t[..whole.start()];
            let start = s.char_indices().rev().nth(11).map_or(0, |(i, _)| i);
            s[start..].to_string()
        };
        let rank = if KEYWORD.is_match(&before) { 0 } else { 1 };
        if best.as_ref().is_none_or(|(r, _)| rank < *r) {
            best = Some((rank, num.as_str().to_string()));
        }
        from = num.end();
    }
    best.map(|(_, v)| v)
}

/// Patch notes from Steam news, oldest first: (time, version, title, url).
pub fn fetch(app: &str, game: &str) -> Result<Vec<(i64, String, String, String)>, String> {
    let url = format!(
        "https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/?appid={app}&count=500&maxlength=1&feeds=steam_community_announcements"
    );
    let b = crate::net::get(&url)?;
    let v: Value = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for n in v
        .pointer("/appnews/newsitems")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let title = n.get("title").and_then(Value::as_str).unwrap_or_default();
        let Some(ver) = extract_version(title, game) else {
            continue;
        };
        let patch = n
            .get("tags")
            .and_then(Value::as_array)
            .is_some_and(|t| t.iter().any(|x| x.as_str() == Some("patchnotes")));
        // Patch-note posts, titled updates, or unmistakable x.y.z numbers.
        let named = KEYWORD.is_match(&title.replace(game, ""));
        if !patch && ver.matches('.').count() < 2 && !named {
            continue;
        }
        out.push((
            n.get("date").and_then(Value::as_i64).unwrap_or(0),
            ver,
            title.to_string(),
            n.get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ));
    }
    out.sort_by_key(|n| n.0);
    Ok(out)
}

/// The newest note published by `t` (+`lead`).
pub fn version_at(t: i64, notes: &[Note], lead: i64) -> Option<&Note> {
    if t == 0 {
        return None;
    }
    notes.iter().take_while(|n| n.time <= t + lead).last()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_from_titles() {
        let e = |t: &str, g: &str| extract_version(t, g);
        assert_eq!(e("Patch 1.16.1 is live", "").as_deref(), Some("1.16.1"));
        assert_eq!(e("Update v2.3 notes", "").as_deref(), Some("2.3"));
        assert_eq!(e("4.5 million players!", ""), None);
        assert_eq!(
            e(
                "NieR Replicant ver.1.22474487139... Update",
                "NieR Replicant ver.1.22474487139..."
            ),
            None
        );
        assert_eq!(e("Hotfix 1.0.3a", "").as_deref(), Some("1.0.3a"));
        assert_eq!(e("Build 12.34.56.78.90", ""), None);
    }
}
