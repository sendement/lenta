//! Lenta's own achievement banner: a small layer-shell surface on the
//! overlay layer (over fullscreen games), top middle, that lets clicks
//! through, slides in, stays a few seconds and goes. Its own process
//! (`lenta --banner <json>`): eframe/winit can't make layer surfaces, so
//! this is a small Wayland + EGL + egui backend after kadr's overlay.
//! Exit code 0 when shown, 3 when this desktop can't (no layer-shell, no
//! EGL): the caller falls back to a desktop notification.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Rect, Stroke, pos2, vec2};
use khronos_egl as egl;
use serde::{Deserialize, Serialize};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, registry_handlers,
};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_region, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};

use crate::trophy::{Tally, Tier};

/// What one banner tells.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Card {
    pub game: String,
    pub name: String,
    pub desc: String,
    pub icon: Option<String>,
    /// The game's tally with this one.
    pub got: u32,
    pub total: u32,
    /// The grade this one reached, when it's a new one.
    pub new_tier: Option<String>,
}

const W: u32 = 520;
const H: u32 = 112;
const IN: f32 = 0.35;
const STAY: f32 = 8.0;
const OUT: f32 = 0.45;

struct Gl {
    egl: egl::Instance<egl::Static>,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    gl: Arc<glow::Context>,
}

struct Banner {
    registry: RegistryState,
    outputs: OutputState,
    gl: Gl,
    layer: LayerSurface,
    viewport: Option<wp_viewport::WpViewport>,
    egl_window: Option<wayland_egl::WlEglSurface>,
    egl_surface: Option<egl::Surface>,
    painter: Option<egui_glow::Painter>,
    ctx: egui::Context,
    size: (u32, u32),
    scale: f64,
    configured: bool,
    frame_pending: bool,
    shown_at: Option<Instant>,
    closed: bool,
    card: Card,
    icon: Option<egui::ColorImage>,
    texture: Option<egui::TextureHandle>,
    accent: (Color32, Color32),
    /// How shown the last frame drawn was (held still: nothing to draw).
    drawn: f32,
}

pub fn tier_of(name: &str) -> Option<Tier> {
    [Tier::Bronze, Tier::Silver, Tier::Gold, Tier::Platinum]
        .into_iter()
        .find(|t| t.key() == name)
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

impl Banner {
    /// 0…1 shown, and whether it's over.
    fn phase(&self) -> (f32, bool) {
        let Some(at) = self.shown_at else {
            return (0.0, false);
        };
        let t = at.elapsed().as_secs_f32();
        if t < IN {
            (ease(t / IN), false)
        } else if t < IN + STAY {
            (1.0, false)
        } else if t < IN + STAY + OUT {
            (1.0 - ease((t - IN - STAY) / OUT), false)
        } else {
            (0.0, true)
        }
    }

    fn render(&mut self, qh: &QueueHandle<Banner>) {
        if !self.configured || self.frame_pending {
            return;
        }
        let Some(surf) = self.egl_surface else {
            return;
        };
        if self
            .gl
            .egl
            .make_current(
                self.gl.display,
                Some(surf),
                Some(surf),
                Some(self.gl.context),
            )
            .is_err()
        {
            return;
        }
        if self.shown_at.is_none() {
            self.shown_at = Some(Instant::now());
        }
        let (show, _) = self.phase();
        if show >= 1.0 && self.drawn >= 1.0 {
            return;
        }
        self.drawn = show;
        let (w, h) = self.size;
        let ppp = self.scale as f32;
        let mut raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(w as f32, h as f32),
            )),
            max_texture_side: self.painter.as_ref().map(|p| p.max_texture_side()),
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        if self.texture.is_none()
            && let Some(img) = self.icon.take()
        {
            self.texture = Some(self.ctx.load_texture("icon", img, Default::default()));
        }
        let card = &self.card;
        let tex = self.texture.as_ref().map(|t| t.id());
        let accent = self.accent;
        let mut out = self.ctx.run_ui(raw, |ui| {
            draw(ui, card, tex, accent, show);
        });
        let prims = self.ctx.tessellate(std::mem::take(&mut out.shapes), ppp);
        let px = (
            (w as f64 * self.scale).round() as u32,
            (h as f64 * self.scale).round() as u32,
        );
        if let Some(p) = &mut self.painter {
            egui_glow::painter::clear(&self.gl.gl, [px.0, px.1], [0.0, 0.0, 0.0, 0.0]);
            p.paint_and_update_textures([px.0, px.1], ppp, &prims, &mut out.textures_delta);
        }
        let s = self.layer.wl_surface().clone();
        s.frame(qh, s.clone());
        self.frame_pending = true;
        let _ = self.gl.egl.swap_buffers(self.gl.display, surf);
    }
}

