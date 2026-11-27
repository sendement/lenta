//! The library: a tab per launcher on top (Steam, Heroic, Lutris), and
//! for each, shelves down the left: all, installed, favourites,
//! collections, third-party, soundtracks (those it has). Collections show
//! as cards of fanned covers; one opened shows its games under its name.
//! The last tab holds the launcher's own collections across launchers
//! (plain and dynamic): games go in by dragging a card onto the panel
//! that comes up on the left, or from the context menu (Y).

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::card::{self, Axis, Pose};
use super::grid::{self, Layout};
use super::ribbon;
use super::widgets;
use crate::app::App;
use crate::colls::{Coll as MyColl, Facts};
use crate::config::Shape;
use crate::game::{Game, Source};
use crate::i18n::tr;
use crate::input::Action;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shelf {
    All,
    Installed,
    Favorites,
    Collections,
    ThirdParty,
    Soundtracks,
    /// The launcher's own collections: all, plain, dynamic.
    MyAll,
    MyPlain,
    MyDynamic,
}

/// A tab on top: a launcher, or the launcher's own collections.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LibTab {
    Launcher(Source),
    Mine,
}

impl Shelf {
    fn key(self) -> &'static str {
        match self {
            Shelf::All => "all",
            Shelf::Installed => "shelf_installed",
            Shelf::Favorites => "shelf_favorites",
            Shelf::Collections => "shelf_collections",
            Shelf::ThirdParty => "shelf_third_party",
            Shelf::Soundtracks => "shelf_soundtracks",
            Shelf::MyAll => "all",
            Shelf::MyPlain => "coll_plain_pl",
            Shelf::MyDynamic => "coll_dynamic_pl",
        }
    }
}

#[derive(Default)]
pub struct LibView {
    /// Index into the launcher's shelves.
    pub shelf: usize,
    /// The focus is on the shelves (not the grid).
    pub side: bool,
    /// A collection opened, and where the grid was before.
    pub open: Option<usize>,
    pub back_sel: usize,
    /// One of the launcher's own collections opened.
    pub my_open: Option<u64>,
    /// A game card being dragged (its key).
    pub drag: Option<String>,
    /// Where the first tile and the drop rows were last drawn (tests).
    pub tile0: Option<Rect>,
    pub drop_rows: Vec<(Rect, Option<u64>)>,
}

/// A collection: its name and games.
struct Coll<'a> {
    name: String,
    games: Vec<&'a Game>,
}

enum Item<'a> {
    Game(&'a Game),
    Coll(usize),
    /// "New collection".
    New,
    My(u64),
    /// Own programs: add one, the prefixes.
    AddProgram,
    Prefixes,
}

/// A tile as the grid draws it (owned).
enum Tile {
    Game(Box<Game>),
    Coll(usize),
    New,
    My(u64),
    AddProgram,
    Prefixes,
}

/// A collection card's look: name, count, up to three covers, dynamic.
struct Summary {
    name: String,
    n: usize,
    covers: Vec<Game>,
    dynamic: bool,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn sorted(mut v: Vec<&Game>) -> Vec<&Game> {
    // Installed first, then by name.
    v.sort_by_key(|g| (!g.installed, g.title.to_lowercase()));
    v.dedup_by_key(|g| g.key.clone());
    v
}

impl App {
    /// Everything of a launcher the library shows.
    fn pool(&self, src: Source) -> Vec<&Game> {
        self.games
            .iter()
            .chain(
                self.steam
                    .more
                    .iter()
                    .filter(|g| !g.installed && self.cfg.show_uninstalled),
            )
            .filter(|g| g.source == src)
            .collect()
    }

    /// The launchers with something in them.
    fn lib_sources(&self) -> Vec<Source> {
        Source::ALL
            .into_iter()
            .filter(|s| !self.pool(*s).is_empty() || (*s == Source::Own && self.cfg.own))
            .collect()
    }

    /// The tabs on top: the launchers, then the own collections.
    pub fn lib_tabs(&self) -> Vec<LibTab> {
        self.lib_sources()
            .into_iter()
            .map(LibTab::Launcher)
            .chain([LibTab::Mine])
            .collect()
    }

    fn lib_tab(&self) -> LibTab {
        let t = self.lib_tabs();
        t[self.lib.filter.min(t.len() - 1)]
    }

    /// The launcher shown (for the page's colours too).
    pub(crate) fn lib_source(&self) -> Option<Source> {
        match self.lib_tab() {
            LibTab::Launcher(s) => Some(s),
            LibTab::Mine => None,
        }
    }

    /// Games in a launcher's favourites (any launcher), by key.
    fn favorite_keys(&self) -> std::collections::HashSet<String> {
        let mut out = std::collections::HashSet::new();
        let all = || self.games.iter().chain(&self.steam.more);
        if let Some(c) = self.steam.collections.iter().find(|c| c.id == "favorite") {
            let ids: std::collections::HashSet<u32> = c.apps.iter().copied().collect();
            out.extend(
                all()
                    .filter(|g| g.steam_appid.is_some_and(|i| ids.contains(&i)))
                    .map(|g| g.key.clone()),
            );
        }
        for cats in [&self.shelves.lutris, &self.shelves.heroic] {
            for (n, keys) in cats.iter() {
                if n == "favorite" {
                    out.extend(
                        all()
                            .filter(|g| {
                                keys.iter()
                                    .any(|k| crate::sources::heroic::key_matches(k, &g.key))
                            })
                            .map(|g| g.key.clone()),
                    );
                }
            }
        }
        out
    }

