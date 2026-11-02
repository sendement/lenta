//! The system's own programs (.desktop entries) as tiles.

use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};

use super::grid::{self, Layout};
use crate::app::App;
use crate::i18n::tr;
use crate::input::Action;
use crate::sources::desktop::{App as DesktopApp, Group};

impl App {
    fn sys_filters(&self) -> Vec<Group> {
        Group::ALL
            .into_iter()
            .filter(|g| self.apps.iter().any(|a| a.group == *g))
            .collect()
    }

    fn sys_apps(&self) -> Vec<&DesktopApp> {
        let f = self.sys_filters();
        let pick = self
            .sys
            .filter
            .checked_sub(1)
            .and_then(|i| f.get(i).copied());
        self.apps
            .iter()
            .filter(|a| pick.is_none_or(|g| a.group == g))
            .collect()
    }

    pub fn system_input(&mut self, a: Action) -> bool {
        let nf = self.sys_filters().len() + 1;
        match a {
            Action::LeftTrigger | Action::RightTrigger => {
                let d = if a == Action::RightTrigger { 1 } else { nf - 1 };
                self.sys.filter = (self.sys.filter + d) % nf;
                self.sys.sel = 0;
                self.sys.scroll = 0.0;
                return false;
            }
            Action::Accept | Action::Select => {
                if let Some(app) = self.sys_apps().get(self.sys.sel).map(|a| (*a).clone()) {
                    self.launch_app(&app);
                }
                return false;
            }
            _ => {}
        }
        let n = self.sys_apps().len();
        grid::nav(&mut self.sys, n, a)
    }

    fn launch_app(&mut self, app: &DesktopApp) {
        match app.launch() {
            Ok(()) => self.toast(format!("{} {}…", tr("launching"), app.name)),
            Err(e) => self.toast(format!("{} {}: {e}", tr("launch_failed"), app.name)),
        }
    }

    pub fn draw_system(&mut self, ui: &mut Ui, rect: Rect) {
        let rect = rect.shrink2(vec2(36.0, 0.0));
        let p = ui.painter_at(rect);
        p.text(
            pos2(rect.left(), rect.top() + 34.0),
            Align2::LEFT_CENTER,
            tr("system"),
            FontId::proportional(32.0),
            Color32::WHITE,
        );
        // What the chosen program is, from its .desktop Comment.
        if let Some(app) = self
            .sys_apps()
            .get(self.sys.sel)
            .filter(|_| !self.side_focus)
        {
            let text = match &app.comment {
                Some(c) => format!("{} — {c}", app.name),
                None => app.name.clone(),
            };
            let mut job = egui::text::LayoutJob::simple_singleline(
                text,
                FontId::proportional(16.0),
                Color32::from_gray(190),
            );
            job.wrap.max_width = rect.width() * 0.55;
            job.wrap.max_rows = 1;
            let galley = p.fonts_mut(|f| f.layout_job(job));
            p.galley(
                pos2(
                    rect.right() - galley.size().x,
                    rect.top() + 34.0 - galley.size().y / 2.0,
                ),
                galley,
                Color32::WHITE,
            );
        }
        let filters = self.sys_filters();
        let mut labels = vec![format!("{} · {}", tr("all"), self.apps.len())];
        labels.extend(filters.iter().map(|g| tr(g.key()).to_string()));
        if let Some(i) = grid::chips(
            ui,
            pos2(rect.left(), rect.top() + 66.0),
            rect.right(),
            &labels,
            self.sys.filter,
            &|_| self.accent,
        ) {
            self.sys.filter = i;
            self.sys.sel = 0;
            self.sys.scroll = 0.0;
        }
        let apps: Vec<DesktopApp> = self.sys_apps().into_iter().cloned().collect();
        let view = Rect::from_min_max(
            pos2(rect.left() - 30.0, rect.top() + 116.0),
            pos2(rect.right() + 30.0, rect.bottom()),
        );
        let cell = vec2(150.0, 150.0);
        let lay = Layout::new(
            view.shrink2(vec2(30.0, 0.0)).translate(vec2(0.0, 16.0)),
            cell,
            18.0,
        );
        self.sys.cols = lay.cols;
        self.sys.sel = self.sys.sel.min(apps.len().saturating_sub(1));
        let ease = self.ease(12.0);
        grid::scroll(ui, &mut self.sys, &lay, view, apps.len(), ease);
        let (c1, _) = self.accent;
        let mut launch = None;
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(view));
        ui.set_clip_rect(view);
        for (i, app) in apps.iter().enumerate() {
            let r = lay.rect(i, self.sys.scroll);
            if r.bottom() < view.top() || r.top() > view.bottom() {
                continue;
            }
            let resp = ui.interact(
                r.intersect(view),
                ui.id().with(("sys", &app.id)),
                Sense::click(),
            );
            let selected = i == self.sys.sel && !self.side_focus;
            let p = ui.painter().clone();
            let fill = if selected {
                Color32::from_white_alpha(34)
            } else if resp.hovered() {
                Color32::from_white_alpha(22)
            } else {
                Color32::from_white_alpha(12)
            };
            p.rect_filled(r, 18.0, fill);
            if selected {
                p.rect_stroke(
                    r,
                    18.0,
                    egui::Stroke::new(2.5, c1),
                    egui::StrokeKind::Inside,
                );
            }
            let icon_r =
                Rect::from_center_size(pos2(r.center().x, r.top() + 56.0), vec2(64.0, 64.0));
            match &app.icon {
                Some(path) => {
                    egui::Image::new(format!("file://{}", path.display()))
                        .fit_to_exact_size(icon_r.size())
                        .paint_at(&ui, icon_r);
                }
                None => {
                    p.rect_filled(icon_r.shrink(6.0), 12.0, Color32::from_white_alpha(30));
                    let first = app
                        .name
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default();
                    p.text(
                        icon_r.center(),
                        Align2::CENTER_CENTER,
                        first,
                        FontId::proportional(30.0),
                        Color32::WHITE,
                    );
                }
            }
            let mut job = egui::text::LayoutJob::simple(
                app.name.clone(),
                FontId::proportional(14.0),
                Color32::from_gray(if selected { 255 } else { 210 }),
                r.width() - 16.0,
            );
            job.wrap.max_rows = 2;
            job.halign = egui::Align::Center;
            let galley = p.fonts_mut(|f| f.layout_job(job));
            p.galley(pos2(r.center().x, r.top() + 98.0), galley, Color32::WHITE);
            if resp.clicked() {
                self.side_focus = false;
                if self.sys.sel == i {
                    launch = Some(app.clone());
                } else {
                    self.sys.sel = i;
                }
            }
        }
        if let Some(a) = launch {
            self.launch_app(&a);
        }
    }
}