/// The banner itself: a dark rounded plate, the icon, the words and the
/// grade, sliding down as `show` goes 0 → 1.
pub(crate) fn draw(
    ui: &mut egui::Ui,
    card: &Card,
    tex: Option<egui::TextureId>,
    (c1, c2): (Color32, Color32),
    show: f32,
) {
    let p = ui.painter();
    let full = ui.max_rect();
    let r = Rect::from_min_size(
        pos2(full.left() + 10.0, full.top() + 6.0 - (1.0 - show) * 24.0),
        vec2(full.width() - 20.0, full.height() - 16.0),
    );
    let a = show;
    let round = 20.0;
    p.rect_filled(
        r.translate(vec2(0.0, 4.0)),
        round,
        Color32::from_black_alpha((90.0 * a) as u8),
    );
    p.rect_filled(
        r,
        round,
        Color32::from_rgba_unmultiplied(18, 20, 28, (238.0 * a) as u8),
    );
    // A thin line of the accent along the top.
    let line = Rect::from_min_size(
        r.left_top() + vec2(round, 0.0),
        vec2(r.width() - 2.0 * round, 3.0),
    );
    crate::ui::widgets::gradient_rect(p, line, c1.gamma_multiply(a), c2.gamma_multiply(a));
    p.rect_stroke(
        r,
        round,
        Stroke::new(1.0, Color32::from_white_alpha((28.0 * a) as u8)),
        egui::StrokeKind::Inside,
    );
    let ir = Rect::from_min_size(r.left_top() + vec2(16.0, 14.0), vec2(62.0, 62.0));
    match tex {
        Some(t) => {
            p.image(
                t,
                ir,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE.gamma_multiply(a),
            );
        }
        None => {
            p.rect_filled(ir, 10.0, Color32::from_white_alpha((20.0 * a) as u8));
            crate::ui::widgets::icon(
                p,
                ir.center(),
                30.0,
                crate::ui::widgets::Icon::Trophy,
                Color32::from_gray(200).gamma_multiply(a),
            );
        }
    }
    let tx = ir.right() + 16.0;
    let right = r.right() - 84.0;
    let head = match card.new_tier.as_deref().and_then(tier_of) {
        Some(t) => format!(
            "{} · {}: {}",
            crate::i18n::tr("ach_got"),
            crate::i18n::tr("tr_new"),
            crate::i18n::tr(t.key())
        ),
        None => format!("{} · {}", crate::i18n::tr("ach_got"), card.game),
    };
    let clip = |s: &str, size: f32, color: Color32| {
        let mut job = egui::text::LayoutJob::simple_singleline(
            s.to_string(),
            FontId::proportional(size),
            color,
        );
        job.wrap.max_width = right - tx;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        p.fonts_mut(|f| f.layout_job(job))
    };
    let g = clip(&head, 13.0, c1.gamma_multiply(a));
    p.galley(pos2(tx, r.top() + 14.0), g, Color32::WHITE);
    let g = clip(&card.name, 19.0, Color32::WHITE.gamma_multiply(a));
    p.galley(pos2(tx, r.top() + 33.0), g, Color32::WHITE);
    if !card.desc.is_empty() {
        let g = clip(&card.desc, 14.0, Color32::from_gray(185).gamma_multiply(a));
        p.galley(pos2(tx, r.top() + 60.0), g, Color32::WHITE);
    }
    // The grade and the tally at the right.
    let t = Tally {
        got: card.got,
        total: card.total,
    };
    let mc = pos2(r.right() - 44.0, r.top() + 36.0);
    match t.tier() {
        Some(tier) => crate::trophy::mark(p, mc, 40.0, tier, a),
        None => crate::trophy::mark(p, mc, 40.0, Tier::Bronze, 0.3 * a),
    }
    if card.total > 0 {
        p.text(
            pos2(mc.x, r.top() + 66.0),
            Align2::CENTER_TOP,
            format!("{} / {}", card.got, card.total),
            FontId::proportional(13.0),
            Color32::from_gray(200).gamma_multiply(a),
        );
        let bar = Rect::from_min_size(
            pos2(r.left() + round, r.bottom() - 7.0),
            vec2(r.width() - 2.0 * round, 3.0),
        );
        crate::ui::widgets::bar(
            p,
            bar,
            t.share(),
            c1.gamma_multiply(a),
            c2.gamma_multiply(a),
        );
    }
}

