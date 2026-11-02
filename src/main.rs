//! lenta: a gamepad-first game launcher.

mod app;
mod config;
mod fonts;
mod game;
mod i18n;
mod images;
mod input;
mod net;
mod sources;
mod store;
mod ui;
mod vdf;

#[cfg(test)]
mod shots;

fn main() -> eframe::Result {
    if std::env::args().nth(1).as_deref() == Some("--list") {
        // What was found, for checking the sources without the window.
        let cfg = config::Config::load();
        i18n::set(cfg.lang);
        let mut games = sources::scan(&cfg);
        sources::sort(&mut games, cfg.sort);
        for g in &games {
            println!(
                "{:<8} {:<40} {:>6} min  last {:>10}  portrait {}",
                g.source.name(),
                g.title,
                g.playtime_min.unwrap_or(0),
                g.last_played.map(game::date).unwrap_or_default(),
                match &g.art.portrait {
                    Some(game::Art::File(p)) => p.display().to_string(),
                    Some(game::Art::Url(u)) => u.clone(),
                    None => "-".into(),
                }
            );
        }
        let apps = sources::desktop::apps();
        println!("{} games, {} system apps", games.len(), apps.len());
        return Ok(());
    }
    let cfg = config::Config::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Lenta")
            .with_app_id("lenta")
            .with_inner_size([1440.0, 860.0])
            .with_min_inner_size([800.0, 500.0])
            .with_fullscreen(cfg.fullscreen),
        // No waiting for the display in eglSwapBuffers: on Wayland a window
        // behind a full-screen game gets no frame callbacks, and the wait
        // never ends. Frames are paced in App::ui.
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: false,
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "lenta",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            fonts::install(&cc.egui_ctx);
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(app::App::new(cc.egui_ctx.clone())))
        }),
    )
}
