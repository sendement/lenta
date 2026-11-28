//! The Look tab: a game's pictures from SteamGridDB, a row per kind
//! (cover, wide capsule, background, logo). ← → go through them, A takes
//! one (or the first tile, the game's own, back). Picks are kept per game.

use std::collections::HashMap;

use crossbeam_channel::Receiver;
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::app::App;
use crate::game::{Art, Game};
use crate::i18n::tr;
use crate::images::Kind;
use crate::sgdb::{self, ArtKind, Asset};

/// SteamGridDB's pictures for a game, per kind.
pub type Options = HashMap<ArtKind, Vec<Asset>>;

#[derive(Default)]
pub struct ArtPick {
    pub game: String,
    pub loading: Option<Receiver<Option<Options>>>,
    /// None while loading or when SteamGridDB doesn't know the game.
    pub options: Option<HashMap<ArtKind, Vec<Asset>>>,
    pub tried: bool,
    /// The tile shown per row; 0 is the game's own picture.
    pub at: [usize; 4],
    /// Tiles per line in each section, as last laid out.
    pub cols: [usize; 4],
}

fn label(k: ArtKind) -> &'static str {
    tr(match k {
        ArtKind::Portrait => "art_portrait",
        ArtKind::Landscape => "art_landscape",
        ArtKind::Hero => "art_hero",
        ArtKind::Logo => "art_logo",
    })
}

/// A tile's size in a row.
fn tile(k: ArtKind) -> egui::Vec2 {
    match k {
        ArtKind::Portrait => vec2(150.0, 225.0),
        ArtKind::Landscape => vec2(300.0, 140.0),
        ArtKind::Hero => vec2(420.0, 136.0),
        ArtKind::Logo => vec2(280.0, 114.0),
    }
}

fn current(g: &Game, k: ArtKind) -> Option<&Art> {
    match k {
        ArtKind::Portrait => g.art.portrait.as_ref(),
        ArtKind::Landscape => g.art.landscape.as_ref(),
        ArtKind::Hero => g.art.hero.as_ref(),
        ArtKind::Logo => g.art.logo.as_ref(),
    }
}