impl CompositorHandler for Banner {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if self.viewport.is_none() {
            self.scale = factor as f64;
            surface.set_buffer_scale(factor);
            self.resize();
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_pending = false;
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Banner {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl Banner {
    fn resize(&mut self) {
        let (w, h) = self.size;
        let pw = (w as f64 * self.scale).round() as i32;
        let ph = (h as f64 * self.scale).round() as i32;
        if let Some(win) = &self.egl_window {
            win.resize(pw, ph, 0, 0);
        }
        if let Some(vp) = &self.viewport {
            vp.set_destination(w as i32, h as i32);
        }
    }
}

impl LayerShellHandler for Banner {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        cfg: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = (
            if cfg.new_size.0 == 0 {
                W
            } else {
                cfg.new_size.0
            },
            if cfg.new_size.1 == 0 {
                H
            } else {
                cfg.new_size.1
            },
        );
        self.size = (w, h);
        if self.egl_window.is_none() {
            let pw = (w as f64 * self.scale).round() as i32;
            let ph = (h as f64 * self.scale).round() as i32;
            let Ok(win) = wayland_egl::WlEglSurface::new(self.layer.wl_surface().id(), pw, ph)
            else {
                self.closed = true;
                return;
            };
            let surf = unsafe {
                self.gl.egl.create_window_surface(
                    self.gl.display,
                    self.gl.config,
                    win.ptr() as egl::NativeWindowType,
                    None,
                )
            };
            let Ok(surf) = surf else {
                self.closed = true;
                return;
            };
            self.egl_window = Some(win);
            self.egl_surface = Some(surf);
            let _ = self.gl.egl.make_current(
                self.gl.display,
                Some(surf),
                Some(surf),
                Some(self.gl.context),
            );
            let _ = self.gl.egl.swap_interval(self.gl.display, 0);
            self.painter = egui_glow::Painter::new(self.gl.gl.clone(), "", None, false).ok();
            if let Some(vp) = &self.viewport {
                vp.set_destination(w as i32, h as i32);
            } else {
                self.layer
                    .wl_surface()
                    .set_buffer_scale(self.scale.round() as i32);
            }
        } else {
            self.resize();
        }
        self.configured = true;
    }
}

impl ProvidesRegistryState for Banner {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for Banner {
    fn event(
        b: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        ev: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = ev {
            let new = scale as f64 / 120.0;
            if (new - b.scale).abs() > 1e-6 {
                b.scale = new;
                b.resize();
            }
        }
    }
}

delegate_noop!(Banner: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(Banner: ignore wp_viewporter::WpViewporter);
delegate_noop!(Banner: ignore wp_viewport::WpViewport);
delegate_noop!(Banner: ignore wl_region::WlRegion);
delegate_compositor!(Banner);
delegate_output!(Banner);
delegate_layer!(Banner);
delegate_registry!(Banner);

fn init_egl(conn: &Connection) -> Result<Gl, String> {
    let e = egl::Instance::new(egl::Static);
    const PLATFORM_WAYLAND: egl::Enum = 0x31D8;
    let display = unsafe {
        e.get_platform_display(
            PLATFORM_WAYLAND,
            conn.backend().display_ptr() as *mut _,
            &[egl::ATTRIB_NONE],
        )
    }
    .map_err(|err| format!("EGL display: {err}"))?;
    e.initialize(display)
        .map_err(|err| format!("eglInitialize: {err}"))?;
    e.bind_api(egl::OPENGL_ES_API)
        .map_err(|err| err.to_string())?;
    let attrs = [
        egl::SURFACE_TYPE,
        egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_ES3_BIT,
        egl::RED_SIZE,
        8,
        egl::GREEN_SIZE,
        8,
        egl::BLUE_SIZE,
        8,
        egl::ALPHA_SIZE,
        8,
        egl::NONE,
    ];
    let config = e
        .choose_first_config(display, &attrs)
        .map_err(|err| err.to_string())?
        .ok_or("no EGL config")?;
    let context = e
        .create_context(
            display,
            config,
            None,
            &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE],
        )
        .map_err(|err| format!("eglCreateContext: {err}"))?;
    e.make_current(display, None, None, Some(context))
        .map_err(|err| err.to_string())?;
    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            e.get_proc_address(s)
                .map_or(std::ptr::null(), |f| f as *const _)
        })
    };
    Ok(Gl {
        egl: e,
        display,
        config,
        context,
        gl: Arc::new(gl),
    })
}

