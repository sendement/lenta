//! lenta: a gamepad-first game launcher.

mod account;
mod app;
mod colls;
mod config;
mod downgrade;
mod fonts;
mod game;
mod i18n;
mod images;
mod input;
mod keyboard;
mod maint;
mod music;
mod net;
mod protonup;
mod sfx;
mod sgdb;
mod sources;
mod store;
mod ui;
mod vdf;

#[cfg(test)]
mod shots;

fn main() -> eframe::Result {
    // For development: one app's appinfo record.
    if std::env::args().nth(1).as_deref() == Some("--appinfo")
        && let Some(id) = std::env::args().nth(2).and_then(|a| a.parse().ok())
    {
        for root in sources::steam::roots() {
            let all = sources::steam::appinfo(&root, &[id].into_iter().collect());
            println!("{:#?}", all.get(&id));
        }
        return Ok(());
    }
    // Rolling back from the command line: --versions APP, --install APP N
    // (the Nth listed version: downloads what's missing, closes Steam,
    // installs, locks), --unlock APP [--validate].
    let args: Vec<String> = std::env::args().collect();
    if let [_, cmd, app, rest @ ..] = args.as_slice()
        && matches!(cmd.as_str(), "--versions" | "--install" | "--unlock")
    {
        let cfg = config::Config::load();
        i18n::set(cfg.lang);
        return downgrade_cli(cmd, app, rest);
    }
    // SteamGridDB's API key through the Steam sign-in: --sgdb [NAME]
    // (NAME: create the account with it when there is none).
    if args.get(1).map(String::as_str) == Some("--sgdb") {
        let Some(acc) = account::Account::load() else {
            eprintln!("sign in to Steam first");
            std::process::exit(1)
        };
        let run = || -> Result<String, String> {
            let (mut s, who) = sgdb::Session::sign_in(&acc)?;
            println!("signed in through Steam: {who:?}");
            if who == sgdb::Who::NoAccount {
                let Some(name) = args.get(2) else {
                    return Err(format!(
                        "no SteamGridDB account; suggested names: {:?}",
                        sgdb::name_choices(&acc.name)
                    ));
                };
                s.check_name(name)?;
                s.register(name)?;
                println!("account created: {:?}", s.whoami()?);
            }
            let key = s.api_key()?;
            if !sgdb::check_key(&key)? {
                return Err("the API did not accept the key".into());
            }
            sgdb::save_key(&key)?;
            Ok(format!(
                "key saved ({}…{}, {} chars)",
                &key[..4],
                &key[key.len() - 2..],
                key.len()
            ))
        };
        match run() {
            Ok(m) => println!("{m}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        }
        return Ok(());
    }
    // For development: why the account's game list does or doesn't load.
    if std::env::args().nth(1).as_deref() == Some("--owned") {
        match account::Account::load() {
            None => println!("not signed in (or the token expired)"),
            Some(a) => match account::fetch_owned(&a) {
                Ok(b) => println!("ok, {} bytes", b.len()),
                Err(e) => println!("{e}"),
            },
        }
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("--list") {
        // What was found, for checking the sources without the window.
        let cfg = config::Config::load();
        i18n::set(cfg.lang);
        let mut games = sources::scan(&cfg);
        sources::web::enrich(&mut games);
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
            if g.steam_appid.is_none() {
                println!(
                    "         store id {:?}, prefix {:?}, emulator save {:?}",
                    g.store_appid,
                    g.prefix,
                    g.emu.as_ref().map(|e| &e.file)
                );
            }
        }
        let apps = sources::desktop::apps();
        println!("{} games, {} system apps", games.len(), apps.len());
        let cols = sources::steamplay::collections();
        println!(
            "collections: {}",
            cols.iter()
                .map(|c| format!("{} ({})", c.name, c.apps.len()))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let tools = sources::steamplay::tools();
        println!(
            "tools: {}",
            tools
                .iter()
                .map(|t| t.display.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let map = sources::steamplay::mapping();
        println!(
            "tool per game: {} set, default {:?}",
            map.len(),
            map.get(&0)
        );
        println!(
            "steam account: {}",
            account::Account::load().map_or("-".into(), |a| a.name)
        );
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

fn downgrade_cli(cmd: &str, app: &str, rest: &[String]) -> eframe::Result {
    use downgrade::flow;
    let fail = |e: String| -> eframe::Result {
        eprintln!("{e}");
        std::process::exit(1)
    };
    if cmd == "--unlock" {
        return match flow::unlock(app, rest.iter().any(|a| a == "--validate")) {
            Ok(m) => {
                println!("{m}");
                Ok(())
            }
            Err(e) => fail(e),
        };
    }
    let Some(l) = flow::load(app) else {
        return fail(format!("{app}: not installed"));
    };
    if cmd == "--versions" {
        println!(
            "{} — build {}{}",
            l.game.name,
            l.game.buildid,
            if l.lock.is_some() { " (locked)" } else { "" }
        );
        for (i, v) in l.versions.iter().enumerate() {
            let t = flow::changes(v, &l.actual);
            let state = if t.is_empty() {
                "installed".to_string()
            } else if l.ready(&t) {
                "downloaded".to_string()
            } else {
                format!("{} depots to download", l.jobs(&t).len())
            };
            println!(
                "{i:>3}  {}  {}  v{}  [{}]  {state}",
                game::date_time(v.time.max(0) as u64),
                v.title,
                v.game_version,
                v.source.label()
            );
        }
        return Ok(());
    }
    let Some(n) = rest.first().and_then(|n| n.parse::<usize>().ok()) else {
        return fail("--install APP N".into());
    };
    let Some(v) = l.versions.get(n) else {
        return fail(format!("no version {n}"));
    };
    let target = flow::changes(v, &l.actual);
    if target.is_empty() {
        println!("already installed");
        return Ok(());
    }
    let jobs = l.jobs(&target);
    if !jobs.is_empty() {
        let Some(acc) = account::Account::load() else {
            return fail("sign in to Steam first".into());
        };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let last = std::sync::Mutex::new(std::time::Instant::now());
        let r = flow::download_all(&jobs, &acc, &cancel, &|p| {
            let mut l = last.lock().unwrap();
            if l.elapsed().as_millis() > 1000 {
                *l = std::time::Instant::now();
                eprintln!(
                    "depot {}/{} {} {}/{} MB, {} KB/s (reused {} MB, unchanged {} MB)",
                    p.depot + 1,
                    p.depots,
                    p.status,
                    p.stats.done() >> 20,
                    p.stats.total >> 20,
                    p.speed >> 10,
                    p.stats.reused >> 20,
                    p.stats.unchanged >> 20
                );
            }
        });
        if let Err(e) = r {
            return fail(e);
        }
        println!("downloaded");
    }
    let buildid = (v.exact && v.buildid != 0).then_some(v.buildid);
    match flow::install(app, &target, buildid) {
        Ok(m) => {
            println!("{m}");
            Ok(())
        }
        Err(e) => fail(e),
    }
}
