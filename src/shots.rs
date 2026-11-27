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
        // None of the user's modules (a test puts in the example).
        std::env::set_var("LENTA_MODULES", cfg.join("no-modules"));
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
    // Held and dragged to the left by the mouse: later cards come in.
    {
        use egui::{Event, PointerButton, pos2};
        let before = h.state().home.sel;
        let n = h.state().games.len();
        let press = |h: &mut Harness<'static, App>, at, pressed| {
            h.input_mut().events.push(Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
            h.step();
        };
        let start = pos2(1300.0, 330.0);
        h.input_mut().events.push(Event::PointerMoved(start));
        h.step();
        press(&mut h, start, true);
        for i in 1..=20 {
            h.input_mut().events.push(Event::PointerMoved(
                start - egui::vec2(i as f32 * 30.0, 0.0),
            ));
            h.step();
        }
        assert!(h.state().home.dragging, "ribbon not dragged");
        press(&mut h, start - egui::vec2(600.0, 0.0), false);
        settle(&mut h, 30);
        let after = h.state().home.sel;
        assert!(n < 3 || after > before, "drag moved {before} -> {after}");
        assert!(!h.state().home.dragging);
        h.state_mut().home.sel = before;
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
    // Into the list: the header folds away; the third one chosen.
    h.state_mut().handle(Action::Down);
    h.state_mut().handle(Action::Down);
    h.state_mut().handle(Action::Down);
    settle(&mut h, 40);
    snap(&mut h, &dir, "game-achievements-list");
    for _ in 0..3 {
        h.state_mut().handle(Action::Up);
    }
    h.state_mut().handle(Action::RightTrigger);
    settle(&mut h, 40);
    snap(&mut h, &dir, "game-shots");
    h.state_mut().handle(Action::Down);
    h.state_mut().handle(Action::Right);
    settle(&mut h, 40);
    snap(&mut h, &dir, "game-shots-chosen");
    h.state_mut().handle(Action::Back);
    // A Steam game's compatibility tab, its list focused.
    // Graveyard Keeper 2 when it's there (several versions known).
    let steam_game = h
        .state()
        .games
        .iter()
        .find(|g| g.key == "steam:4358690")
        .or_else(|| h.state().games.iter().find(|g| g.compat_id.is_some()))
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
        // Down to the upkeep rows; the question is looked at and cancelled.
        for _ in 0..40 {
            h.state_mut().handle(Action::Down);
        }
        settle(&mut h, 40);
        snap(&mut h, &dir, "game-compat-upkeep");
        h.state_mut().handle(Action::Accept);
        settle(&mut h, 20);
        snap(&mut h, &dir, "game-prefix-ask");
        if h.state().confirm.is_some() {
            h.state_mut().handle(Action::Back);
        }
        assert!(h.state().confirm.is_none());
        h.state_mut().handle(Action::Back);
        // The Versions tab next to it, its list focused (reading only).
        h.state_mut().handle(Action::Right);
        for _ in 0..200 {
            h.step();
            if h.state().versions.data.is_some() && h.state().versions.loading.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        h.state_mut().handle(Action::Down);
        snap(&mut h, &dir, "game-versions");
        // Tick the version above the installed one and ask to install it;
        // the question is only looked at and answered No.
        h.state_mut().handle(Action::Up);
        h.state_mut().handle(Action::Accept);
        snap(&mut h, &dir, "game-versions-ticked");
        h.state_mut().handle(Action::Select);
        snap(&mut h, &dir, "game-versions-ask");
        assert!(h.state().versions.ask.is_some());
        h.state_mut().handle(Action::Back);
        assert!(h.state().versions.ask.is_none() && h.state().versions.op.is_none());
        h.state_mut().handle(Action::Back);
        // The Look tab next to it: browse the covers (nothing taken).
        h.state_mut().handle(Action::Right);
        for _ in 0..300 {
            h.step();
            if h.state().art.tried {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        h.state_mut().handle(Action::Down);
        h.state_mut().handle(Action::Right);
        h.state_mut().handle(Action::Right);
        settle(&mut h, 60);
        snap(&mut h, &dir, "game-art");
        // The theme line under the header (title put in by hand; no sound
        // in tests).
        h.state_mut().handle(Action::Back);
        h.state_mut().cfg.theme_music = true;
        let key = h
            .state()
            .game
            .as_ref()
            .map(|v| v.key.clone())
            .unwrap_or_default();
        *h.state().music.now.lock().unwrap() = Some((
            key,
            "Graveyard Keeper 2 Original Soundtrack - Workshop Place".into(),
        ));
        settle(&mut h, 30);
        snap(&mut h, &dir, "game-music");
        h.state_mut().cfg.theme_music = false;
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
        (Page::Proton, "proton"),
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
            // The SteamGridDB question (names given, nothing sent).
            h.state_mut().sgdb.ask = Some((vec!["Sendem".into(), "Sendem_lenta".into()], 0));
            h.state_mut().handle(Action::Right);
            snap(&mut h, &dir, "sgdb-ask");
            h.state_mut().handle(Action::Back);
            assert!(h.state().sgdb.ask.is_none() && h.state().sgdb.job.is_none());
            // Settings at the SteamGridDB row.
            h.state_mut().go(Page::Settings);
            for _ in 0..60 {
                h.state_mut().handle(Action::Down);
            }
            snap(&mut h, &dir, "settings-sgdb");
            // The Sounds page: installed packs, then the store.
            h.state_mut().go(Page::Sounds);
            for _ in 0..300 {
                h.step();
                if h.state().sounds.loaded && h.state().sounds.loading.is_none() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            settle(&mut h, 30);
            snap(&mut h, &dir, "sounds-installed");
            h.state_mut().handle(Action::RightTrigger);
            h.state_mut().handle(Action::Right);
            settle(&mut h, 80);
            snap(&mut h, &dir, "sound-store");
            h.state_mut().handle(Action::LeftTrigger);
        }
        h.state_mut().go(p);
        snap(&mut h, &dir, name);
        if p == Page::Proton {
            for _ in 0..300 {
                h.step();
                if h.state().proton.loading.is_none() && !h.state().proton.lists.is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            for _ in 0..4 {
                h.state_mut().handle(Action::Down);
            }
            snap(&mut h, &dir, "proton");
            // The wheel scrolls the page and it stays there (the choice
            // doesn't pull it back).
            {
                use egui::{Event, MouseWheelUnit, pos2, vec2};
                let before = h.state().proton.scroll;
                h.input_mut()
                    .events
                    .push(Event::PointerMoved(pos2(700.0, 500.0)));
                h.step();
                for _ in 0..6 {
                    h.input_mut().events.push(Event::MouseWheel {
                        unit: MouseWheelUnit::Point,
                        delta: vec2(0.0, -120.0),
                        modifiers: Default::default(),
                        phase: egui::TouchPhase::Move,
                    });
                    h.step();
                }
                settle(&mut h, 30);
                let after = h.state().proton.scroll;
                assert!(after > before + 100.0, "wheel: {before} -> {after}");
            }
            // A download under way (made up: nothing is fetched).
            let first = h
                .state()
                .proton
                .lists
                .get(&crate::protonup::Flavour::Ge)
                .and_then(|l| l.as_ref().ok())
                .and_then(|l| l.first())
                .map(|r| r.tag.clone());
            if let Some(tag) = first {
                let (_tx, rx) = crossbeam_channel::bounded(1);
                std::mem::forget(_tx);
                h.state_mut().proton.job = Some(crate::ui::proton::Job {
                    tag,
                    progress: std::sync::Arc::new(std::sync::Mutex::new(
                        crate::protonup::Progress {
                            done: 120 << 20,
                            total: 560 << 20,
                            unpacking: false,
                        },
                    )),
                    cancel: Default::default(),
                    done: rx,
                });
                for _ in 0..8 {
                    h.state_mut().handle(Action::Down);
                }
                snap(&mut h, &dir, "proton-download");
                h.state_mut().proton.job = None;
            }
        }
        if p == Page::Library {
            // Each source's own tab colours.
            for i in 1..=3 {
                h.state_mut().handle(Action::RightTrigger);
                snap(&mut h, &dir, &format!("library-{i}"));
            }
            // Back on Steam: the shelves, then Collections and one opened.
            h.state_mut().lib.filter = 0;
            h.state_mut().libv.side = true;
            for _ in 0..3 {
                h.state_mut().handle(Action::Down);
            }
            settle(&mut h, 60);
            snap(&mut h, &dir, "library-collections");
            h.state_mut().handle(Action::Right);
            h.state_mut().handle(Action::Accept);
            settle(&mut h, 60);
            snap(&mut h, &dir, "library-collection-open");
            h.state_mut().handle(Action::Back);
            h.state_mut().handle(Action::Back);
            own_collections(&mut h, &dir);
        }
    }
}

/// The launcher's own collections: made in the editor (the name typed on
/// the on-screen keyboard), a card dragged into one, the context menu, a
/// dynamic one. Nothing is written to disk in a dry run.
fn own_collections(h: &mut Harness<'static, App>, dir: &Option<PathBuf>) {
    use crate::colls::Played;
    use egui::{Event, PointerButton};
    // The last tab: own collections, the "New" card chosen.
    h.state_mut().lib.filter = 99;
    h.state_mut().libv.shelf = 0;
    h.state_mut().libv.side = false;
    h.state_mut().lib.sel = 0;
    snap(h, dir, "colls-empty");
    h.state_mut().handle(Action::Accept);
    assert!(h.state().cedit.is_some(), "editor not open");
    // A on the name: the keyboard; a word typed on it, one by a real
    // keyboard.
    h.state_mut().handle(Action::Accept);
    assert!(h.state().osk.is_some(), "keyboard not open");
    for a in [
        Action::Right,
        Action::Right,
        Action::Accept,
        Action::Down,
        Action::Accept,
    ] {
        h.state_mut().handle(a);
    }
    h.input_mut().events.push(Event::Text("оп".into()));
    snap(h, dir, "osk");
    h.state_mut().handle(Action::Menu);
    assert!(h.state().osk.is_none());
    let name = h.state().cedit.as_ref().unwrap().name.clone();
    assert!(name.ends_with("оп"), "typed {name:?}");
    h.state_mut().cedit.as_mut().unwrap().name = "Кооп".into();
    // Plain → dynamic and back: the rule rows come and go.
    h.state_mut().handle(Action::Down);
    h.state_mut().handle(Action::Right);
    snap(h, dir, "colls-editor-dynamic");
    h.state_mut().handle(Action::Left);
    h.state_mut().handle(Action::Menu);
    assert_eq!(h.state().colls.list.len(), 1);
    // A dynamic one: played this year.
    h.state_mut().edit_coll(None, None);
    {
        let e = h.state_mut().cedit.as_mut().unwrap();
        e.name = "Недавние".into();
        e.dynamic = true;
        e.rule.played = Played::Year;
    }
    h.state_mut().handle(Action::Menu);
    assert_eq!(h.state().colls.list.len(), 2);

    // Steam, all games: the first card dragged onto "Кооп".
    h.state_mut().lib.filter = 0;
    h.state_mut().libv.shelf = 0;
    h.state_mut().lib.sel = 0;
    h.state_mut().lib.scroll = 0.0;
    settle(h, 20);
    let tile = h.state().libv.tile0.expect("no tile");
    let key = h.state().lib_first_key();
    let press = |h: &mut Harness<'static, App>, at, pressed| {
        h.input_mut().events.push(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.step();
    };
    let start = tile.center();
    h.input_mut().events.push(Event::PointerMoved(start));
    h.step();
    press(h, start, true);
    for i in 1..=6 {
        h.input_mut().events.push(Event::PointerMoved(
            start + egui::vec2(-i as f32 * 10.0, i as f32 * 4.0),
        ));
        h.step();
    }
    assert_eq!(
        h.state().libv.drag.as_deref(),
        Some(key.as_str()),
        "not dragging"
    );
    let target = h.state().libv.drop_rows[0].0.center();
    for i in 1..=10 {
        let t = i as f32 / 10.0;
        h.input_mut()
            .events
            .push(Event::PointerMoved(start + (target - start) * t));
        h.step();
    }
    let img = h.render().expect("render");
    if let Some(d) = dir {
        img.save(d.join("library-drag.png")).unwrap();
    }
    press(h, target, false);
    settle(h, 5);
    assert!(h.state().libv.drag.is_none());
    assert!(
        h.state().colls.list[0].games.contains(&key),
        "not dropped in: {:?}",
        h.state().colls.list[0].games
    );
    // The context menu on the same card.
    h.input_mut()
        .events
        .push(Event::PointerMoved(egui::pos2(1500.0, 860.0)));
    h.state_mut().handle(Action::Actions);
    snap(h, dir, "library-menu");
    h.state_mut().handle(Action::Back);
    assert!(h.state().menu.is_none());
    // Own collections, then "Кооп" opened.
    h.state_mut().lib.filter = 99;
    h.state_mut().lib.sel = 1;
    snap(h, dir, "colls");
    h.state_mut().handle(Action::Accept);
    snap(h, dir, "colls-open");
    h.state_mut().handle(Action::Back);
    h.state_mut().lib.filter = 0;
}

/// A game page scrolled a little with the wheel: the header folds once
/// and stays, the content moves one way only (it used to jump back and
/// forth when the page was only a bit taller than the window).
#[test]
#[ignore]
fn game_page_wheel_steady() {
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let n = h.state().games.len().min(8);
    let mut checked = 0;
    for gi in 0..n {
        let key = h.state().games[gi].key.clone();
        h.state_mut().open_game(key.clone());
        settle(&mut h, 25);
        h.input_mut()
            .events
            .push(egui::Event::PointerMoved(egui::pos2(800.0, 700.0)));
        let mut trace = Vec::new();
        for f in 0..90 {
            if f < 6 {
                h.input_mut().events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -25.0),
                    modifiers: Default::default(),
                    phase: egui::TouchPhase::Move,
                });
            }
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(16));
            let v = h.state().game.as_ref().unwrap();
            trace.push((v.shown, v.folded, v.max_scroll));
        }
        let ups = trace.windows(2).filter(|w| w[1].0 < w[0].0 - 0.5).count();
        let unfolds = trace.windows(2).filter(|w| w[0].1 && !w[1].1).count();
        let (shown, folded, max) = *trace.last().unwrap();
        eprintln!(
            "{key}: shown {shown:.0} of {max:.0}, folded {folded}, back-moves {ups}, unfolds {unfolds}"
        );
        assert_eq!(ups, 0, "{key}: content moved back while scrolling down");
        assert_eq!(unfolds, 0, "{key}: header opened again by itself");
        checked += 1;
        h.state_mut().game = None;
        settle(&mut h, 3);
    }
    assert!(checked > 0);
}