fn load_icon(path: &str) -> Option<egui::ColorImage> {
    let img = image::open(path).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

/// The compositor's socket when WAYLAND_DISPLAY isn't set (some sandboxes
/// pass the socket without it).
fn socket_in_runtime_dir() -> Option<Connection> {
    let dir = std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let mut socks: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("wayland-") && !n.ends_with(".lock"))
        })
        .collect();
    socks.sort();
    socks.into_iter().find_map(|p| {
        let s = std::os::unix::net::UnixStream::connect(p).ok()?;
        Connection::from_socket(s).ok()
    })
}

/// Show one banner and wait till it's gone; the process exit code.
pub fn run(card: Card) -> i32 {
    let conn = match Connection::connect_to_env().or_else(|e| socket_in_runtime_dir().ok_or(e)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lenta: banner: Wayland: {e}");
            return 3;
        }
    };
    let Ok((globals, queue)) = registry_queue_init::<Banner>(&conn) else {
        eprintln!("lenta: banner: no Wayland registry");
        return 3;
    };
    let qh = queue.handle();
    let (Ok(compositor), Ok(layer_shell)) = (
        CompositorState::bind(&globals, &qh),
        LayerShell::bind(&globals, &qh),
    ) else {
        eprintln!("lenta: banner: the compositor has no wlr-layer-shell");
        return 3;
    };
    let gl = match init_egl(&conn) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("lenta: banner: {e}");
            return 3;
        }
    };
    let fractional: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1> =
        globals.bind(&qh, 1..=1, ()).ok();
    let viewporter: Option<wp_viewporter::WpViewporter> = globals.bind(&qh, 1..=1, ()).ok();
    let surface = compositor.create_surface(&qh);
    // Clicks go through to the game.
    let region = compositor.wl_compositor().create_region(&qh, ());
    surface.set_input_region(Some(&region));
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface.clone(),
        Layer::Overlay,
        Some("lenta-achievement"),
        None,
    );
    layer.set_anchor(Anchor::TOP);
    layer.set_size(W, H);
    layer.set_margin(28, 0, 0, 0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(-1);
    let _frac = fractional
        .as_ref()
        .map(|f| f.get_fractional_scale(&surface, &qh, ()));
    let viewport = if fractional.is_some() {
        viewporter
            .as_ref()
            .map(|v| v.get_viewport(&surface, &qh, ()))
    } else {
        None
    };
    layer.commit();
    let cfg = crate::config::Config::load();
    let ctx = egui::Context::default();
    crate::fonts::install(&ctx);
    let accent = (
        Color32::from_rgb(cfg.color1[0], cfg.color1[1], cfg.color1[2]),
        Color32::from_rgb(cfg.color2[0], cfg.color2[1], cfg.color2[2]),
    );
    let icon = card.icon.as_deref().and_then(load_icon);
    let mut b = Banner {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        gl,
        layer,
        viewport,
        egl_window: None,
        egl_surface: None,
        painter: None,
        ctx,
        size: (W, H),
        scale: 1.0,
        configured: false,
        frame_pending: false,
        shown_at: None,
        closed: false,
        card,
        icon,
        texture: None,
        accent,
        drawn: -1.0,
    };
    let Ok(mut event_loop) = EventLoop::<Banner>::try_new() else {
        return 3;
    };
    if WaylandSource::new(conn.clone(), queue)
        .insert(event_loop.handle())
        .is_err()
    {
        return 3;
    }
    let start = Instant::now();
    loop {
        if event_loop
            .dispatch(Some(Duration::from_millis(16)), &mut b)
            .is_err()
            || b.closed
        {
            break;
        }
        // Never configured: this desktop won't show it.
        if !b.configured && start.elapsed() > Duration::from_secs(3) {
            eprintln!("lenta: banner: the surface was never configured");
            return 3;
        }
        let (_, over) = b.phase();
        if over {
            break;
        }
        b.render(&qh);
        let _ = conn.flush();
    }
    if let Some(p) = &mut b.painter {
        p.destroy();
    }
    if b.shown_at.is_some() { 0 } else { 3 }
}
