//! A Steam game's store page (description, developer, genres, screenshots)
//! from the store's appdetails API, kept for a week; and its achievements
//! read from Steam's local cache. Both loaded off the UI thread.

use std::collections::HashMap;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use serde_json::Value;

use crate::game::{Art, Game};
use crate::sources::steam::{self, Achievement};
use crate::sources::{emu, web};

#[derive(Clone, Debug, Default)]
pub struct Details {
    pub short: String,
    pub about: String,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub genres: Vec<String>,
    pub released: Option<String>,
    pub metacritic: Option<i64>,
    /// (thumbnail, full size).
    pub shots: Vec<(Art, Art)>,
}

pub enum State<T> {
    Loading,
    Ready(T),
    Failed,
}

enum Msg {
    Details(u32, Option<Details>),
    Ach(String, Option<Vec<Achievement>>),
}

pub struct Store {
    ctx: egui::Context,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    details: HashMap<u32, State<Details>>,
    /// By game key: the same store id may be two games with two saves.
    ach: HashMap<String, State<Vec<Achievement>>>,
}

const FRESH: Duration = Duration::from_secs(7 * 86400);

impl Store {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Store {
            ctx,
            tx,
            rx,
            details: HashMap::new(),
            ach: HashMap::new(),
        }
    }

    pub fn pump(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Details(id, d) => {
                    self.details
                        .insert(id, d.map_or(State::Failed, State::Ready));
                }
                Msg::Ach(id, a) => {
                    self.ach.insert(id, a.map_or(State::Failed, State::Ready));
                }
            }
        }
    }

    pub fn details(&mut self, appid: u32) -> &State<Details> {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.details.entry(appid).or_insert_with(|| {
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Details(appid, fetch(appid, crate::i18n::steam_lang())));
                ctx.request_repaint();
            });
            State::Loading
        })
    }

    /// A game's achievements: Steam's own record for a Steam game, an
    /// emulator's save matched to the store's list for one from elsewhere.
    pub fn achievements(&mut self, g: &Game) -> &State<Vec<Achievement>> {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let (key, steam_id, emu) = (g.key.clone(), g.steam_appid, g.emu.clone());
        self.ach.entry(g.key.clone()).or_insert_with(|| {
            std::thread::spawn(move || {
                let lang = crate::i18n::steam_lang();
                let list = match (steam_id, emu) {
                    (Some(id), _) => steam::achievements(id, lang).map(|mut l| {
                        // How rare each is, when the store answers.
                        if let Some(all) = web::achievement_list(id, lang) {
                            let pct: HashMap<&str, f32> = all
                                .iter()
                                .filter_map(|a| Some((a.id.as_str(), a.percent?)))
                                .collect();
                            for a in &mut l {
                                a.percent = pct.get(a.id.as_str()).copied();
                            }
                        }
                        l
                    }),
                    (None, Some(save)) => web::achievement_list(save.appid, lang).map(|mut l| {
                        let got = emu::unlocked(&save.file);
                        let lower: HashMap<String, u64> =
                            got.iter().map(|(k, v)| (k.to_lowercase(), *v)).collect();
                        for a in &mut l {
                            a.unlocked = got
                                .get(&a.id)
                                .or_else(|| lower.get(&a.id.to_lowercase()))
                                .copied();
                        }
                        l
                    }),
                    (None, None) => None,
                };
                let _ = tx.send(Msg::Ach(key, list));
                ctx.request_repaint();
            });
            State::Loading
        })
    }

    /// Forget what was loaded (after a rescan: play time and achievements
    /// may have moved on).
    pub fn forget_achievements(&mut self) {
        self.ach.retain(|_, s| matches!(s, State::Loading));
    }

    /// Forget everything (the language changed).
    pub fn clear(&mut self) {
        self.details.retain(|_, s| matches!(s, State::Loading));
        self.forget_achievements();
    }
}

fn fetch(appid: u32, lang: &str) -> Option<Details> {
    let file = crate::net::cache_dir().join(format!("store/{appid}-{lang}.json"));
    let fresh = std::fs::metadata(&file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|a| a < FRESH);
    let cached = std::fs::read(&file).ok();
    let body = if fresh {
        cached
    } else {
        let url = format!("https://store.steampowered.com/api/appdetails?appids={appid}&l={lang}");
        match crate::net::get(&url) {
            Ok(b) if parse(appid, &b).is_some() => {
                let _ = std::fs::create_dir_all(file.parent()?);
                let _ = std::fs::write(&file, &b);
                Some(b)
            }
            // Offline or refused: what was kept, however old.
            _ => cached,
        }
    }?;
    parse(appid, &body)
}

fn parse(appid: u32, body: &[u8]) -> Option<Details> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let app = v.get(appid.to_string())?;
    if app.get("success").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let d = app.get("data")?;
    let s = |k: &str| {
        d.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let list = |k: &str| {
        d.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(Details {
        short: html_to_text(&s("short_description")),
        about: html_to_text(&{
            let a = s("about_the_game");
            if a.trim().is_empty() {
                s("detailed_description")
            } else {
                a
            }
        }),
        developers: list("developers"),
        publishers: list("publishers"),
        genres: d
            .get("genres")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|g| g.get("description")?.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        released: d
            .pointer("/release_date/date")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        metacritic: d.pointer("/metacritic/score").and_then(Value::as_i64),
        shots: d
            .get("screenshots")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        Some((
                            Art::Url(x.get("path_thumbnail")?.as_str()?.to_string()),
                            Art::Url(x.get("path_full")?.as_str()?.to_string()),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Store HTML as plain text: paragraphs, line breaks and list bullets kept.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[i + 1..i + j].trim().to_lowercase();
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        match name {
            "br" => out.push('\n'),
            "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "ul" | "ol" => {
                if !out.ends_with("\n\n") {
                    out.push_str(if out.ends_with('\n') { "\n" } else { "\n\n" });
                }
            }
            "li" if !tag.starts_with('/') => out.push_str("\n• "),
            _ => {}
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    let text = decode_entities(&out);
    // Tidy: no trailing spaces, at most one blank line in a row.
    let mut tidy = String::new();
    let mut blank = 0;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            blank += 1;
            if blank == 1 && !tidy.is_empty() {
                tidy.push('\n');
            }
        } else {
            blank = 0;
            tidy.push_str(l);
            tidy.push('\n');
        }
    }
    tidy.trim().to_string()
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = tail.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| {
            let ent = &tail[1..e];
            let c = match ent {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" | "#39" => '\'',
                "nbsp" => ' ',
                "mdash" => '—',
                "ndash" => '–',
                "laquo" => '«',
                "raquo" => '»',
                "hellip" => '…',
                "trade" => '™',
                "reg" => '®',
                "copy" => '©',
                _ if ent.starts_with("#x") => {
                    char::from_u32(u32::from_str_radix(&ent[2..], 16).ok()?)?
                }
                _ if ent.starts_with('#') => char::from_u32(ent[1..].parse().ok()?)?,
                _ => return None,
            };
            Some((c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &tail[e + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html() {
        let t = html_to_text(
            "<h2>Story</h2><p>One &amp; two<br>three</p><ul><li>a</li><li>b</li></ul><img src=\"x\">",
        );
        assert_eq!(t, "Story\n\nOne & two\nthree\n\n• a\n• b");
    }
}