    /// What a rule needs to know about a game.
    pub fn facts(&self, g: &Game) -> Facts {
        Facts {
            favorite: self.favorite_keys().contains(&g.key),
            now: now(),
        }
    }

    /// A collection's games: those put in by hand (installed or not), and
    /// for a dynamic one those that fit (not installed only when shown).
    pub fn coll_games(&self, c: &MyColl) -> Vec<&Game> {
        let fav = self.favorite_keys();
        let now = now();
        sorted(
            self.games
                .iter()
                .chain(&self.steam.more)
                .filter(|g| {
                    c.games.contains(&g.key)
                        || ((g.installed || self.cfg.show_uninstalled)
                            && c.has(
                                g,
                                &Facts {
                                    favorite: fav.contains(&g.key),
                                    now,
                                },
                            ))
                })
                .collect(),
        )
    }

    pub fn save_colls(&mut self) {
        if let Err(e) = self.colls.save() {
            self.toast(format!("{}: {e}", tr("coll_save_failed")));
        }
    }

    /// Put a game in a collection (from the drag or the menu).
    fn add_to_coll(&mut self, id: u64, key: &str) {
        let title = self
            .game_by_key(key)
            .map(|g| g.title.clone())
            .unwrap_or_default();
        let Some(c) = self.colls.get_mut(id) else {
            return;
        };
        c.add(key);
        let name = c.name.clone();
        self.save_colls();
        self.toast(format!("{title} → «{name}»"));
    }

    /// A new plain collection with this game: only its name is asked.
    pub fn new_coll_with(&mut self, key: String) {
        self.type_text(
            tr("coll_new_title").into(),
            String::new(),
            move |app, name| {
                let name = if name.is_empty() {
                    format!("{} {}", tr("coll_default_name"), app.colls.list.len() + 1)
                } else {
                    name
                };
                let id = app.colls.create(name, None);
                app.add_to_coll(id, &key);
            },
        );
    }

    /// Show one of the own collections.
    pub fn open_my(&mut self, id: u64) {
        self.go(crate::app::Page::Library);
        self.lib.filter = self.lib_tabs().len() - 1;
        self.libv.shelf = 0;
        self.libv.side = false;
        self.libv.back_sel = self.lib.sel;
        self.libv.my_open = Some(id);
        self.lib.sel = 0;
        self.lib.scroll = 0.0;
    }

    pub fn ask_delete_coll(&mut self, id: u64) {
        let Some(c) = self.colls.get(id) else { return };
        let title = format!("{} «{}»?", tr("coll_delete_q"), c.name);
        self.ask(
            title,
            vec![tr("coll_delete_note").to_string()],
            tr("coll_delete_short"),
            move |app| {
                app.colls.delete(id);
                app.save_colls();
                app.cedit = None;
                if app.libv.my_open == Some(id) {
                    app.libv.my_open = None;
                }
            },
        );
    }

    /// The first game the grid shows (tests).
    #[cfg(test)]
    pub fn lib_first_key(&self) -> String {
        match self.lib_items().first() {
            Some(Item::Game(g)) => g.key.clone(),
            _ => String::new(),
        }
    }

    /// The own collections a shelf shows.
    fn my_ids(&self, shelf: Shelf) -> Vec<u64> {
        self.colls
            .list
            .iter()
            .filter(|c| match shelf {
                Shelf::MyPlain => !c.dynamic(),
                Shelf::MyDynamic => c.dynamic(),
                _ => true,
            })
            .map(|c| c.id)
            .collect()
    }

    /// A launcher's categories, favourites ("favorite") among them.
    fn categories(&self, src: Source) -> Vec<Coll<'_>> {
        let all: Vec<&Game> = self.games.iter().chain(&self.steam.more).collect();
        match src {
            Source::Steam => self
                .steam
                .collections
                .iter()
                .map(|c| {
                    let ids: std::collections::HashSet<u32> = c.apps.iter().copied().collect();
                    Coll {
                        name: if c.id == "favorite" {
                            "favorite".into()
                        } else {
                            c.name.clone()
                        },
                        games: all
                            .iter()
                            .copied()
                            .filter(|g| g.steam_appid.is_some_and(|i| ids.contains(&i)))
                            .filter(|g| g.installed || self.cfg.show_uninstalled)
                            .collect(),
                    }
                })
                .collect(),
            Source::Hydra | Source::Own => Vec::new(),
            Source::Lutris | Source::Heroic => {
                let cats = if src == Source::Lutris {
                    &self.shelves.lutris
                } else {
                    &self.shelves.heroic
                };
                cats.iter()
                    .filter(|(n, _)| !n.starts_with('.'))
                    .map(|(n, keys)| Coll {
                        name: n.clone(),
                        games: all
                            .iter()
                            .copied()
                            .filter(|g| {
                                keys.iter()
                                    .any(|k| crate::sources::heroic::key_matches(k, &g.key))
                            })
                            .collect(),
                    })
                    .collect()
            }
        }
    }