/// The mouse gets the sounds the pad gets: the ribbon dragged and
/// wheeled ticks, a click presses, a slider dragged ticks.
#[test]
#[ignore]
fn sounds_follow_mouse() {
    use crate::sfx::Sound;
    use egui::{Event, PointerButton, pos2, vec2};
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    settle(&mut h, 10);
    let heard = |h: &mut Harness<'static, App>| -> Vec<Sound> {
        std::mem::take(&mut *h.state().sfx.heard.borrow_mut())
    };
    heard(&mut h);
    let press = |h: &mut Harness<'static, App>, at, pressed| {
        h.input_mut().events.push(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.step();
    };
    // The ribbon dragged.
    let start = pos2(1300.0, 330.0);
    h.input_mut().events.push(Event::PointerMoved(start));
    h.step();
    press(&mut h, start, true);
    for i in 1..=20 {
        h.input_mut()
            .events
            .push(Event::PointerMoved(start - vec2(i as f32 * 30.0, 0.0)));
        h.step();
    }
    press(&mut h, start - vec2(600.0, 0.0), false);
    settle(&mut h, 10);
    let got = heard(&mut h);
    eprintln!("drag: {got:?}");
    assert!(got.contains(&Sound::Navigate), "ribbon drag silent");
    // The wheel on the ribbon.
    for _ in 0..4 {
        h.input_mut().events.push(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 60.0),
            modifiers: Default::default(),
            phase: egui::TouchPhase::Move,
        });
        h.step();
    }
    settle(&mut h, 10);
    let got = heard(&mut h);
    eprintln!("wheel: {got:?}");
    assert!(got.contains(&Sound::Navigate), "ribbon wheel silent");
    // A click on the library's second tab.
    h.state_mut().go(Page::Library);
    settle(&mut h, 5);
    heard(&mut h);
    let chip = pos2(285.0, 84.0);
    h.input_mut().events.push(Event::PointerMoved(chip));
    h.step();
    press(&mut h, chip, true);
    press(&mut h, chip, false);
    settle(&mut h, 3);
    let got = heard(&mut h);
    eprintln!("click: {got:?} filter {}", h.state().lib.filter);
    assert!(got.contains(&Sound::Activate), "click silent");
}

