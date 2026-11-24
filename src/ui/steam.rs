//! What the Steam account and client add: collections, the owned games
//! that aren't installed (after signing in), the sign-in window with its
//! QR code, and choosing a game's compatibility tool (Proton).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};

use super::widgets;
use crate::account::{self, Account, Poll, QrLogin};
use crate::app::App;
use crate::config::Config;
use crate::game::Game;
use crate::i18n::tr;
use crate::input::Action;
use crate::sources::steamplay::{self, Collection, Tool};

#[derive(Clone, Default)]
pub struct SteamData {
    pub collections: Vec<Collection>,
    pub tools: Vec<Tool>,
    /// Chosen tool per game; 0 is the default for all.
    pub mapping: HashMap<u32, String>,
    pub account: Option<Account>,
    /// For the library only: owned games that aren't installed, and
    /// installed ones in the hidden collection.
    pub more: Vec<Game>,
    /// Tool changes waiting for Steam to close (it rewrites its config
    /// when it quits).
    pub pending: Vec<(u32, Option<String>)>,
    /// Games whose Steam launch options get (true) or lose lenta's start
    /// script, waiting for Steam to close: appid, game key.
    pub hooks: Vec<crate::launch::SteamWrite>,
    /// Steam being restarted to apply them: the outcome comes here.
    pub restart: Option<Receiver<Result<(), String>>>,
    /// Why the account's games couldn't be read.
    pub error: Option<String>,
}

impl SteamData {
    /// What's on disk: collections, tools, the tool per game, the account.
    pub fn local(cfg: &Config) -> SteamData {
        if !cfg.steam {
            return SteamData::default();
        }
        SteamData {
            collections: steamplay::collections(),
            tools: steamplay::tools(),
            mapping: steamplay::mapping(),
            account: Account::load(),
            ..Default::default()
        }
    }

    /// With the account: the owned games not installed, and play time
    /// counted on every device for the installed ones.
    pub fn online(&mut self, cfg: &Config, games: &mut [Game]) {
        let Some(a) = self.account.clone() else {
            return;
        };
        let owned = match account::owned(&a) {
            Ok(o) => o,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let by_id: HashMap<u32, &account::Owned> = owned.iter().map(|o| (o.appid, o)).collect();
        for g in games.iter_mut() {
            if let Some(o) = g.steam_appid.and_then(|id| by_id.get(&id)) {
                g.playtime_min = g.playtime_min.max(Some(o.playtime_min).filter(|&m| m > 0));
                g.last_played = g.last_played.max(Some(o.last_played).filter(|&t| t > 0));
            }
        }
        if !cfg.show_uninstalled {
            return;
        }
        let have: HashSet<u32> = games.iter().filter_map(|g| g.steam_appid).collect();
        let rest: Vec<account::Owned> = owned
            .into_iter()
            .filter(|o| !have.contains(&o.appid))
            .collect();
        let ids: Vec<u32> = rest.iter().map(|o| o.appid).collect();
        crate::sources::web::prefetch_assets(&ids);
        let mut more = crate::sources::steam::uninstalled(&rest);
        crate::sources::web::enrich_with(&mut more, false);
        self.more = more;
    }

    pub fn hidden(&self) -> HashSet<u32> {
        self.collections
            .iter()
            .find(|c| c.id == "hidden")
            .map(|c| c.apps.iter().copied().collect())
            .unwrap_or_default()
    }

    /// The tool a game runs with: its own, else the default ("" = none).
    pub fn tool_of(&self, id: u32) -> Option<&str> {
        self.mapping.get(&id).map(String::as_str)
    }

    pub fn display(&self, name: &str) -> String {
        self.tools
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.display.clone())
            .unwrap_or_else(|| name.to_string())
    }
}

// ---------------------------------------------------------------- sign-in

enum LoginMsg {
    Code(String),
    Scanned,
    Done(Account),
    Failed(String),
}

pub struct Login {
    rx: Receiver<LoginMsg>,
    stop: Arc<AtomicBool>,
    code: Option<qrcode::QrCode>,
    scanned: bool,
    error: Option<String>,
    started: Instant,
}