    fn collections(&self, src: Source) -> Vec<Coll<'_>> {
        self.categories(src)
            .into_iter()
            .filter(|c| c.name != "favorite" && !c.games.is_empty())
            .collect()
    }

    fn shelf_games(&self, src: Source, shelf: Shelf) -> Vec<&Game> {
        let pool = self.pool(src);
        sorted(match shelf {
            Shelf::All => pool,
            Shelf::Installed => pool.into_iter().filter(|g| g.installed).collect(),
            Shelf::Favorites => self
                .categories(src)
                .into_iter()
                .find(|c| c.name == "favorite")
                .map(|c| c.games)
                .unwrap_or_default(),
            Shelf::ThirdParty => pool
                .into_iter()
                .filter(|g| {
                    g.key.starts_with("steam-shortcut:") || g.sub.as_deref() == Some("Sideload")
                })
                .collect(),
            Shelf::Soundtracks => {
                if src == Source::Steam {
                    self.shelves.soundtracks.iter().collect()
                } else {
                    Vec::new()
                }
            }
            Shelf::Collections | Shelf::MyAll | Shelf::MyPlain | Shelf::MyDynamic => Vec::new(),
        })
    }

    /// The shelves a tab has (All always).
    fn shelves_of(&self, tab: LibTab) -> Vec<Shelf> {
        let src = match tab {
            LibTab::Launcher(s) => s,
            LibTab::Mine => {
                let mut v = vec![Shelf::MyAll];
                if self.colls.list.iter().any(|c| !c.dynamic()) {
                    v.push(Shelf::MyPlain);
                }
                if self.colls.list.iter().any(|c| c.dynamic()) {
                    v.push(Shelf::MyDynamic);
                }
                return v;
            }
        };
        let pool = self.pool(src);
        let mut v = vec![Shelf::All];
        if pool.iter().any(|g| !g.installed) {
            v.push(Shelf::Installed);
        }
        for s in [
            Shelf::Favorites,
            Shelf::Collections,
            Shelf::ThirdParty,
            Shelf::Soundtracks,
        ] {
            let some = if s == Shelf::Collections {
                !self.collections(src).is_empty()
            } else {
                !self.shelf_games(src, s).is_empty()
            };
            if some {
                v.push(s);
            }
        }
        v
    }

    fn lib_shelf(&self) -> (LibTab, Shelf) {
        let tab = self.lib_tab();
        let shelves = self.shelves_of(tab);
        (tab, shelves[self.libv.shelf.min(shelves.len() - 1)])
    }

    /// What the grid holds now.
    fn lib_items(&self) -> Vec<Item<'_>> {
        let (tab, shelf) = self.lib_shelf();
        let src = match tab {
            LibTab::Launcher(s) => s,
            LibTab::Mine => {
                if let Some(c) = self.libv.my_open.and_then(|id| self.colls.get(id)) {
                    return self.coll_games(c).into_iter().map(Item::Game).collect();
                }
                return std::iter::once(Item::New)
                    .chain(self.my_ids(shelf).into_iter().map(Item::My))
                    .collect();
            }
        };
        if shelf == Shelf::Collections {
            let colls = self.collections(src);
            return match self.libv.open.filter(|i| *i < colls.len()) {
                Some(i) => sorted(
                    colls
                        .into_iter()
                        .nth(i)
                        .map(|c| c.games)
                        .unwrap_or_default(),
                )
                .into_iter()
                .map(Item::Game)
                .collect(),
                None => (0..colls.len()).map(Item::Coll).collect(),
            };
        }
        let games = self.shelf_games(src, shelf).into_iter().map(Item::Game);
        if src == Source::Own && shelf == Shelf::All {
            return [Item::AddProgram, Item::Prefixes]
                .into_iter()
                .chain(games)
                .collect();
        }
        games.collect()
    }

    fn reset_grid(&mut self) {
        self.lib.sel = 0;
        self.lib.scroll = 0.0;
        self.libv.open = None;
        self.libv.my_open = None;
    }

    fn lib_accept(&mut self, play: bool) {
        let item = match self.lib_items().get(self.lib.sel) {
            Some(Item::Game(g)) => Some(Ok(g.key.clone())),
            Some(Item::Coll(i)) => Some(Err(*i)),
            Some(Item::New) => {
                let dynamic = self.lib_shelf().1 == Shelf::MyDynamic;
                self.edit_coll(None, None);
                if let Some(e) = &mut self.cedit {
                    e.dynamic = dynamic;
                }
                return;
            }
            Some(Item::My(id)) => {
                let id = *id;
                self.open_my(id);
                return;
            }
            Some(Item::AddProgram) => {
                self.add_program();
                return;
            }
            Some(Item::Prefixes) => {
                self.open_prefixes();
                return;
            }
            None => None,
        };
        match item {
            Some(Ok(k)) if play => self.play(&k),
            Some(Ok(k)) => self.open_game(k),
            Some(Err(i)) => {
                self.libv.back_sel = self.lib.sel;
                self.libv.open = Some(i);
                self.lib.sel = 0;
                self.lib.scroll = 0.0;
            }
            None => {}
        }
    }

    pub fn library_input(&mut self, a: Action) -> bool {
        let ns = self.lib_tabs().len();
        match a {
            Action::LeftTrigger | Action::RightTrigger => {
                let d = if a == Action::RightTrigger { 1 } else { ns - 1 };
                self.lib.filter = (self.lib.filter + d) % ns;
                self.libv.shelf = 0;
                self.reset_grid();
                return false;
            }
            _ => {}
        }
        // On the shelves.
        if self.libv.side {
            let n = self.shelves_of(self.lib_tab()).len();
            match a {
                Action::Up => {
                    self.libv.shelf = self.libv.shelf.saturating_sub(1);
                    self.reset_grid();
                }
                Action::Down => {
                    self.libv.shelf = (self.libv.shelf + 1).min(n - 1);
                    self.reset_grid();
                }
                Action::Right | Action::Accept => self.libv.side = false,
                Action::Back => return true,
                _ => {}
            }
            return false;
        }
        match a {
            Action::Accept => self.lib_accept(false),
            Action::Select => self.lib_accept(true),
            // Back: out of a collection, else to the shelves.
            Action::Actions => {
                let what = match self.lib_items().get(self.lib.sel) {
                    Some(Item::Game(g)) => Some(crate::ui::menu::MenuFor::Game(g.key.clone())),
                    Some(Item::My(id)) => Some(crate::ui::menu::MenuFor::Coll(*id)),
                    _ => None,
                };
                if let Some(w) = what {
                    self.open_menu(w, None);
                }
            }
            Action::Back => {
                if self.libv.open.take().is_some() || self.libv.my_open.take().is_some() {
                    self.lib.sel = self.libv.back_sel;
                    self.lib.follow = true;
                } else {
                    self.libv.side = true;
                }
            }
            // Left from the first column: the shelves.
            Action::Left if self.lib.sel.is_multiple_of(self.lib.cols.max(1)) => {
                self.libv.side = true
            }
            _ => {
                let n = self.lib_items().len();
                grid::nav(&mut self.lib, n, a);
            }
        }
        false
    }

    pub fn draw_library(&mut self, ui: &mut Ui, rect: Rect) {
        let rect = rect.shrink2(vec2(36.0, 0.0));
        let p = ui.painter_at(rect);
        let (c1, c2) = self.accent;
        p.text(
            pos2(rect.left(), rect.top() + 34.0),
            Align2::LEFT_CENTER,
            tr("library"),
            FontId::proportional(32.0),
            Color32::WHITE,
        );
        // The launchers on top, the own collections last.
        let tabs = self.lib_tabs();
        self.lib.filter = self.lib.filter.min(tabs.len() - 1);
        let labels: Vec<String> = tabs
            .iter()
            .map(|t| match t {
                LibTab::Launcher(s) => format!("{} · {}", s.name(), self.pool(*s).len()),
                LibTab::Mine => format!("{} · {}", tr("my_colls"), self.colls.list.len()),
            })
            .collect();
        let accent = self.accent;
        if let Some(i) = grid::chips(
            ui,
            pos2(rect.left(), rect.top() + 66.0),
            rect.right(),
            &labels,
            self.lib.filter,
            &|i| match tabs[i] {
                LibTab::Launcher(s) => widgets::source_colors(s),
                LibTab::Mine => accent,
            },
        ) {
            self.lib.filter = i;
            self.libv.shelf = 0;
            self.reset_grid();
        }
        let tab = self.lib_tab();
        // The shelves down the left.
        let shelves = self.shelves_of(tab);
        self.libv.shelf = self.libv.shelf.min(shelves.len() - 1);
        let side_w = 220.0;
        let side_top = rect.top() + 122.0;
        let mut y = side_top;
        for (i, s) in shelves.iter().enumerate() {
            let count = match (tab, s) {
                (LibTab::Mine, _) => self.my_ids(*s).len(),
                (LibTab::Launcher(src), Shelf::Collections) => self.collections(src).len(),
                (LibTab::Launcher(src), _) => self.shelf_games(src, *s).len(),
            };
            let r = Rect::from_min_size(pos2(rect.left(), y), vec2(side_w, 44.0));
            y += 50.0;
            let resp = ui.interact(r, ui.id().with(("shelf", i)), Sense::click());
            let on = i == self.libv.shelf;
            p.rect_filled(
                r,
                12.0,
                if on {
                    Color32::from_white_alpha(34)
                } else if resp.hovered() {
                    Color32::from_white_alpha(18)
                } else {
                    Color32::TRANSPARENT
                },
            );
            if on && self.libv.side && !self.side_focus {
                p.rect_stroke(r, 12.0, Stroke::new(2.5, c1), StrokeKind::Inside);
            }
            p.text(
                r.left_center() + vec2(16.0, 0.0),
                Align2::LEFT_CENTER,
                tr(s.key()),
                FontId::proportional(17.0),
                Color32::from_gray(if on { 255 } else { 190 }),
            );
            p.text(
                r.right_center() - vec2(14.0, 0.0),
                Align2::RIGHT_CENTER,
                count.to_string(),
                FontId::proportional(14.0),
                Color32::from_gray(140),
            );
            if resp.clicked() {
                self.side_focus = false;
                self.libv.shelf = i;
                self.libv.side = true;
                self.reset_grid();
            }
        }
        // The grid on the right; an open collection's name above it.
        let mut top = rect.top() + 116.0;
        let shelf = shelves[self.libv.shelf];
        // Owned summaries of the cards.
        let summary = |name: String, games: Vec<&Game>, dynamic: bool| Summary {
            name,
            n: games.len(),
            covers: games.into_iter().take(3).cloned().collect(),
            dynamic,
        };
        let launcher_colls: Vec<Summary> = match tab {
            LibTab::Launcher(src) => self
                .collections(src)
                .into_iter()
                .map(|c| summary(c.name, sorted(c.games), false))
                .collect(),
            LibTab::Mine => Vec::new(),
        };
        let my_colls: Vec<(u64, Summary)> = match tab {
            LibTab::Mine => self
                .colls
                .list
                .iter()
                .map(|c| {
                    (
                        c.id,
                        summary(c.name.clone(), self.coll_games(c), c.dynamic()),
                    )
                })
                .collect(),
            LibTab::Launcher(_) => Vec::new(),
        };
        let opened: Option<(String, usize, Option<u64>)> = match tab {
            LibTab::Launcher(_) if shelf == Shelf::Collections => self
                .libv
                .open
                .and_then(|i| launcher_colls.get(i))
                .map(|s| (s.name.clone(), s.n, None)),
            LibTab::Mine => self.libv.my_open.and_then(|id| {
                my_colls
                    .iter()
                    .find(|(i, _)| *i == id)
                    .map(|(i, s)| (s.name.clone(), s.n, Some(*i)))
            }),
            _ => None,
        };
        if let Some((cname, cn, my)) = &opened {
            let hr = Rect::from_min_size(pos2(rect.left() + side_w + 28.0, top), vec2(600.0, 40.0));
            let g = p.layout_no_wrap(
                format!("‹  {cname}  ·  {cn}"),
                FontId::proportional(22.0),
                Color32::WHITE,
            );
            let hr = Rect::from_min_size(hr.min, vec2(g.size().x + 8.0, 40.0));
            let resp = ui.interact(hr, ui.id().with("coll_back"), Sense::click());
            p.galley(
                pos2(hr.left(), hr.center().y - g.size().y / 2.0),
                g,
                Color32::WHITE,
            );
            if resp.clicked() {
                self.libv.open = None;
                self.libv.my_open = None;
                self.lib.sel = self.libv.back_sel;
            }
            // The own one: a button to change it.
            if let Some(id) = my {
                let er =
                    Rect::from_min_size(pos2(hr.right() + 18.0, hr.top() + 4.0), vec2(130.0, 32.0));
                let eresp = ui.interact(er, ui.id().with("coll_edit_btn"), Sense::click());
                p.rect_filled(
                    er,
                    16.0,
                    Color32::from_white_alpha(if eresp.hovered() { 34 } else { 18 }),
                );
                p.text(
                    er.center(),
                    Align2::CENTER_CENTER,
                    tr("coll_edit"),
                    FontId::proportional(15.0),
                    Color32::WHITE,
                );
                if eresp.clicked() {
                    self.edit_coll(Some(*id), None);
                }
            }
            top += 44.0;
        }
        let view = Rect::from_min_max(
            pos2(rect.left() + side_w + 8.0, top),
            pos2(rect.right() + 30.0, rect.bottom()),
        );
        let showing_colls =
            opened.is_none() && (shelf == Shelf::Collections || tab == LibTab::Mine);
        let cell = if showing_colls {
            vec2(300.0, 230.0)
        } else {
            match self.cfg.lib_shape {
                Shape::Portrait => vec2(160.0, 240.0),
                Shape::Landscape => vec2(260.0, 260.0 * 215.0 / 460.0),
            }
        };
        let title_h = 30.0;
        let lay = Layout::new(
            view.shrink2(vec2(20.0, 0.0)).translate(vec2(0.0, 20.0)),
            cell + vec2(0.0, title_h),
            22.0,
        );
        // Owned copies: the grid draws from them.
        let items: Vec<Tile> = self
            .lib_items()
            .into_iter()
            .map(|it| match it {
                Item::Game(g) => Tile::Game(Box::new(g.clone())),
                Item::Coll(i) => Tile::Coll(i),
                Item::New => Tile::New,
                Item::My(id) => Tile::My(id),
                Item::AddProgram => Tile::AddProgram,
                Item::Prefixes => Tile::Prefixes,
            })
            .collect();

        self.lib.cols = lay.cols;
        self.lib.sel = self.lib.sel.min(items.len().saturating_sub(1));
        let ease = self.ease(12.0);
        grid::scroll(ui, &mut self.lib, &lay, view, items.len(), ease);
        let p = ui.painter_at(view);
        let pulse = ((self.time * 2.2).sin() * 0.5 + 0.5) as f32;
        let grid_focus = !self.libv.side && !self.side_focus;
        let mut clicked = None;
        let mut menu = None;
        if items.is_empty() && opened.as_ref().is_some_and(|o| o.2.is_some()) {
            let note = p.layout(
                tr("coll_empty").to_string(),
                FontId::proportional(17.0),
                Color32::from_gray(160),
                view.width() - 80.0,
            );
            p.galley(view.min + vec2(28.0, 30.0), note, Color32::WHITE);
        }
        for (i, it) in items.iter().enumerate() {
            let cell_r = lay.rect(i, self.lib.scroll);
            if cell_r.bottom() < view.top() || cell_r.top() > view.bottom() {
                continue;
            }
            let r = Rect::from_min_size(cell_r.min, cell);
            if i == 0 {
                self.libv.tile0 = Some(r);
            }
            let selected = i == self.lib.sel && grid_focus;
            let sense = if matches!(it, Tile::Game(_)) {
                Sense::click_and_drag()
            } else {
                Sense::click()
            };
            let resp = ui.interact(r.intersect(view), ui.id().with(("lib", i)), sense);
            let dragged = matches!(it, Tile::Game(g) if self.libv.drag.as_deref() == Some(&g.key));
            match it {
                Tile::Game(g) => {
                    self.draw_game_tile(&p, r, g, selected, resp.hovered(), pulse);
                    if dragged {
                        p.rect_filled(r.expand(4.0), 12.0, Color32::from_black_alpha(120));
                    }
                    if resp.drag_started() {
                        self.libv.drag = Some(g.key.clone());
                    }
                }
                Tile::Coll(ci) => {
                    let s = &launcher_colls[*ci];
                    self.draw_coll_tile(&p, r, s, selected, resp.hovered(), (c1, c2), pulse);
                }
                Tile::My(id) => {
                    if let Some((_, s)) = my_colls.iter().find(|(i, _)| i == id) {
                        self.draw_coll_tile(&p, r, s, selected, resp.hovered(), (c1, c2), pulse);
                    }
                }
                Tile::New => self.draw_new_tile(
                    &p,
                    r,
                    selected,
                    resp.hovered(),
                    (c1, c2),
                    pulse,
                    "coll_new",
                    false,
                    false,
                ),
                Tile::AddProgram => self.draw_new_tile(
                    &p,
                    r,
                    selected,
                    resp.hovered(),
                    (c1, c2),
                    pulse,
                    "pg_add",
                    false,
                    true,
                ),
                Tile::Prefixes => self.draw_new_tile(
                    &p,
                    r,
                    selected,
                    resp.hovered(),
                    (c1, c2),
                    pulse,
                    "pg_prefixes",
                    true,
                    true,
                ),
            }
            if let Tile::Game(g) = it {
                let mut job = egui::text::LayoutJob::simple_singleline(
                    g.title.clone(),
                    FontId::proportional(14.0),
                    Color32::from_gray(if selected { 255 } else { 190 }),
                );
                job.wrap.max_width = cell.x;
                job.wrap.max_rows = 1;
                let one = p.fonts_mut(|f| f.layout_job(job));
                p.galley(
                    pos2(r.center().x - one.size().x / 2.0, r.bottom() + 8.0),
                    one,
                    Color32::WHITE,
                );
            }
            if resp.clicked() {
                clicked = Some(i);
            }
            if resp.secondary_clicked() {
                let at = resp.interact_pointer_pos();
                menu = match it {
                    Tile::Game(g) => Some((i, crate::ui::menu::MenuFor::Game(g.key.clone()), at)),
                    Tile::My(id) => Some((i, crate::ui::menu::MenuFor::Coll(*id), at)),
                    _ => None,
                };
            }
        }
        if let Some(i) = clicked {
            self.side_focus = false;
            self.libv.side = false;
            if self.lib.sel == i {
                self.lib_accept(false);
            } else {
                self.lib.sel = i;
            }
        }
        if let Some((i, what, at)) = menu {
            self.side_focus = false;
            self.libv.side = false;
            self.lib.sel = i;
            self.open_menu(what, at);
        }
        self.draw_drop(
            ui,
            Rect::from_min_max(
                pos2(rect.left(), side_top - 6.0),
                pos2(rect.left() + side_w, rect.bottom()),
            ),
        );
    }

    /// While a card is dragged: it follows the pointer, and the left
    /// column becomes the own collections to drop it on (and a new one).
    fn draw_drop(&mut self, ui: &mut Ui, col: Rect) {
        let Some(key) = self.libv.drag.clone() else {
            return;
        };
        let (down, released, at) = ui.input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
            )
        });
        if !down && !released {
            self.libv.drag = None;
            return;
        }
        let Some(g) = self.game_by_key(&key).cloned() else {
            self.libv.drag = None;
            return;
        };
        let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("lib_drag"));
        let p = ui.ctx().layer_painter(layer);
        let (c1, c2) = self.accent;
        p.rect_filled(
            col.expand2(vec2(14.0, 6.0)),
            16.0,
            Color32::from_rgb(16, 18, 26),
        );
        p.text(
            col.left_top() + vec2(8.0, 6.0),
            Align2::LEFT_TOP,
            tr("coll_drop_here"),
            FontId::proportional(15.0),
            Color32::from_gray(170),
        );
        let mut y = col.top() + 34.0;
        let mut target: Option<Option<u64>> = None;
        let rows: Vec<(Option<u64>, String, bool)> = self
            .colls
            .list
            .iter()
            .map(|c| {
                let inside = c.games.contains(&key);
                (Some(c.id), c.name.clone(), inside)
            })
            .chain([(None, tr("coll_new_dots").to_string(), false)])
            .collect();
        let row_h = ((col.bottom() - y) / rows.len() as f32).clamp(30.0, 50.0);
        self.libv.drop_rows.clear();
        for (id, name, inside) in rows {
            let r = Rect::from_min_size(pos2(col.left(), y), vec2(col.width(), row_h - 6.0));
            y += row_h;
            if r.top() > col.bottom() {
                break;
            }
            self.libv.drop_rows.push((r, id));
            let over = at.is_some_and(|a| r.contains(a));
            if over {
                widgets::gradient_pill(&p, r, 12.0, c1, c2);
                target = Some(id);
            } else {
                p.rect_filled(r, 12.0, Color32::from_white_alpha(16));
            }
            let mut job = egui::text::LayoutJob::simple_singleline(
                name,
                FontId::proportional(16.0),
                Color32::WHITE,
            );
            job.wrap.max_width = r.width() - 40.0;
            job.wrap.max_rows = 1;
            let gl = p.fonts_mut(|f| f.layout_job(job));
            p.galley(
                pos2(r.left() + 14.0, r.center().y - gl.size().y / 2.0),
                gl,
                Color32::WHITE,
            );
            if inside {
                let b =
                    Rect::from_center_size(r.right_center() - vec2(18.0, 0.0), vec2(18.0, 18.0));
                p.rect_filled(b, 5.0, Color32::from_white_alpha(60));
                crate::ui::menu::tick_mark(&p, b);
            }
        }
        // The card under the pointer.
        if let Some(a) = at {
            let size = vec2(90.0, 135.0);
            let pose = Pose::flat(Rect::from_center_size(a + vec2(56.0, 30.0), size), 10.0);
            let (fill, has_art) =
                ribbon::fill_for(&mut self.images, &g, Shape::Portrait, pose.size);
            card::shadow(&p, &pose, 1.0);
            card::draw(&p, &pose, &fill, Color32::from_white_alpha(230), None);
            if !has_art {
                ribbon::title_on(&p, &mut self.images, &g, pose.bounds(), 0.6);
            }
        }
        ui.ctx().request_repaint();
        if released {
            self.libv.drag = None;
            match target {
                Some(Some(id)) => self.add_to_coll(id, &key),
                Some(None) => self.new_coll_with(key),
                None => {}
            }
        }
    }

    /// "New collection" (and the Own tab's "Add a program", "Prefixes"): a
    /// plus, or lines, on an empty card.
    #[allow(clippy::too_many_arguments)]
    fn draw_new_tile(
        &self,
        p: &egui::Painter,
        r: Rect,
        selected: bool,
        hovered: bool,
        accent: (Color32, Color32),
        pulse: f32,
        label: &str,
        lines: bool,
        card: bool,
    ) {
        // Among games: a card's size, its name under it as theirs.
        let base = if card {
            r
        } else {
            Rect::from_min_size(r.min, vec2(r.width(), r.height() - 46.0))
        };
        p.rect_filled(
            base,
            16.0,
            Color32::from_white_alpha(if selected {
                22
            } else if hovered {
                14
            } else {
                6
            }),
        );
        p.rect_stroke(
            base.shrink(6.0),
            12.0,
            Stroke::new(1.5, Color32::from_white_alpha(50)),
            StrokeKind::Inside,
        );
        if selected {
            let pose = Pose::flat(base, 16.0);
            card::glow(
                p,
                &pose,
                accent.0,
                accent.1,
                self.cfg.glow * 0.5,
                0.75 + 0.25 * pulse,
            );
        }
        let c = base.center();
        let st = Stroke::new(4.0, Color32::from_gray(if selected { 255 } else { 200 }));
        if lines {
            // A stack of glasses, for prefixes.
            for k in -1..=1 {
                let y = c.y + k as f32 * 14.0;
                p.line_segment([pos2(c.x - 24.0, y), pos2(c.x + 24.0, y)], st);
            }
        } else {
            p.line_segment([c - vec2(22.0, 0.0), c + vec2(22.0, 0.0)], st);
            p.line_segment([c - vec2(0.0, 22.0), c + vec2(0.0, 22.0)], st);
        }
        let (at, align) = if card {
            (pos2(r.center().x, r.bottom() + 8.0), Align2::CENTER_TOP)
        } else {
            (pos2(r.left() + 4.0, base.bottom() + 12.0), Align2::LEFT_TOP)
        };
        p.text(
            at,
            align,
            tr(label),
            FontId::proportional(if card { 14.0 } else { 17.0 }),
            Color32::from_gray(if selected { 255 } else { 220 }),
        );
    }

    fn draw_game_tile(
        &mut self,
        p: &egui::Painter,
        r: Rect,
        g: &Game,
        selected: bool,
        hovered: bool,
        pulse: f32,
    ) {
        let (c1, c2) = self.accent;
        let grow = if selected {
            1.06
        } else if hovered {
            1.03
        } else {
            1.0
        };
        let pose = Pose::flat(
            Rect::from_center_size(r.center(), r.size() * grow),
            self.cfg.rounding * 0.7,
        );
        let (fill, has_art) = ribbon::fill_for(&mut self.images, g, self.cfg.lib_shape, pose.size);
        card::shadow(p, &pose, 1.0);
        if selected {
            card::glow(p, &pose, c1, c2, self.cfg.glow * 0.7, 0.75 + 0.25 * pulse);
        }
        let mut light = if selected || self.cfg.dim == 0.0 {
            1.0
        } else {
            1.0 - self.cfg.dim * 0.5
        };
        // Not installed: faded, with a mark.
        if !g.installed {
            light *= 0.55;
        }
        card::draw(
            p,
            &pose,
            &fill,
            Color32::from_gray((light * 255.0) as u8),
            None,
        );
        if !g.installed {
            let b = pose.bounds();
            let mark = p.layout_no_wrap(
                tr("not_installed_one").to_string(),
                FontId::proportional(12.0),
                Color32::WHITE,
            );
            let mr = Rect::from_min_size(
                pos2(b.left() + 8.0, b.bottom() - 8.0 - mark.size().y - 8.0),
                mark.size() + vec2(16.0, 8.0),
            );
            p.rect_filled(mr, mr.height() / 2.0, Color32::from_black_alpha(170));
            p.galley(mr.min + vec2(8.0, 4.0), mark, Color32::WHITE);
        }
        if !has_art {
            ribbon::title_on(p, &mut self.images, g, pose.bounds(), 1.0);
        }
    }

    /// A collection: up to three covers fanned out, its name and count.
    #[allow(clippy::too_many_arguments)]
    fn draw_coll_tile(
        &mut self,
        p: &egui::Painter,
        r: Rect,
        s: &Summary,
        selected: bool,
        hovered: bool,
        accent: (Color32, Color32),
        pulse: f32,
    ) {
        let base = Rect::from_min_size(r.min, vec2(r.width(), r.height() - 46.0));
        p.rect_filled(
            base,
            16.0,
            Color32::from_white_alpha(if selected {
                26
            } else if hovered {
                18
            } else {
                10
            }),
        );
        if selected {
            let pose = Pose::flat(base, 16.0);
            card::glow(
                p,
                &pose,
                accent.0,
                accent.1,
                self.cfg.glow * 0.5,
                0.75 + 0.25 * pulse,
            );
        }
        // Back to front: the side ones turned away, the first in front.
        let h = base.height() * 0.84;
        let size = vec2(h * 2.0 / 3.0, h);
        let order: [(usize, f32, f32); 3] = [(2, 0.78, 0.55), (1, -0.78, -0.55), (0, 0.0, 0.0)];
        for (idx, dx, angle) in order {
            let Some(g) = s.covers.get(idx) else { continue };
            let center = base.center() + vec2(dx * size.x * 0.75, 0.0);
            let scale = if idx == 0 { 1.0 } else { 0.9 };
            let pose = Pose {
                center,
                size: size * scale,
                angle,
                axis: Axis::Upright,
                radius: 8.0,
            };
            let (fill, has_art) = ribbon::fill_for(&mut self.images, g, Shape::Portrait, pose.size);
            card::shadow(p, &pose, 0.8);
            let light = if idx == 0 { 255 } else { 190 };
            card::draw(p, &pose, &fill, Color32::from_gray(light), None);
            if !has_art {
                ribbon::title_on(p, &mut self.images, g, pose.bounds(), 0.8);
            }
        }
        let mut job = egui::text::LayoutJob::simple_singleline(
            s.name.clone(),
            FontId::proportional(17.0),
            Color32::from_gray(if selected { 255 } else { 220 }),
        );
        job.wrap.max_width = r.width() - 60.0;
        job.wrap.max_rows = 1;
        let ng = p.fonts_mut(|f| f.layout_job(job));
        p.galley(
            pos2(r.left() + 4.0, base.bottom() + 12.0),
            ng,
            Color32::WHITE,
        );
        // A dynamic one: a mark in its corner.
        if s.dynamic {
            widgets::chip(
                p,
                base.left_top() + vec2(10.0, 10.0),
                tr("coll_dynamic"),
                (accent.0.gamma_multiply(0.85), accent.1.gamma_multiply(0.85)),
                12.0,
            );
        }
        p.text(
            pos2(r.right() - 4.0, base.bottom() + 14.0),
            Align2::RIGHT_TOP,
            format!("{}", s.n),
            FontId::proportional(14.0),
            Color32::from_gray(150),
        );
    }
}