/// Modules: the example's section, its window, its tab on a game's page.
#[test]
#[ignore]
fn modules() {
    use crate::app::{Page, Tab};
    let dir = std::env::var_os("LENTA_SHOT_DIR").map(PathBuf::from);
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    // Nothing yet: the empty Modules section.
    h.state_mut().go(Page::Modules);
    snap(&mut h, &dir, "modules-empty");
    let ex = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/modules");
    h.state_mut().mods = crate::ui::modules::Mods::load_from(vec![(only(&ex, "demo"), false)]);
    assert_eq!(h.state().mods.list.len(), 1);
    assert!(h.state().pages().contains(&Page::Module(0)));
    snap(&mut h, &dir, "modules");
    // Its section: the module's main page, as it draws it.
    h.state_mut().handle(Action::Accept);
    assert_eq!(h.state().page, Page::Module(0));
    let wait = |h: &mut Harness<'static, App>, w: fn(&App) -> bool| {
        for _ in 0..200 {
            h.step();
            if w(h.state()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the module didn't answer");
    };
    wait(&mut h, |a| {
        a.mods
            .page
            .as_ref()
            .is_some_and(|v| v.page.is_some() && !v.busy())
    });
    // The switch, the slider one step, the choice one on.
    h.state_mut().handle(Action::Accept);
    h.state_mut().handle(Action::Down);
    h.state_mut().handle(Action::Right);
    h.state_mut().handle(Action::Down);
    h.state_mut().handle(Action::Right);
    wait(&mut h, |a| a.mods.page.as_ref().is_some_and(|v| !v.busy()));
    snap(&mut h, &dir, "module-page");
    let items = h
        .state()
        .mods
        .page
        .as_ref()
        .unwrap()
        .page
        .clone()
        .unwrap()
        .items;
    use crate::modules::Kind;
    assert_eq!(
        items[1].kind,
        Kind::Toggle(false),
        "the module kept the switch"
    );
    assert!(matches!(items[2].kind, Kind::Slider { value, .. } if value == 4.0));
    // A page further in, and back.
    for _ in 0..2 {
        h.state_mut().handle(Action::Down);
    }
    h.state_mut().handle(Action::Accept);
    wait(&mut h, |a| {
        a.mods
            .page
            .as_ref()
            .is_some_and(|v| v.stack.len() == 2 && v.page.is_some())
    });
    snap(&mut h, &dir, "module-subpage");
    h.state_mut().handle(Action::Back);
    assert_eq!(h.state().mods.page.as_ref().unwrap().stack.len(), 1);
    // Kept to Modules: off the side panel, opens as a window.
    h.state_mut().go(Page::Modules);
    h.state_mut().handle(Action::Actions);
    assert!(!h.state().pages().contains(&Page::Module(0)));
    h.state_mut().handle(Action::Accept);
    assert!(h.state().mods.over.is_some());
    wait(&mut h, |a| {
        a.mods.over.as_ref().is_some_and(|v| v.page.is_some())
    });
    snap(&mut h, &dir, "module-window");
    h.state_mut().handle(Action::Back);
    assert!(h.state().mods.over.is_none());
    // Its tab on a game's page.
    let key = h.state().games.first().map(|g| g.key.clone());
    if let Some(key) = key {
        h.state_mut().open_game(key);
        let tab = h.state().game.as_ref().map(|v| {
            let g = h.state().game_by_key(&v.key).unwrap().clone();
            h.state().tabs_of(&g)
        });
        let tabs = tab.unwrap();
        let i = tabs
            .iter()
            .position(|t| matches!(t, Tab::Module(_)))
            .expect("module tab");
        {
            let v = h.state_mut().game.as_mut().unwrap();
            v.tab = tabs[i];
            v.focus = i + 1;
        }
        h.state_mut().handle(Action::Down);
        wait(&mut h, |a| {
            a.mods.game.as_ref().is_some_and(|v| v.page.is_some())
        });
        h.state_mut().handle(Action::Accept);
        wait(&mut h, |a| a.mods.game.as_ref().is_some_and(|v| !v.busy()));
        snap(&mut h, &dir, "module-game-tab");
        let items = h
            .state()
            .mods
            .game
            .as_ref()
            .unwrap()
            .page
            .clone()
            .unwrap()
            .items;
        assert_eq!(items[0].kind, Kind::Toggle(true));
        assert!(!items[1].disabled, "on for the game: the slider wakes");
    }
    // The module gone (its package removed): the section with it.
    h.state_mut().game = None;
    h.state_mut().go(Page::Modules);
    h.state_mut().handle(Action::Actions);
    h.state_mut().go(Page::Module(0));
    let empty = cfg_dir().join("no-modules");
    h.state_mut().mods.dirs = vec![(empty, false)];
    h.state_mut().mods_rescan_now();
    assert_eq!(h.state().page, Page::Home);
    assert!(
        !h.state()
            .pages()
            .iter()
            .any(|p| matches!(p, Page::Module(_)))
    );
}

/// A modules folder with just this one of the examples.
fn only(ex: &std::path::Path, id: &str) -> PathBuf {
    let d = cfg_dir().join(format!("modules-{id}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::os::unix::fs::symlink(ex.join(id), d.join(id)).unwrap();
    d
}

fn cfg_dir() -> PathBuf {
    std::env::temp_dir().join(format!("lenta-test-{}", std::process::id()))
}

/// The System module: its own side panel (Wi-Fi, Bluetooth, About), pages
/// asked again while shown. A dry run: it reads, changes nothing.
#[test]
#[ignore]
fn system_module() {
    use crate::app::Page;
    use crate::modules::Kind;
    let dir = std::env::var_os("LENTA_SHOT_DIR").map(PathBuf::from);
    let mut h = harness([1600.0, 900.0]);
    settle(&mut h, 5);
    let ex = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/modules");
    h.state_mut().mods = crate::ui::modules::Mods::load_from(vec![(only(&ex, "system"), false)]);
    let i = h
        .state()
        .mods
        .list
        .iter()
        .position(|m| m.id == "system")
        .unwrap();
    h.state_mut().go(Page::Module(i as u16));
    let wait = |h: &mut Harness<'static, App>, w: &dyn Fn(&App) -> bool| {
        for _ in 0..300 {
            h.step();
            if w(h.state()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the module didn't answer");
    };
    let ready = |a: &App| {
        a.mods
            .page
            .as_ref()
            .is_some_and(|v| v.page.is_some() && !v.busy())
    };
    wait(&mut h, &ready);
    {
        let v = h.state().mods.page.as_ref().unwrap();
        assert_eq!(v.nav.len(), 3, "its panel");
        assert!(v.side, "the choice starts in the panel");
        assert_eq!(v.stack, vec!["main".to_string()]);
    }
    snap(&mut h, &dir, "system-wifi");
    // ↓ in the panel: the next section at once.
    h.state_mut().handle(Action::Down);
    wait(&mut h, &|a: &App| {
        a.mods
            .page
            .as_ref()
            .is_some_and(|v| v.stack == ["bluetooth"] && v.page.is_some())
    });
    snap(&mut h, &dir, "system-bluetooth");
    h.state_mut().handle(Action::Down);
    wait(&mut h, &|a: &App| {
        a.mods
            .page
            .as_ref()
            .is_some_and(|v| v.stack == ["about"] && v.page.is_some())
    });
    snap(&mut h, &dir, "system-about");
    // Back to Wi-Fi, into the page: the switch (a dry run: it stays on).
    h.state_mut().handle(Action::Up);
    h.state_mut().handle(Action::Up);
    wait(&mut h, &|a: &App| {
        a.mods
            .page
            .as_ref()
            .is_some_and(|v| v.stack == ["wifi"] && v.page.is_some() && !v.busy())
    });
    h.state_mut().handle(Action::Right);
    assert!(!h.state().mods.page.as_ref().unwrap().side);
    let first = |a: &App| {
        a.mods.page.as_ref().unwrap().page.as_ref().unwrap().items[0]
            .kind
            .clone()
    };
    let was = first(h.state());
    h.state_mut().handle(Action::Accept);
    wait(&mut h, &|a: &App| {
        a.mods.page.as_ref().is_some_and(|v| !v.busy())
    });
    assert_eq!(first(h.state()), was, "a dry run changed nothing");
    assert!(
        h.state()
            .toast
            .as_ref()
            .is_some_and(|t| t.0.contains("nmcli")),
        "told what it would do: {:?}",
        h.state().toast
    );
    // A network: its page, then back.
    let net = h
        .state()
        .mods
        .page
        .as_ref()
        .unwrap()
        .page
        .as_ref()
        .unwrap()
        .items
        .iter()
        .position(|it| matches!(&it.kind, Kind::Link { page, .. } if page.starts_with("net:")));
    if let Some(n) = net {
        h.state_mut().mods.page.as_mut().unwrap().sel = n;
        h.state_mut().handle(Action::Accept);
        wait(&mut h, &|a: &App| {
            a.mods
                .page
                .as_ref()
                .is_some_and(|v| v.stack.len() == 2 && v.page.is_some())
        });
        snap(&mut h, &dir, "system-network");
        h.state_mut().handle(Action::Back);
        assert_eq!(h.state().mods.page.as_ref().unwrap().stack.len(), 1);
        wait(&mut h, &ready);
    }
    // A danger button asks first.
    let items = h
        .state()
        .mods
        .page
        .as_ref()
        .unwrap()
        .page
        .clone()
        .unwrap()
        .items;
    let danger = items
        .iter()
        .position(|it| matches!(it.kind, Kind::Button { danger: true, .. }))
        .unwrap();
    h.state_mut().mods.page.as_mut().unwrap().sel = danger;
    h.state_mut().handle(Action::Accept);
    assert!(h.state().confirm.is_some());
    snap(&mut h, &dir, "system-ask");
    h.state_mut().handle(Action::Back);
    assert!(h.state().confirm.is_none());
    // Back from the page: to its panel; again: home.
    h.state_mut().handle(Action::Back);
    assert!(h.state().mods.page.as_ref().unwrap().side);
    h.state_mut().handle(Action::Back);
    assert_eq!(h.state().page, Page::Home);
}

/// MangoHud: its hub in Modules (a window), its tab on a game's page;
/// turned on for a game, the launcher keeps the game's start part.
#[test]
#[ignore]
fn mangohud_module() {
    use crate::app::{Page, Tab};
    use crate::modules::Kind;
    let dir = std::env::var_os("LENTA_SHOT_DIR").map(PathBuf::from);
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let ex = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/modules");
    h.state_mut().mods = crate::ui::modules::Mods::load_from(vec![(only(&ex, "mangohud"), false)]);
    // Kept to Modules: no section of its own.
    assert!(
        !h.state()
            .pages()
            .iter()
            .any(|p| matches!(p, Page::Module(_)))
    );
    h.state_mut().go(Page::Modules);
    h.state_mut().handle(Action::Accept);
    let wait = |h: &mut Harness<'static, App>, w: &dyn Fn(&App) -> bool| {
        for _ in 0..300 {
            h.step();
            if w(h.state()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the module didn't answer");
    };
    wait(&mut h, &|a: &App| {
        a.mods
            .over
            .as_ref()
            .is_some_and(|v| v.page.is_some() && !v.busy())
    });
    snap(&mut h, &dir, "mangohud-hub");
    h.state_mut().handle(Action::Back);
    assert!(h.state().mods.over.is_none());
    // A Steam game's page: the MangoHud tab.
    let Some(key) = h
        .state()
        .games
        .iter()
        .find(|g| g.steam_appid.is_some())
        .map(|g| g.key.clone())
    else {
        return;
    };
    h.state_mut().go(Page::Home);
    h.state_mut().open_game(key.clone());
    let g = h.state().game_by_key(&key).unwrap().clone();
    let tabs = h.state().tabs_of(&g);
    let i = tabs
        .iter()
        .position(|t| matches!(t, Tab::Module(_)))
        .expect("MangoHud tab");
    {
        let v = h.state_mut().game.as_mut().unwrap();
        v.tab = tabs[i];
        v.focus = i + 1;
    }
    h.state_mut().handle(Action::Down);
    wait(&mut h, &|a: &App| {
        a.mods
            .game
            .as_ref()
            .is_some_and(|v| v.page.is_some() && !v.busy())
    });
    // On for this game: the launcher holds its part.
    h.state_mut().handle(Action::Accept);
    wait(&mut h, &|a: &App| {
        a.mods
            .game
            .as_ref()
            .is_some_and(|v| !v.busy() && v.page.as_ref().is_some_and(|p| p.items.len() > 1))
    });
    assert!(
        h.state().launch.wants(&key),
        "the game's start has MangoHud"
    );
    // Its own settings.
    h.state_mut().handle(Action::Down);
    let at = h
        .state()
        .mods
        .game
        .as_ref()
        .unwrap()
        .page
        .as_ref()
        .unwrap()
        .items
        .iter()
        .position(|it| it.id == "own");
    if let Some(at) = at {
        h.state_mut().mods.game.as_mut().unwrap().sel = at;
        h.state_mut().handle(Action::Right);
        wait(&mut h, &|a: &App| {
            a.mods.game.as_ref().is_some_and(|v| {
                !v.busy()
                    && v.page
                        .as_ref()
                        .is_some_and(|p| p.items.iter().any(|i| i.id == "g.style"))
            })
        });
    }
    snap(&mut h, &dir, "mangohud-game");
    // Off again: nothing kept for the game.
    h.state_mut().mods.game.as_mut().unwrap().sel = 0;
    h.state_mut().handle(Action::Accept);
    wait(&mut h, &|a: &App| {
        a.mods.game.as_ref().is_some_and(|v| !v.busy())
    });
    assert!(!h.state().launch.wants(&key));
    let first = h
        .state()
        .mods
        .game
        .as_ref()
        .unwrap()
        .page
        .as_ref()
        .unwrap()
        .items[0]
        .kind
        .clone();
    assert_eq!(first, Kind::Toggle(false));
}

/// The Launch tab: the options as Steam (or Lutris) has them, changed
/// here. A dry run: nothing is written anywhere.
#[test]
#[ignore]
fn launch_options() {
    use crate::app::Tab;
    let dir = std::env::var_os("LENTA_SHOT_DIR").map(PathBuf::from);
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let keys: Vec<String> = h
        .state()
        .games
        .iter()
        .filter(|g| g.key.starts_with("steam:") || g.key.starts_with("lutris:"))
        .map(|g| g.key.clone())
        .collect();
    let steam = keys.iter().find(|k| k.starts_with("steam:")).cloned();
    let lutris = keys.iter().find(|k| k.starts_with("lutris:")).cloned();
    for (key, name) in [(steam, "launch-steam"), (lutris, "launch-lutris")] {
        let Some(key) = key else { continue };
        h.state_mut().open_game(key.clone());
        let g = h.state().game_by_key(&key).unwrap().clone();
        let tabs = h.state().tabs_of(&g);
        let i = tabs
            .iter()
            .position(|t| *t == Tab::Launch)
            .expect("Launch tab");
        {
            let v = h.state_mut().game.as_mut().unwrap();
            v.tab = Tab::Launch;
            v.focus = i + 1;
        }
        h.state_mut().handle(Action::Down);
        settle(&mut h, 3);
        let before = h.state().launch.get(&key).and_then(|s| s.line.clone());
        assert!(before.is_some(), "looked at the launcher's options");
        // Typed on the keyboard: " -novid" at the end.
        h.state_mut().handle(Action::Accept);
        assert!(h.state().osk.is_some());
        h.input_mut()
            .events
            .push(egui::Event::Text(" -novid".into()));
        settle(&mut h, 2);
        h.state_mut().handle(Action::Menu);
        assert!(h.state().osk.is_none());
        let after = h
            .state()
            .launch
            .get(&key)
            .and_then(|s| s.line.clone())
            .unwrap();
        assert!(after.ends_with("-novid"), "{after:?}");
        snap(&mut h, &dir, name);
        h.state_mut().game = None;
    }
}

/// A game running: Play is Close (asks first); Steam updating it: the
/// button fills as it goes. The state is put in by hand (nothing runs).
#[test]
#[ignore]
fn game_running() {
    use crate::running::{Status, Work};
    let dir = std::env::var_os("LENTA_SHOT_DIR").map(PathBuf::from);
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let Some(key) = h
        .state()
        .games
        .iter()
        .find(|g| g.key.starts_with("steam:"))
        .map(|g| g.key.clone())
    else {
        return;
    };
    // Off the watcher's line: only what's put in here.
    let st: crate::running::Shared = Default::default();
    h.state_mut().running = st.clone();
    h.state_mut().open_game(key.clone());
    st.lock().unwrap().running.insert(key.clone());
    snap(&mut h, &dir, "game-running");
    assert!(
        h.state()
            .hint_items()
            .iter()
            .any(|(_, t)| *t == "Закрыть" || *t == "Close")
    );
    h.state_mut().handle(Action::Accept);
    assert!(h.state().confirm.is_some(), "asks before closing");
    h.state_mut().handle(Action::Back);
    // Ended: the next frame looks again.
    *st.lock().unwrap() = Status::default();
    st.lock().unwrap().work.insert(
        key.clone(),
        Work::Updating {
            done: 45,
            total: 100,
        },
    );
    snap(&mut h, &dir, "game-updating");
    assert!(h.state().was_running.is_empty());
}

/// Power from the side panel (asked, never run), a sideways touchpad
/// swipe turning the Library's tab, and SteamDB versions pasted in
/// (nothing saved under the dry run).
#[test]
#[ignore]
fn power_swipe_steamdb() {
    use crate::app::{Page, Tab};
    let dir = std::env::var_os("LENTA_SHOT_DIR").map(PathBuf::from);
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    // Power: the last thing in the side panel.
    h.state_mut().handle(Action::View);
    let n = h.state().pages().len();
    for _ in 0..n + 2 {
        h.state_mut().handle(Action::Down);
    }
    assert_eq!(h.state().side_sel, n);
    snap(&mut h, &dir, "power-side");
    h.state_mut().handle(Action::Accept);
    assert!(h.state().menu.is_some());
    snap(&mut h, &dir, "power-menu");
    h.state_mut().handle(Action::Down);
    h.state_mut().handle(Action::Accept);
    assert!(h.state().confirm.is_some(), "asks before restarting");
    snap(&mut h, &dir, "power-ask");
    h.state_mut().handle(Action::Back);
    assert!(h.state().confirm.is_none() && h.state().menu.is_none());

    // A swipe to the left on the Library: the next launcher, once.
    h.state_mut().go(Page::Library);
    settle(&mut h, 5);
    let before = h.state().lib.filter;
    let tabs = h.state().lib_tabs().len();
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(800.0, 500.0)));
    for _ in 0..12 {
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(-30.0, 0.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.step();
    }
    assert_eq!(
        h.state().lib.filter,
        (before + 1) % tabs,
        "one tab per swipe"
    );
    // Still, then another swipe the other way: back.
    std::thread::sleep(std::time::Duration::from_millis(350));
    settle(&mut h, 3);
    for _ in 0..12 {
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(30.0, 0.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.step();
    }
    assert_eq!(h.state().lib.filter, before);
    snap(&mut h, &dir, "swipe-library");

    // SteamDB: Graveyard Keeper 2 (allowed for tests), its Versions tab.
    let key = "steam:4358690".to_string();
    if !h.state().games.iter().any(|g| g.key == key && g.installed) {
        return;
    }
    h.state_mut().go(Page::Home);
    h.state_mut().open_game(key);
    if let Some(v) = h.state_mut().game.as_mut() {
        v.tab = Tab::Versions;
        v.focus = crate::ui::game_page::INSIDE;
    }
    for _ in 0..300 {
        h.step();
        if h.state().versions.data.is_some() && h.state().versions.loading.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    h.state_mut().open_steamdb_import();
    assert!(h.state().versions.import.is_some());
    snap(&mut h, &dir, "steamdb-empty");
    let depot = h.state().versions.import.as_ref().unwrap().depots[0]
        .0
        .clone();
    h.input_mut().events.push(egui::Event::Paste(format!(
        "Seen Date\tRelative\tManifest ID\n\
         1 March 2025 – 10:00:00 UTC\t8 months ago\t9876543210987654321\n\
         download_depot 4358690 {depot} 1234567890123456789\n\
         \nDate\tBuild ID\tPatch notes\n\
         13 September 2025 – 14:00:00 UTC\t15001000\tHotfix 1.0.5\n"
    )));
    settle(&mut h, 3);
    let im = h.state().versions.import.as_ref().unwrap();
    assert_eq!(im.entries.len(), 2);
    assert_eq!(im.builds.len(), 1);
    snap(&mut h, &dir, "steamdb-pasted");
    assert!(
        h.state()
            .hint_items()
            .iter()
            .any(|(_, t)| *t == "Вставить" || *t == "Paste")
    );
    h.state_mut().handle(Action::Back);
    assert!(h.state().versions.import.is_none());
}

/// Steam libraries, moving and uninstalling: only looked at (every
/// question answered Cancel; jobs do nothing under the dry run).
#[test]
#[ignore]
fn manage_games() {
    use crate::app::Page;
    let dir = std::env::var_os("LENTA_SHOT_DIR").map(PathBuf::from);
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    // Settings → Steam libraries.
    h.state_mut().go(Page::Settings);
    h.state_mut().open_libraries();
    settle(&mut h, 5);
    assert!(h.state().manage.pick.is_some());
    snap(&mut h, &dir, "manage-libraries");
    h.state_mut().handle(Action::Back);
    assert!(h.state().manage.pick.is_none());

    let key = "steam:4358690".to_string();
    if !h.state().games.iter().any(|g| g.key == key && g.installed) {
        return;
    }
    h.state_mut().go(Page::Home);
    h.state_mut()
        .open_menu(crate::ui::menu::MenuFor::Game(key.clone()), None);
    settle(&mut h, 3);
    snap(&mut h, &dir, "manage-menu");
    h.state_mut().menu = None;
    h.state_mut().manage_move(&key);
    settle(&mut h, 3);
    // Somewhere to go only with another drive plugged in.
    if h.state().manage.pick.is_some() {
        snap(&mut h, &dir, "manage-move");
        h.state_mut().handle(Action::Accept);
        if h.state().confirm.is_some() {
            snap(&mut h, &dir, "manage-move-ask");
            h.state_mut().handle(Action::Back);
        }
    } else {
        snap(&mut h, &dir, "manage-move-nowhere");
    }
    h.state_mut().manage.pick = None;
    h.state_mut().manage_uninstall(&key);
    assert!(h.state().confirm.is_some(), "asks before uninstalling");
    snap(&mut h, &dir, "manage-uninstall-ask");
    h.state_mut().handle(Action::Back);
    assert!(h.state().confirm.is_none() && h.state().manage.job.is_none());

    // The job's card, as one would look half way.
    let (_tx, rx) = crossbeam_channel::bounded(1);
    let progress = std::sync::Arc::new(std::sync::Mutex::new(crate::steamlib::Progress {
        done: 45 << 30,
        total: 100 << 30,
        stage: "mg_copying",
    }));
    h.state_mut().manage.job = Some(crate::ui::manage::Job {
        key: Some(key.clone()),
        label: "Перенос «Graveyard Keeper 2»".into(),
        progress,
        cancel: Default::default(),
        done: rx,
        then: None,
    });
    snap(&mut h, &dir, "manage-job");
    h.state_mut().manage.job = None;
}

/// Own programs: the Own tab's tiles, the prefixes, the file browser and
/// what-is-it question; nothing saved (closed with B).
#[test]
#[ignore]
fn own_programs() {
    use crate::app::Page;
    use crate::ui::library::LibTab;
    let dir = std::env::var_os("LENTA_SHOT_DIR").map(PathBuf::from);
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    h.state_mut().go(Page::Library);
    settle(&mut h, 5);
    let tabs = h.state().lib_tabs();
    let own = tabs
        .iter()
        .position(|t| *t == LibTab::Launcher(crate::game::Source::Own))
        .expect("the Own tab, even empty");
    h.state_mut().lib.filter = own;
    h.state_mut().libv.side = false;
    h.state_mut().lib.sel = 0;
    settle(&mut h, 5);
    snap(&mut h, &dir, "own-tab");
    // The second tile: Prefixes.
    h.state_mut().handle(Action::Right);
    h.state_mut().handle(Action::Accept);
    assert!(h.state().modals.choose.is_some(), "the prefixes");
    snap(&mut h, &dir, "own-prefixes");
    h.state_mut().handle(Action::Back);
    assert!(h.state().modals.choose.is_none());
    // The first: Add a program → the file browser on Downloads' places.
    h.state_mut().handle(Action::Left);
    h.state_mut().handle(Action::Accept);
    assert!(h.state().modals.files.is_some(), "the file browser");
    snap(&mut h, &dir, "own-files-places");
    h.state_mut().handle(Action::Down);
    h.state_mut().handle(Action::Accept);
    settle(&mut h, 2);
    snap(&mut h, &dir, "own-files-folder");
    h.state_mut().handle(Action::Menu);
    assert!(h.state().modals.files.is_none());
    // A file chosen: what is it (an installer by its name: that first).
    h.state_mut()
        .program_file(PathBuf::from("/tmp/setup_fox_1.2.exe"), None);
    snap(&mut h, &dir, "own-what-is-it");
    h.state_mut().handle(Action::Back);
    assert!(h.state().modals.choose.is_none());
}

/// The Mods module on a game's page, gathered with MangoHud into one
/// Modules tab (LB/RB between them). Needs `cargo build -p lenta-mods`.
#[test]
#[ignore]
fn mods_module() {
    use crate::app::{Page, Tab};
    let dir = std::env::var_os("LENTA_SHOT_DIR").map(PathBuf::from);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(bin) = ["target/release/lenta-mods", "target/debug/lenta-mods"]
        .iter()
        .map(|b| root.join(b))
        .find(|b| b.is_file())
    else {
        eprintln!("lenta-mods isn't built");
        return;
    };
    // Both modules in one folder: MangoHud's example and Mods (with its
    // program beside its module.json).
    let d = cfg_dir().join("modules-mods");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("mods")).unwrap();
    std::os::unix::fs::symlink(root.join("examples/modules/mangohud"), d.join("mangohud")).unwrap();
    std::fs::copy(
        root.join("modules/mods/module.json"),
        d.join("mods/module.json"),
    )
    .unwrap();
    std::os::unix::fs::symlink(&bin, d.join("mods/lenta-mods")).unwrap();
    let mut h = harness([1600.0, 900.0]);
    for _ in 0..200 {
        h.step();
        if !h.state().scanning {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    h.state_mut().mods = crate::ui::modules::Mods::load_from(vec![(d.clone(), false)]);
    let Some(key) = h
        .state()
        .games
        .iter()
        .find(|g| g.key == "steam:2244210")
        .or_else(|| h.state().games.iter().find(|g| g.steam_appid.is_some()))
        .map(|g| g.key.clone())
    else {
        return;
    };
    h.state_mut().go(Page::Home);
    h.state_mut().open_game(key.clone());
    let g = h.state().game_by_key(&key).unwrap().clone();
    let group = h.state().mod_group(&g).expect("two modules gathered");
    assert_eq!(group.len(), 2);
    let tabs = h.state().tabs_of(&g);
    assert_eq!(
        tabs.iter().filter(|t| matches!(t, Tab::Module(_))).count(),
        1,
        "one Modules tab"
    );
    // The Mods module first.
    let mods_i = group
        .iter()
        .copied()
        .find(|i| h.state().mods.list[*i as usize].id == "mods")
        .unwrap();
    h.state_mut().mod_group_show(mods_i);
    let i = h
        .state()
        .tabs_of(&g)
        .iter()
        .position(|t| matches!(t, Tab::Module(_)))
        .unwrap();
    h.state_mut().game.as_mut().unwrap().focus = i + 1;
    h.state_mut().handle(Action::Down);
    let wait = |h: &mut Harness<'static, App>| {
        for _ in 0..300 {
            h.step();
            if h.state()
                .mods
                .game
                .as_ref()
                .is_some_and(|v| v.page.is_some() && !v.busy())
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the module didn't answer");
    };
    wait(&mut h);
    settle(&mut h, 4);
    snap(&mut h, &dir, "mods-tab");
    // RB: MangoHud in the same tab.
    h.state_mut().handle(Action::NextTab);
    wait(&mut h);
    settle(&mut h, 4);
    assert_ne!(h.state().game.as_ref().unwrap().tab, Tab::Module(mods_i));
    snap(&mut h, &dir, "mods-group-next");
    let _ = std::fs::remove_dir_all(&d);
}
