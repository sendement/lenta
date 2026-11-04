//! Typing a short text with the pad: Polka's on-screen keyboard (its
//! layouts, keys and typing rules from `keyboard.rs`), drawn in the
//! launcher's look. A real keyboard types into it too.

use egui::{
    Align2, Color32, FontId, Id, LayerId, Order, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::app::App;
use crate::i18n::tr;
use crate::input::Action;
use crate::keyboard::{self, OskKey, Slot};

/// What happens with the text on OK.
type Done = Box<dyn FnOnce(&mut App, String)>;

pub struct Osk {
    pub title: String,
    pub text: Vec<char>,
    pub caret: usize,
    row: usize,
    col: usize,
    layout: usize,
    shift: bool,
    caps: bool,
    /// The character typed just before (for accents, Hangul, kana).
    prev: Option<char>,
    /// Runs with the text on OK.
    done: Option<Done>,
}

impl Osk {
    fn key_at(&self, row: usize, col: usize) -> OskKey {
        match keyboard::row(row).get(col).map(|(s, _)| *s) {
            Some(Slot::Ch(i)) => OskKey::Char(
                keyboard::char_at(self.layout, row, i, self.shift, self.caps).unwrap_or(' '),
            ),
            Some(Slot::Key(k)) => k,
            None => OskKey::Ok,
        }
    }

    fn nav(&mut self, a: Action) {
        let rows = keyboard::ROWS;
        let len = |r: usize| keyboard::row(r).len();
        match a {
            Action::Up | Action::Down => {
                // The key under the same spot, as on a real keyboard.
                let x = keyboard::center(self.row, self.col);
                self.row = if a == Action::Up {
                    (self.row + rows - 1) % rows
                } else {
                    (self.row + 1) % rows
                };
                self.col = keyboard::nearest(self.row, x);
            }
            Action::Left => self.col = (self.col + len(self.row) - 1) % len(self.row),
            Action::Right => self.col = (self.col + 1) % len(self.row),
            _ => {}
        }
    }

    fn insert(&mut self, s: &str) {
        for c in s.chars() {
            if self.text.len() >= 80 {
                break;
            }
            self.text.insert(self.caret, c);
            self.caret += 1;
        }
    }

    fn backspace(&mut self) {
        if self.caret > 0 {
            self.caret -= 1;
            self.text.remove(self.caret);
        }
    }

    /// A key pressed (on the screen or by the pad); true on OK.
    fn press(&mut self, key: OskKey) -> bool {
        let typed = match key {
            OskKey::Char(c) => Some(c),
            OskKey::Space => Some(' '),
            _ => None,
        };
        let composed =
            typed.and_then(|c| self.prev.and_then(|p| keyboard::compose(self.layout, p, c)));
        match key {
            OskKey::Shift => self.shift = !self.shift,
            OskKey::Caps => self.caps = !self.caps,
            OskKey::Lang => {
                self.layout = keyboard::next_layout(self.layout);
                keyboard::set_last_layout(self.layout);
                self.prev = None;
            }
            OskKey::Char(_) | OskKey::Space => {
                let c = typed.unwrap_or(' ');
                let before = self
                    .caret
                    .checked_sub(1)
                    .and_then(|i| self.text.get(i))
                    .copied();
                match composed.filter(|_| before == self.prev) {
                    Some(r) => {
                        self.backspace();
                        self.insert(&r);
                        self.prev = r.chars().last();
                    }
                    None => {
                        self.insert(&c.to_string());
                        self.prev = Some(c);
                    }
                }
                self.shift = false;
            }
            OskKey::Backspace => {
                self.backspace();
                self.prev = None;
            }
            OskKey::Left => self.caret = self.caret.saturating_sub(1),
            OskKey::Right => self.caret = (self.caret + 1).min(self.text.len()),
            OskKey::Up => self.caret = 0,
            OskKey::Down => self.caret = self.text.len(),
            OskKey::Enter | OskKey::Ok => return true,
            // Nothing to send them to here.
            OskKey::Tab | OskKey::Ctrl | OskKey::Alt | OskKey::Paste => {}
        }
        if matches!(
            key,
            OskKey::Left | OskKey::Right | OskKey::Up | OskKey::Down
        ) {
            self.prev = None;
        }
        false
    }
}

/// Keys that do nothing here: shown dimmed.
fn usable(k: OskKey) -> bool {
    !matches!(k, OskKey::Ctrl | OskKey::Alt | OskKey::Tab | OskKey::Paste)
}

impl App {
    /// Ask for a text; `done` gets it (trimmed, maybe empty).
    pub fn type_text(
        &mut self,
        title: String,
        text: String,
        done: impl FnOnce(&mut App, String) + 'static,
    ) {
        let text: Vec<char> = text.chars().collect();
        self.osk = Some(Osk {
            title,
            caret: text.len(),
            text,
            // On "q" (or its layout's first letter).
            row: 1,
            col: 1,
            layout: keyboard::last_layout(),
            shift: false,
            caps: false,
            prev: None,
            done: Some(Box::new(done)),
        });
    }

    fn osk_finish(&mut self) {
        if let Some(mut o) = self.osk.take()
            && let Some(f) = o.done.take()
        {
            let s: String = o.text.iter().collect();
            f(self, s.trim().to_string());
        }
    }

    fn osk_key(&mut self, key: OskKey) {
        let Some(o) = &mut self.osk else { return };
        if o.press(key) {
            self.osk_finish();
        }
    }

    /// The pad, as in Polka: ✕ the key, □ erase, △ space, L1/R1 the caret,
    /// L2 Shift, View the layout, Menu or R2 done, ○ cancel.
    pub fn osk_input(&mut self, a: Action) {
        let Some(o) = &mut self.osk else { return };
        match a {
            Action::Up | Action::Down | Action::Left | Action::Right => o.nav(a),
            Action::Accept => {
                let k = o.key_at(o.row, o.col);
                self.osk_key(k);
            }
            Action::Back => self.osk = None,
            Action::Select => self.osk_key(OskKey::Backspace),
            Action::Actions => self.osk_key(OskKey::Space),
            Action::PrevTab => self.osk_key(OskKey::Left),
            Action::NextTab => self.osk_key(OskKey::Right),
            Action::LeftTrigger => self.osk_key(OskKey::Shift),
            Action::View => self.osk_key(OskKey::Lang),
            Action::Menu | Action::RightTrigger => self.osk_key(OskKey::Ok),
            _ => {}
        }
    }

    /// A real keyboard: what's typed, pasted, Enter, Esc, Backspace,
    /// the arrows (the caret), Home/End.
    pub fn osk_keys(&mut self, ui: &Ui) {
        let events = ui.input(|i| i.events.clone());
        for e in events {
            let Some(o) = &mut self.osk else { return };
            match e {
                egui::Event::Text(t) => {
                    let t: String = t.chars().filter(|c| !c.is_control()).collect();
                    o.prev = None;
                    o.insert(&t);
                }
                egui::Event::Paste(t) => {
                    o.prev = None;
                    let line = t.split(['\r', '\n', '\t']).filter(|l| !l.is_empty());
                    o.insert(&line.collect::<Vec<_>>().join(" "));
                }
                egui::Event::Key {
                    key, pressed: true, ..
                } => {
                    use egui::Key as K;
                    let k = match key {
                        K::Enter => OskKey::Ok,
                        K::Escape => {
                            self.osk = None;
                            return;
                        }
                        K::Backspace => OskKey::Backspace,
                        K::Delete => {
                            if o.caret < o.text.len() {
                                o.text.remove(o.caret);
                            }
                            continue;
                        }
                        K::ArrowLeft => OskKey::Left,
                        K::ArrowRight => OskKey::Right,
                        K::Home => OskKey::Up,
                        K::End => OskKey::Down,
                        _ => continue,
                    };
                    self.osk_key(k);
                }
                _ => {}
            }
        }
    }

    pub fn draw_osk(&mut self, ui: &mut Ui, area: Rect) {
        let Some(o) = &self.osk else { return };
        let layer = LayerId::new(Order::Tooltip, Id::new("osk"));
        let p = ui.ctx().layer_painter(layer);
        p.rect_filled(area, 0.0, Color32::from_black_alpha(200));
        let (c1, c2) = self.accent;
        // Fifteen key widths a row, fitted to the window.
        let unit = ((area.width() - 120.0) / 15.0).min(62.0);
        let key_h = unit * 0.86;
        let gap = 6.0;
        let kb_w = 15.0 * unit;
        let w = kb_w + 64.0;
        let rows_h = keyboard::ROWS as f32 * (key_h + gap) - gap;
        let h = 28.0 + 30.0 + 16.0 + 56.0 + 22.0 + rows_h + 28.0;
        let card = Rect::from_center_size(area.center(), vec2(w, h));
        p.rect_filled(card, 22.0, Color32::from_rgb(24, 27, 36));
        let mut y = card.top() + 28.0;
        p.text(
            pos2(card.left() + 32.0, y),
            Align2::LEFT_TOP,
            &o.title,
            FontId::proportional(22.0),
            Color32::WHITE,
        );
        p.text(
            pos2(card.right() - 32.0, y + 4.0),
            Align2::RIGHT_TOP,
            keyboard::LAYOUTS[o.layout].name,
            FontId::proportional(17.0),
            c1,
        );
        y += 30.0 + 16.0;
        // The field, the caret blinking where it is.
        let field = Rect::from_min_size(pos2(card.left() + 32.0, y), vec2(kb_w, 56.0));
        p.rect_filled(field, 14.0, Color32::from_white_alpha(14));
        p.rect_stroke(field, 14.0, Stroke::new(2.0, c1), StrokeKind::Inside);
        let font = FontId::proportional(22.0);
        let all: String = o.text.iter().collect();
        let head: String = o.text[..o.caret].iter().collect();
        let g = p.layout_no_wrap(all, font.clone(), Color32::WHITE);
        let caret_x = p.layout_no_wrap(head, font, Color32::WHITE).size().x;
        let room = field.width() - 40.0;
        let shift_x = (caret_x - room).max(0.0);
        let tx = field.left() + 18.0 - shift_x;
        let clip = p.with_clip_rect(field.shrink(4.0));
        clip.galley(
            pos2(tx, field.center().y - g.size().y / 2.0),
            g,
            Color32::WHITE,
        );
        if (self.time * 1.6).fract() < 0.6 {
            let cx = tx + caret_x + 1.0;
            p.line_segment(
                [
                    pos2(cx, field.top() + 14.0),
                    pos2(cx, field.bottom() - 14.0),
                ],
                Stroke::new(2.0, Color32::WHITE),
            );
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
        y = field.bottom() + 22.0;
        let top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(area));
        let _veil = top.interact(area, Id::new("osk_veil"), Sense::click());
        let mut pressed = None;
        for r in 0..keyboard::ROWS {
            let mut x = field.left();
            for (col, (slot, kw)) in keyboard::row(r).into_iter().enumerate() {
                let kr = Rect::from_min_size(pos2(x + gap / 2.0, y), vec2(kw * unit - gap, key_h));
                x += kw * unit;
                let key = o.key_at(r, col);
                let resp = top.interact(kr, Id::new(("osk_key", r, col)), Sense::click());
                let cur = r == o.row && col == o.col;
                let lit = match key {
                    OskKey::Shift => o.shift,
                    OskKey::Caps => o.caps,
                    _ => false,
                };
                let special = matches!(slot, Slot::Key(_));
                if key == OskKey::Ok {
                    super::widgets::gradient_pill(&p, kr, 10.0, c1, c2);
                } else {
                    let a = if lit {
                        48
                    } else if resp.hovered() {
                        34
                    } else if special {
                        12
                    } else {
                        20
                    };
                    p.rect_filled(kr, 10.0, Color32::from_white_alpha(a));
                }
                if cur {
                    p.rect_stroke(kr, 10.0, Stroke::new(2.5, c1), StrokeKind::Outside);
                }
                let color = if usable(key) {
                    Color32::WHITE
                } else {
                    Color32::from_gray(90)
                };
                let label: String = match key {
                    OskKey::Char(c) => c.to_string(),
                    OskKey::Backspace => "⌫".into(),
                    OskKey::Tab => "Tab".into(),
                    OskKey::Caps
                        if keyboard::LAYOUTS[o.layout].script == keyboard::Script::Kana =>
                    {
                        "カナ".into()
                    }
                    OskKey::Caps => "Caps".into(),
                    OskKey::Enter => "Enter ⏎".into(),
                    OskKey::Shift => "⇧ Shift".into(),
                    OskKey::Ctrl => "Ctrl".into(),
                    OskKey::Alt => "Alt".into(),
                    OskKey::Lang => format!(
                        "⇄ {}",
                        keyboard::LAYOUTS[keyboard::next_layout(o.layout)].name
                    ),
                    OskKey::Space => tr("osk_space").into(),
                    OskKey::Left => "←".into(),
                    OskKey::Up => "↑".into(),
                    OskKey::Down => "↓".into(),
                    OskKey::Right => "→".into(),
                    OskKey::Paste => tr("osk_paste").into(),
                    OskKey::Ok => tr("osk_done").into(),
                };
                let mut size = if special { 15.0 } else { 20.0 };
                let room = kr.width() - 14.0;
                let lw = p
                    .layout_no_wrap(label.clone(), FontId::proportional(size), color)
                    .size()
                    .x;
                if lw > room {
                    size = (size * room / lw).max(9.0);
                }
                p.text(
                    kr.center(),
                    Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(size),
                    color,
                );
                // The other symbol on the key, small in its corner.
                if let Slot::Ch(i) = slot
                    && let Some(alt) = keyboard::alt_at(o.layout, r, i, o.shift)
                {
                    p.text(
                        kr.min + vec2(6.0, 4.0),
                        Align2::LEFT_TOP,
                        alt.to_string(),
                        FontId::proportional(11.0),
                        Color32::from_gray(140),
                    );
                }
                if resp.clicked() {
                    pressed = Some((r, col, key));
                }
            }
            y += key_h + gap;
        }
        if let Some((r, col, key)) = pressed {
            if let Some(o) = &mut self.osk {
                o.row = r;
                o.col = col;
            }
            self.osk_key(key);
        }
    }
}
