//! Offscreen renders of the screens (wgpu), on this machine's real games.
//! Not run by default:
//!   LENTA_SHOT_DIR=/tmp/shots cargo test shots -- --ignored --nocapture

use std::path::PathBuf;

use egui_kittest::Harness;

use crate::app::{App, Page};
use crate::config::{AccentMode, Anchor, Orientation, Shape, TiltMode};
use crate::input::Action;

fn harness(size: [f32; 2]) -> Harness<'static, App> {
    let cfg = std::env::temp_dir().join(format!("lenta-test-{}", std::process::id()));
    unsafe {
        std::env::set_var("LENTA_CONFIG", cfg.join("config.toml"));
        std::env::set_var("LENTA_NO_GAMEPAD", "1");
        // Never start a game, Steam or a program from a test.
        std::env::set_var("LENTA_DRY_RUN", "1");
    }
    let _ = std::fs::remove_file(cfg.join("config.toml"));
    Harness::builder()
        .with_size(size)
        .wgpu()
        .build_eframe(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            crate::fonts::install(&cc.egui_ctx);
            App::new(cc.egui_ctx.clone())
        })
}

fn settle(h: &mut Harness<'static, App>, rounds: usize) {
    for _ in 0..rounds {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
}

fn snap(h: &mut Harness<'static, App>, dir: &Option<PathBuf>, name: &str) {
    settle(h, 30);
    let img = h.render().expect("render");
    if let Some(d) = dir {
        std::fs::create_dir_all(d).unwrap();
        img.save(d.join(format!("{name}.png"))).unwrap();
    }
}

#[test]
#[ignore]
fn shots() {
    let dir = std::env::var_os("LENTA_SHOT_DIR").map(PathBuf::from);
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    h.state_mut().handle(Action::Right);
    snap(&mut h, &dir, "home");
    {
        let c = &mut h.state_mut().cfg;
        c.shape = Shape::Landscape;
        c.accent_mode = AccentMode::CardGradient;
        c.tilt = 0.0;
        c.anchor = Anchor::Start;
    }
    snap(&mut h, &dir, "home-landscape");
    {
        let c = &mut h.state_mut().cfg;
        c.shape = Shape::Portrait;
        c.orientation = Orientation::Vertical;
        c.anchor = Anchor::Center;
        c.tilt = 24.0;
        c.tilt_mode = TiltMode::Fan;
    }
    snap(&mut h, &dir, "home-vertical");
    h.state_mut().cfg = Default::default();
    h.state_mut().handle(Action::View);
    snap(&mut h, &dir, "sidebar");
    h.state_mut().handle(Action::Back);
    h.state_mut().handle(Action::Accept);
    snap(&mut h, &dir, "game");
    h.state_mut().handle(Action::RightTrigger);
    snap(&mut h, &dir, "game-achievements");
    h.state_mut().handle(Action::RightTrigger);
    settle(&mut h, 40);
    snap(&mut h, &dir, "game-shots");
    // A Steam game's compatibility tab, its list focused.
    let steam_game = h
        .state()
        .games
        .iter()
        .find(|g| g.compat_id.is_some())
        .map(|g| g.key.clone());
    if let Some(k) = steam_game {
        h.state_mut().game = None;
        h.state_mut().open_game(k);
        // Focus walks from Play over the tabs to the last one.
        for _ in 0..4 {
            h.state_mut().handle(Action::Right);
        }
        h.state_mut().handle(Action::Down);
        snap(&mut h, &dir, "game-compat");
        h.state_mut().handle(Action::Back);
        h.state_mut().game = None;
    }
    h.state_mut().handle(Action::Back);
    // A game from elsewhere with a Steam emulator's save, if there is one.
    let other = h
        .state()
        .games
        .iter()
        .find(|g| g.emu.is_some())
        .map(|g| g.key.clone());
    if let Some(k) = other {
        h.state_mut().open_game(k);
        snap(&mut h, &dir, "other-game");
        h.state_mut().handle(Action::RightTrigger);
        settle(&mut h, 40);
        snap(&mut h, &dir, "other-achievements");
        h.state_mut().handle(Action::Back);
    }
    for (p, name) in [
        (Page::Library, "library"),
        (Page::System, "system"),
        (Page::Settings, "settings"),
        (Page::About, "about"),
    ] {
        if p == Page::About {
            // The Steam sign-in window (a real code nobody confirms).
            h.state_mut().start_login();
            for _ in 0..100 {
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            snap(&mut h, &dir, "login");
            h.state_mut().login = None;
        }
        h.state_mut().go(p);
        snap(&mut h, &dir, name);
        if p == Page::Library {
            // Each source's own tab colours.
            for i in 1..=3 {
                h.state_mut().handle(Action::RightTrigger);
                snap(&mut h, &dir, &format!("library-{i}"));
            }
            h.state_mut().handle(Action::RightTrigger);
        }
    }
}
