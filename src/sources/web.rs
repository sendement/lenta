//! Games from elsewhere matched to Steam's store: by title when nothing
//! local names the id, and their art from the store when the launcher has
//! none. Answers are kept a month (misses too) so this costs nothing after
//! the first time.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::game::{Art, Game};
use crate::sources::steam::Achievement;

const KEEP: Duration = Duration::from_secs(30 * 86400);

fn cache_file(sub: &str, name: &str) -> PathBuf {
    crate::net::cache_dir().join(sub).join(name)
}

/// The cached body if it's fresh, else a new one (kept when `ok`), else
/// the stale one.
fn cached_get(file: PathBuf, url: &str, ok: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    let old = std::fs::read(&file).ok();
    let fresh = std::fs::metadata(&file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|a| a < KEEP);
    if fresh {
        return old;
    }
    match crate::net::get(url) {
        Ok(b) if ok(&b) => {
            let _ = std::fs::create_dir_all(file.parent()?);
            let _ = std::fs::write(&file, &b);
            Some(b)
        }
        _ => old,
    }
}

/// Lower-case words of letters and digits; marks like ™ and ® dropped.
pub fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Steam's id for a title, when the store search finds that very game.
pub fn lookup(title: &str) -> Option<u32> {
    let want = words(title);
    if want.is_empty() {
        return None;
    }
    let key = want.join(" ");
    let url = format!(
        "https://store.steampowered.com/api/storesearch/?term={}&l=english&cc=US",
        urlencode(&key)
    );
    let body = cached_get(
        cache_file("match", &format!("{:016x}.json", crate::net::hash(&key))),
        &url,
        |b| serde_json::from_slice::<Value>(b).is_ok(),
    )?;
    let v: Value = serde_json::from_slice(&body).ok()?;
    let items = v.get("items")?.as_array()?;
    let named = |it: &Value| {
        Some((
            it.get("id")?.as_u64()? as u32,
            words(it.get("name")?.as_str()?),
        ))
    };
    let found: Vec<(u32, Vec<String>)> = items.iter().filter_map(named).collect();
    // The same words; or the first hit holding every word of the title
    // ("Hades" → "Hades", not "Hades II").
    found
        .iter()
        .find(|(_, w)| *w == want)
        .or_else(|| {
            found
                .first()
                .filter(|(_, w)| w.len() <= want.len() + 2 && want.iter().all(|x| w.contains(x)))
        })
        .map(|(id, _)| *id)
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Fetch the store's art for many ids at once (fifty per request) into
/// the same cache `assets` reads, skipping the ones already there.
pub fn prefetch_assets(ids: &[u32]) {
    let missing: Vec<u32> = ids
        .iter()
        .copied()
        .filter(|id| {
            std::fs::metadata(cache_file("assets", &format!("{id}.json")))
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_none_or(|a| a > KEEP)
        })
        .collect();
    for chunk in missing.chunks(50) {
        let list: Vec<String> = chunk
            .iter()
            .map(|id| format!(r#"{{"appid":{id}}}"#))
            .collect();
        let input = format!(
            r#"{{"ids":[{}],"context":{{"language":"{}","country_code":"US"}},"data_request":{{"include_assets":true}}}}"#,
            list.join(","),
            crate::i18n::steam_lang()
        );
        let url = format!(
            "https://api.steampowered.com/IStoreBrowseService/GetItems/v1/?input_json={}",
            urlencode(&input)
        );
        let Ok(b) = crate::net::get(&url) else { return };
        let Ok(v) = serde_json::from_slice::<Value>(&b) else {
            return;
        };
        let Some(items) = v.pointer("/response/store_items").and_then(Value::as_array) else {
            continue;
        };
        for it in items {
            let (Some(id), Some(_)) = (
                it.get("appid")
                    .or_else(|| it.get("id"))
                    .and_then(Value::as_u64),
                it.get("assets"),
            ) else {
                continue;
            };
            let one = serde_json::json!({"response": {"store_items": [it]}});
            let f = cache_file("assets", &format!("{id}.json"));
            let _ = std::fs::create_dir_all(f.parent().unwrap_or(&f));
            let _ = std::fs::write(&f, one.to_string());
        }
    }
}

/// The store's art for an id: (portrait, landscape, hero, logo).
pub fn assets(appid: u32) -> Option<[Option<Art>; 4]> {
    let input = format!(
        r#"{{"ids":[{{"appid":{appid}}}],"context":{{"language":"{}","country_code":"US"}},"data_request":{{"include_assets":true}}}}"#,
        crate::i18n::steam_lang()
    );
    let url = format!(
        "https://api.steampowered.com/IStoreBrowseService/GetItems/v1/?input_json={}",
        urlencode(&input)
    );
    let body = cached_get(cache_file("assets", &format!("{appid}.json")), &url, |b| {
        serde_json::from_slice::<Value>(b)
            .ok()
            .and_then(|v| v.pointer("/response/store_items/0/assets").cloned())
            .is_some()
    })?;
    let v: Value = serde_json::from_slice(&body).ok()?;
    let a = v.pointer("/response/store_items/0/assets")?;
    let format = a.get("asset_url_format")?.as_str()?;
    let art = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| a.get(*k).and_then(Value::as_str))
            .map(|f| {
                Art::Url(format!(
                    "https://shared.steamstatic.com/store_item_assets/{}",
                    format.replace("${FILENAME}", f)
                ))
            })
    };
    Some([
        art(&["library_capsule_2x", "library_capsule"]),
        art(&["header_2x", "header", "main_capsule"]),
        art(&["library_hero", "page_background"]),
        art(&["library_logo"]),
    ])
}

/// Match the games from elsewhere to the store and fill the art they
/// lack; give Steam's own games the store's sharper covers. A few games
/// at a time: the first run waits on the network, later ones on the cache.
pub fn enrich(games: &mut [Game]) {
    let queue = std::sync::Mutex::new(games.iter_mut());
    std::thread::scope(|s| {
        for _ in 0..8 {
            // Not `while let`: its lock would be held through the work.
            #[allow(clippy::while_let_loop)]
            s.spawn(|| {
                loop {
                    let Some(g) = queue.lock().ok().and_then(|mut q| q.next()) else {
                        break;
                    };
                    match g.steam_appid {
                        Some(id) => sharpen(g, id),
                        None => match_store(g),
                    }
                }
            });
        }
    });
}

fn match_store(g: &mut Game) {
    if g.store_appid.is_none() {
        g.store_appid = lookup(&g.title);
    }
    let Some(id) = g.store_appid else { return };
    let a = &g.art;
    if a.portrait.is_some() && a.landscape.is_some() && a.hero.is_some() && a.logo.is_some() {
        return;
    }
    if let Some([portrait, landscape, hero, logo]) = assets(id) {
        let a = &mut g.art;
        a.portrait = a.portrait.take().or(portrait);
        a.landscape = a.landscape.take().or(landscape);
        a.hero = a.hero.take().or(hero);
        a.logo = a.logo.take().or(logo);
    }
}

/// A Steam game's cover and capsule from the store when Steam itself has
/// only small ones (or none): the guessed CDN address 404s for newer games,
/// and the local wide capsule is 460×215. Art the user set stays.
fn sharpen(g: &mut Game, id: u32) {
    let custom = |a: &Option<Art>| matches!(a, Some(Art::File(p)) if p.to_string_lossy().contains("/config/grid/"));
    let a = &g.art;
    let guessed = matches!(a.portrait, Some(Art::Url(_)));
    if !guessed && custom(&a.landscape) {
        return;
    }
    let Some([portrait, landscape, _, _]) = assets(id) else {
        return;
    };
    let a = &mut g.art;
    if guessed && portrait.is_some() {
        a.portrait = portrait;
    }
    if !custom(&a.landscape) && landscape.is_some() {
        // The local one stays as the fallback (offline, or a failed fetch).
        if let Some(Art::File(_)) = &a.landscape {
            a.landscape_alt = a.landscape.take();
        }
        a.landscape = landscape;
    }
}

/// Every achievement of a Steam game with its name in `lang`, icons and
/// how many players have it; none marked as earned. Kept a week.
pub fn achievement_list(appid: u32, lang: &str) -> Option<Vec<Achievement>> {
    let url = format!(
        "https://api.steampowered.com/IPlayerService/GetGameAchievements/v1/?appid={appid}&language={lang}"
    );
    let file = cache_file("achievements", &format!("{appid}-{lang}.json"));
    let fresh = std::fs::metadata(&file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|a| a < Duration::from_secs(7 * 86400));
    let body = if fresh {
        std::fs::read(&file).ok()
    } else {
        // Shares move: a week-old copy is refreshed (kept if offline).
        match crate::net::get(&url) {
            Ok(b) if parse_achievements(appid, &b).is_some() => {
                let _ = std::fs::create_dir_all(file.parent()?);
                let _ = std::fs::write(&file, &b);
                Some(b)
            }
            _ => std::fs::read(&file).ok(),
        }
    }?;
    parse_achievements(appid, &body)
}

fn parse_achievements(appid: u32, body: &[u8]) -> Option<Vec<Achievement>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let list = v.pointer("/response/achievements")?.as_array()?;
    let icon =
        |a: &Value, k: &str| {
            a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(|f| {
            Art::Url(format!(
                "https://shared.fastly.steamstatic.com/community_assets/images/apps/{appid}/{f}"
            ))
        })
        };
    let s = |a: &Value, k: &str| {
        a.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let out: Vec<Achievement> = list
        .iter()
        .map(|a| Achievement {
            id: s(a, "internal_name"),
            name: s(a, "localized_name"),
            desc: s(a, "localized_desc"),
            icon: icon(a, "icon"),
            icon_gray: icon(a, "icon_gray"),
            hidden: a.get("hidden").and_then(Value::as_bool).unwrap_or(false),
            unlocked: None,
            percent: a
                .get("player_percent_unlocked")
                .and_then(Value::as_str)
                .and_then(|p| p.parse().ok()),
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_words() {
        assert_eq!(words("HELLDIVERS™ 2"), ["helldivers", "2"]);
        assert_eq!(
            words("Hades: Battle Out of Hell"),
            ["hades", "battle", "out", "of", "hell"]
        );
    }

    /// Talks to the store: `cargo test lookup_live -- --ignored`.
    #[test]
    #[ignore]
    fn lookup_live() {
        unsafe { std::env::set_var("LENTA_CACHE", std::env::temp_dir().join("lenta-test-cache")) };
        assert_eq!(lookup("Hades"), Some(1145360));
        assert_eq!(lookup("Hollow Knight"), Some(367520));
        assert_eq!(lookup("Celeste"), Some(504230));
        assert_eq!(lookup("zzqx not a real game title"), None);
    }
}