impl App {
    fn load_art_options(&mut self, g: &Game) {
        let Some(key) = sgdb::key() else { return };
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        let (steam, title) = (g.steam_appid.or(g.store_appid), g.title.clone());
        std::thread::spawn(move || {
            let ids = sgdb::game_ids(&key, steam, &title);
            let r = (!ids.is_empty()).then(|| {
                ArtKind::ALL
                    .into_iter()
                    .map(|k| (k, sgdb::assets_of(&key, &ids, k)))
                    .collect()
            });
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.art = ArtPick {
            game: g.key.clone(),
            loading: Some(rx),
            ..Default::default()
        };
    }

    fn pump_art(&mut self) {
        if let Some(rx) = &self.art.loading
            && let Ok(r) = rx.try_recv()
        {
            self.art.options = r;
            self.art.loading = None;
            self.art.tried = true;
        }
    }

    pub fn art_rows(&self) -> usize {
        if self.art.options.is_some() { 4 } else { 0 }
    }

    /// Tiles in a section: the game's own, then SteamGridDB's.
    fn tiles(&self, row: usize) -> usize {
        self.art
            .options
            .as_ref()
            .and_then(|o| o.get(&ArtKind::ALL[row]))
            .map_or(0, Vec::len)
            + 1
    }

    /// A move in the grid; returns the section now chosen, None to leave
    /// it (up past the first line).
    pub fn art_nav(&mut self, row: usize, a: crate::input::Action) -> Option<usize> {
        use crate::input::Action;
        let n = self.tiles(row);
        let cols = self.art.cols[row].max(1);
        let at = self.art.at[row].min(n - 1);
        let (line, col) = (at / cols, at % cols);
        match a {
            Action::Left => self.art.at[row] = at.saturating_sub(1),
            Action::Right => self.art.at[row] = (at + 1).min(n - 1),
            Action::Up if line > 0 => self.art.at[row] = at - cols,
            Action::Up if row > 0 => {
                // The same column on the last line of the section above.
                let pn = self.tiles(row - 1);
                let pc = self.art.cols[row - 1].max(1);
                let last = (pn - 1) / pc;
                self.art.at[row - 1] = (last * pc + col.min(pc - 1)).min(pn - 1);
                return Some(row - 1);
            }
            Action::Up => return None,
            Action::Down if at + cols < n => self.art.at[row] = at + cols,
            Action::Down if line < (n - 1) / cols => self.art.at[row] = n - 1,
            Action::Down if row + 1 < ArtKind::ALL.len() => {
                let nc = self.art.cols[row + 1].max(1);
                self.art.at[row + 1] = col.min(nc - 1).min(self.tiles(row + 1) - 1);
                return Some(row + 1);
            }
            _ => {}
        }
        Some(row)
    }

    /// A: take the tile shown in the row.
    pub fn art_take(&mut self, row: usize) {
        let Some(opts) = &self.art.options else {
            return;
        };
        let k = ArtKind::ALL[row];
        let at = self.art.at[row];
        let url = (at > 0)
            .then(|| {
                opts.get(&k)
                    .and_then(|v| v.get(at - 1))
                    .map(|a| a.url.clone())
            })
            .flatten();
        let game = self.art.game.clone();
        if let Err(e) = sgdb::set_pick(&game, k, url.as_deref()) {
            self.toast(format!("SteamGridDB: {e}"));
            return;
        }
        match url {
            Some(u) => {
                // At once on screen; the next scan keeps it.
                let mut all: Vec<&mut Game> = self
                    .games
                    .iter_mut()
                    .chain(self.steam.more.iter_mut())
                    .collect();
                if let Some(g) = all.iter_mut().find(|g| g.key == game) {
                    let art = Some(Art::Url(u));
                    match k {
                        ArtKind::Portrait => {
                            g.art.portrait = art;
                            g.art.portrait_alt = None;
                        }
                        ArtKind::Landscape => {
                            g.art.landscape = art;
                            g.art.landscape_alt = None;
                        }
                        ArtKind::Hero => g.art.hero = art,
                        ArtKind::Logo => g.art.logo = art,
                    }
                }
                self.toast(tr("art_taken"));
            }
            None => {
                // The game's own comes back with a scan.
                self.toast(tr("art_own"));
                self.rescan();
            }
        }
    }

    pub fn draw_art(
        &mut self,
        ui: &mut Ui,
        body: Rect,
        g: &Game,
        scroll: f32,
        focus_inside: bool,
        sel: usize,
    ) -> f32 {
        self.pump_art();
        if self.art.game != g.key {
            self.load_art_options(g);
        }
        let p = ui.painter_at(body);
        let (c1, c2) = self.accent;
        let Some(opts) = self.art.options.clone() else {
            let t = if self.art.loading.is_some() {
                tr("loading")
            } else {
                tr("art_none")
            };
            if self.art.loading.is_some() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(200));
            }
            p.text(
                body.left_top(),
                Align2::LEFT_TOP,
                t,
                FontId::proportional(18.0),
                Color32::from_gray(180),
            );
            return 30.0;
        };
        let mut y = body.top() - scroll;
        let mut clicked: Option<(usize, usize)> = None;
        for (row, k) in ArtKind::ALL.into_iter().enumerate() {
            let list = opts.get(&k).cloned().unwrap_or_default();
            let size = tile(k);
            let focused = focus_inside && row == sel;
            p.text(
                pos2(body.left(), y),
                Align2::LEFT_TOP,
                format!("{}  ·  {}", label(k), list.len()),
                FontId::proportional(17.0),
                if focused {
                    Color32::WHITE
                } else {
                    Color32::from_gray(190)
                },
            );
            y += 28.0;
            // A grid: as many tiles a line as fit, then the next line.
            let gap = 14.0;
            let n = list.len() + 1;
            let cols = (((body.width() + gap) / (size.x + gap)).floor() as usize).max(1);
            self.art.cols[row] = cols;
            let at = self.art.at[row].min(n - 1);
            let lines = n.div_ceil(cols);
            let grid_top = y + 6.0;
            let grid_h = lines as f32 * (size.y + gap);
            // Keep the chosen tile in view (with its section title).
            if focused && self.game.as_ref().is_some_and(|v| v.follow) {
                let line_top = grid_top + (at / cols) as f32 * (size.y + gap) + scroll - body.top();
                if let Some(v) = self.game.as_mut() {
                    let first = if at < cols {
                        line_top - 40.0
                    } else {
                        line_top - 8.0
                    };
                    if first < v.scroll {
                        v.scroll = first.max(0.0);
                    } else if line_top + size.y + 8.0 > v.scroll + body.height() {
                        v.scroll = line_top + size.y + 8.0 - body.height();
                    }
                }
            }
            let cur_url = match current(g, k) {
                Some(Art::Url(u)) => Some(u.clone()),
                _ => None,
            };
            // Wider than the page for the frame, never past its top or bottom.
            let clip = ui.painter_at(Rect::from_min_max(
                pos2(body.left() - 8.0, body.top()),
                pos2(body.right() + 8.0, body.bottom()),
            ));
            let own_in_use = cur_url
                .as_ref()
                .is_none_or(|u| !list.iter().any(|x| x.url == *u));
            for i in 0..n {
                let r = Rect::from_min_size(
                    pos2(
                        body.left() + (i % cols) as f32 * (size.x + gap),
                        grid_top + (i / cols) as f32 * (size.y + gap),
                    ),
                    size,
                );
                if r.bottom() < body.top() - 8.0 || r.top() > body.bottom() + 8.0 {
                    continue;
                }
                clip.rect_filled(r, 12.0, Color32::from_black_alpha(120));
                let picture = if i == 0 {
                    current(g, k).filter(|_| own_in_use).cloned()
                } else {
                    Some(Art::Url(list[i - 1].thumb.clone()))
                };
                if let Some(t) = picture.and_then(|a| self.images.get(&a, Kind::Fit(512))) {
                    if k == ArtKind::Logo {
                        // Logos fit inside, not cropped.
                        let s = (size.x / t.size.x).min(size.y / t.size.y) * 0.88;
                        clip.image(
                            t.handle.id(),
                            Rect::from_center_size(r.center(), t.size * s),
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    } else {
                        crate::ui::card::draw(
                            &clip,
                            &crate::ui::card::Pose::flat(r, 12.0),
                            &crate::ui::card::Fill::Image(
                                t.handle.id(),
                                crate::ui::card::cover_uv(t.size, size),
                            ),
                            Color32::WHITE,
                            None,
                        );
                    }
                }
                if i == 0 {
                    // A tag in the corner.
                    let tg = clip.layout_no_wrap(
                        tr("art_own_tile").to_string(),
                        FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                    let tag =
                        Rect::from_min_size(r.min + vec2(8.0, 8.0), tg.size() + vec2(12.0, 6.0));
                    clip.rect_filled(tag, tag.height() / 2.0, Color32::from_black_alpha(190));
                    clip.galley(tag.min + vec2(6.0, 3.0), tg, Color32::WHITE);
                }
                let in_use = if i == 0 {
                    own_in_use
                } else {
                    cur_url.as_deref() == Some(list[i - 1].url.as_str())
                };
                if in_use {
                    clip.circle_filled(r.right_top() + vec2(-14.0, 14.0), 6.0, c2);
                }
                if focused && i == at {
                    clip.rect_stroke(r, 12.0, Stroke::new(3.0, c1), StrokeKind::Outside);
                }
                let resp = ui.interact(
                    r.intersect(body),
                    ui.id().with(("art", row, i)),
                    Sense::click(),
                );
                if resp.clicked() {
                    clicked = Some((row, i));
                }
            }
            y = grid_top + grid_h + 14.0;
        }
        if let Some((row, i)) = clicked {
            if let Some(v) = self.game.as_mut() {
                v.focus = crate::ui::game_page::INSIDE;
                v.tool_sel = row;
            }
            self.art.at[row] = i;
            self.art_take(row);
        }
        y + scroll - body.top()
    }
}