impl Login {
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn err_text(e: &str) -> String {
    match e {
        "expired" => tr("login_expired").to_string(),
        "rate_limit" => tr("login_rate").to_string(),
        "offline" => tr("login_offline").to_string(),
        e => format!("{}: {e}", tr("login_failed")),
    }
}

impl App {
    pub fn start_login(&mut self) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, ctx) = (stop.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let send = |m: LoginMsg| {
                let _ = tx.send(m);
                ctx.request_repaint();
            };
            let mut q = match QrLogin::start() {
                Ok(q) => q,
                Err(e) => return send(LoginMsg::Failed(e)),
            };
            send(LoginMsg::Code(q.url.clone()));
            // A code lasts a few minutes; Steam hands over new ones.
            while !stop2.load(Ordering::Relaxed) {
                std::thread::sleep(q.interval.min(Duration::from_secs(5)));
                if stop2.load(Ordering::Relaxed) {
                    return;
                }
                match q.poll() {
                    Ok(Poll::Waiting) => {}
                    Ok(Poll::Scanned) => send(LoginMsg::Scanned),
                    Ok(Poll::NewCode(u)) => send(LoginMsg::Code(u)),
                    Ok(Poll::Done(a)) => return send(LoginMsg::Done(a)),
                    Err(e) => return send(LoginMsg::Failed(e)),
                }
            }
        });
        self.login = Some(Login {
            rx,
            stop,
            code: None,
            scanned: false,
            error: None,
            started: Instant::now(),
        });
    }

    pub fn sign_out(&mut self) {
        Account::forget();
        self.steam.account = None;
        self.steam
            .more
            .retain(|g| g.installed || g.source != crate::game::Source::Steam);
        self.toast(tr("signed_out"));
    }

    fn pump_login(&mut self) {
        let Some(l) = self.login.as_mut() else { return };
        let mut done = None;
        while let Ok(m) = l.rx.try_recv() {
            match m {
                LoginMsg::Code(u) => {
                    l.code = qrcode::QrCode::new(u.as_bytes()).ok();
                    l.scanned = false;
                }
                LoginMsg::Scanned => l.scanned = true,
                LoginMsg::Failed(e) => l.error = Some(err_text(&e)),
                LoginMsg::Done(a) => done = Some(a),
            }
        }
        if let Some(a) = done {
            self.login = None;
            self.toast(format!("{} {}", tr("signed_in"), a.name));
            self.steam.account = Some(a);
            self.rescan();
        }
    }

    /// Keys while the sign-in window is up: Back closes it, A tries again
    /// after an error.
    pub fn login_input(&mut self, a: Action) {
        match a {
            Action::Back => self.login = None,
            Action::Accept if self.login.as_ref().is_some_and(|l| l.error.is_some()) => {
                self.start_login()
            }
            _ => {}
        }
    }

    pub fn draw_login(&mut self, ui: &mut Ui, full: Rect) {
        self.pump_login();
        let Some(l) = &self.login else { return };
        let p = ui.painter_at(full);
        p.rect_filled(full, 0.0, Color32::from_black_alpha(215));
        // Clicks don't reach what's under it.
        let resp = ui.interact(full, ui.id().with("login_veil"), Sense::click());
        let card = Rect::from_center_size(full.center(), vec2(760.0, 470.0));
        p.rect_filled(card, 22.0, Color32::from_rgb(22, 25, 34));
        let (c1, c2) = widgets::source_colors(crate::game::Source::Steam);
        widgets::gradient_pill(
            &p,
            Rect::from_min_size(card.min + vec2(40.0, 40.0), vec2(56.0, 6.0)),
            3.0,
            c1,
            c2,
        );
        p.text(
            card.min + vec2(40.0, 62.0),
            Align2::LEFT_TOP,
            tr("login_title"),
            FontId::proportional(28.0),
            Color32::WHITE,
        );
        let text_w = card.width() - 40.0 - 300.0 - 40.0 - 32.0;
        let status = if let Some(e) = &l.error {
            e.clone()
        } else if l.scanned {
            tr("login_confirm").to_string()
        } else if l.code.is_some() {
            tr("login_scan").to_string()
        } else {
            tr("loading").to_string()
        };
        let g = p.layout(
            status,
            FontId::proportional(18.0),
            Color32::from_gray(210),
            text_w,
        );
        p.galley(card.min + vec2(40.0, 120.0), g, Color32::WHITE);
        let g = p.layout(
            tr("login_note").to_string(),
            FontId::proportional(14.0),
            Color32::from_gray(140),
            text_w,
        );
        p.galley(
            pos2(card.left() + 40.0, card.bottom() - 40.0 - g.size().y),
            g,
            Color32::WHITE,
        );
        // The code, black on white with its quiet zone.
        let box_r = Rect::from_min_size(
            pos2(card.right() - 40.0 - 300.0, card.top() + 85.0),
            vec2(300.0, 300.0),
        );
        match &l.code {
            Some(code) if l.error.is_none() => {
                p.rect_filled(box_r, 14.0, Color32::WHITE);
                let n = code.width();
                let cell = (box_r.width() - 32.0) / n as f32;
                let origin = box_r.center() - vec2(cell * n as f32, cell * n as f32) / 2.0;
                let colors = code.to_colors();
                for y in 0..n {
                    for x in 0..n {
                        if colors[y * n + x] == qrcode::Color::Dark {
                            p.rect_filled(
                                Rect::from_min_size(
                                    origin + vec2(x as f32 * cell, y as f32 * cell),
                                    vec2(cell + 0.5, cell + 0.5),
                                ),
                                0.0,
                                Color32::BLACK,
                            );
                        }
                    }
                }
                if l.scanned {
                    p.rect_filled(box_r, 14.0, Color32::from_black_alpha(170));
                    widgets::icon(
                        &p,
                        box_r.center(),
                        64.0,
                        widgets::Icon::Trophy,
                        Color32::WHITE,
                    );
                }
            }
            _ => {
                p.rect_filled(box_r, 14.0, Color32::from_white_alpha(18));
                if l.error.is_none() {
                    let t = l.started.elapsed().as_secs_f32();
                    for i in 0..3 {
                        let a = ((t * 3.0 - i as f32 * 0.6).sin() * 0.5 + 0.5) * 200.0;
                        p.circle_filled(
                            box_r.center() + vec2((i as f32 - 1.0) * 26.0, 0.0),
                            7.0,
                            Color32::from_white_alpha(a as u8 + 40),
                        );
                    }
                    ui.ctx().request_repaint();
                }
            }
        }
        if resp.clicked()
            && let Some(pt) = resp.interact_pointer_pos()
            && !card.contains(pt)
        {
            self.login = None;
        }
        // The window waits on the network: look again now and then.
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }

    // ------------------------------------------------------ Steam Play

    /// The tools to pick from for a game: "Default (…)" first.
    pub fn tool_choices(&self) -> Vec<(Option<String>, String)> {
        let default = self
            .steam
            .tool_of(0)
            .map(|n| self.steam.display(n))
            .unwrap_or_else(|| tr("tool_none").to_string());
        let mut v = vec![(None, format!("{} ({default})", tr("tool_default")))];
        v.extend(
            self.steam
                .tools
                .iter()
                .map(|t| (Some(t.name.clone()), t.display.clone())),
        );
        v
    }

    /// Choose a game's tool: written at once while Steam is closed, else
    /// kept until it is (or until it's restarted from here).
    pub fn choose_tool(&mut self, id: u32, tool: Option<String>) {
        if self.steam.mapping.get(&id) == tool.as_ref() && !self.has_pending(id) {
            return;
        }
        self.steam.pending.retain(|(i, _)| *i != id);
        self.steam.pending.push((id, tool.clone()));
        match &tool {
            Some(t) => self.steam.mapping.insert(id, t.clone()),
            None => self.steam.mapping.remove(&id),
        };
        if !self.apply_pending_tools(true) {
            self.toast(tr("tool_pending"));
        }
    }

    pub fn has_pending(&self, id: u32) -> bool {
        self.steam.pending.iter().any(|(i, _)| *i == id)
    }

    /// Write the waiting changes if Steam is closed; true when written.
    pub fn apply_pending_tools(&mut self, say: bool) -> bool {
        if (self.steam.pending.is_empty() && self.steam.hooks.is_empty())
            || steamplay::steam_running()
        {
            return false;
        }
        let r = steamplay::set_tools(&self.steam.pending)
            .and_then(|()| crate::launch::write_steam(&self.steam.hooks));
        match r {
            Ok(()) => {
                self.steam.pending.clear();
                let hooks = std::mem::take(&mut self.steam.hooks);
                self.steam_written(&hooks);
                if say {
                    self.toast(tr("tool_saved"));
                }
                true
            }
            Err(e) => {
                self.toast(format!("{}: {e}", tr("tool_failed")));
                false
            }
        }
    }

    /// Close Steam, write the changes, start it again (in the tray).
    pub fn restart_steam(&mut self) {
        if self.steam.restart.is_some() {
            return;
        }
        let pending = self.steam.pending.clone();
        let hooks = self.steam.hooks.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let r = (|| {
                crate::game::run_first(&crate::sources::steam::steam_run(&["-shutdown"]))?;
                let t = Instant::now();
                while steamplay::steam_running() {
                    if t.elapsed() > Duration::from_secs(60) {
                        return Err("Steam did not close".to_string());
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                // Let it finish writing its files.
                std::thread::sleep(Duration::from_secs(1));
                steamplay::set_tools(&pending)?;
                crate::launch::write_steam(&hooks)?;
                crate::game::run_first(&crate::sources::steam::steam_run(&["-silent"]))
            })();
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.steam.restart = Some(rx);
        self.toast(tr("steam_restarting"));
    }

    pub fn pump_restart(&mut self) {
        let Some(r) = self
            .steam
            .restart
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        else {
            return;
        };
        self.steam.restart = None;
        match r {
            Ok(()) => {
                self.steam.pending.clear();
                let hooks = std::mem::take(&mut self.steam.hooks);
                self.steam_written(&hooks);
                self.toast(tr("tool_saved"));
            }
            Err(e) => self.toast(format!("{}: {e}", tr("tool_failed"))),
        }
    }
}
