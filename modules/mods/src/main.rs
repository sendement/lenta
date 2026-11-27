//! lenta-mods: a mod manager as a lenta module.
//!
//! A game's tab: mods put in by it (on, off, away; conflicts), mods found
//! in the game (UE4SS mods, paks: on and off), adding one from an archive
//! (Downloads, Nexus downloads, any file), Nexus Mods updates. Its own page:
//! the Nexus account and the "Mod Manager Download" links.
//!
//! `lenta-mods` answers lenta's questions (one JSON line in, one out);
//! `lenta-mods nxm <link>` takes a link from the browser;
//! `lenta-mods work <job>` does a job's work on its own.

mod layout;
mod nexus;
mod store;
mod work;

use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use layout::Layout;
use store::{Game, dry};
use work::Job;

struct Q {
    op: String,
    page: String,
    id: String,
    value: Value,
    game: Value,
    ru: bool,
}

fn t(ru: bool, en: &str, r: &str) -> String {
    if ru { r.to_string() } else { en.to_string() }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("work") => {
            if let Some(id) = args.get(2) {
                work::run(id);
            }
            return;
        }
        Some("nxm") => {
            if let Some(link) = args.get(2) {
                nxm(link);
            }
            return;
        }
        _ => {}
    }
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let ru = match s("lang").as_str() {
        "" => std::env::var("LENTA_LANG").is_ok_and(|l| l == "ru"),
        l => l == "ru",
    };
    let q = Q {
        op: s("op"),
        page: s("page"),
        id: s("id"),
        value: v.get("value").cloned().unwrap_or(Value::Null),
        game: v.get("game").cloned().unwrap_or(Value::Null),
        ru,
    };
    let out = answer(&q);
    println!("{out}");
}

/// From the browser: download, then (when the game is known) put it in.
fn nxm(link: &str) {
    let ru = std::env::var("LANG").is_ok_and(|l| l.starts_with("ru"));
    let notify = |s: &str| {
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "Lenta", "-i", "applications-games", "Lenta", s])
            .status();
    };
    let j = match work::start_download(link) {
        Ok(j) => j,
        Err(e) => return notify(&e),
    };
    notify(&t(
        ru,
        "Downloading from Nexus Mods…",
        "Загрузка с Nexus Mods…",
    ));
    work::run(&j.id);
    let Some(j) = Job::load(&j.id) else { return };
    if j.status != "done" {
        return notify(&format!(
            "{}: {}",
            t(ru, "Nexus download failed", "Не скачалось с Nexus"),
            j.message
        ));
    }
    // The game this Nexus game is (mods of it put in before): in at once.
    let domain = j
        .nexus
        .as_ref()
        .map(|n| n.domain.clone())
        .unwrap_or_default();
    let games: Vec<Game> = std::fs::read_dir(store::data().join("games"))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    serde_json::from_str::<Game>(
                        &std::fs::read_to_string(e.path().join("mods.json")).ok()?,
                    )
                    .ok()
                })
                .filter(|g| g.domain == domain && g.root.is_dir())
                .collect()
        })
        .unwrap_or_default();
    if let [g] = games.as_slice() {
        match work::start_install(
            &g.key,
            &g.root,
            &j.archive,
            j.nexus.clone(),
            Some((j.name.clone(), j.version.clone())),
        ) {
            Ok(_) => {
                j.forget();
                notify(&format!(
                    "{} {}",
                    t(ru, "Installing", "Устанавливается"),
                    j.name
                ));
            }
            Err(e) => notify(&e),
        }
    } else {
        notify(&format!(
            "{} {} — {}",
            t(ru, "Downloaded", "Скачано:"),
            j.name,
            t(
                ru,
                "install it from the game's Mods tab in Lenta",
                "установите на вкладке «Моды» игры в Ленте"
            )
        ));
    }
}

fn answer(q: &Q) -> Value {
    if q.game.is_object() {
        return game_answer(q);
    }
    main_answer(q)
}

// ------------------------------------------------------------ own page

fn main_answer(q: &Q) -> Value {
    let ru = q.ru;
    let mut cfg = nexus::Config::load();
    match (q.op.as_str(), q.id.as_str()) {
        ("set", "key") => {
            cfg.apikey = q.value.as_str().unwrap_or_default().trim().to_string();
            let msg = match nexus::validate(&cfg.apikey) {
                Ok((name, premium)) => {
                    cfg.user = name.clone();
                    cfg.premium = premium;
                    format!("{}: {name}", t(ru, "Signed in", "Вход выполнен"))
                }
                Err(e) => {
                    cfg.user.clear();
                    e
                }
            };
            let _ = cfg.save();
            return json!({ "toast": msg });
        }
        ("press", "nxm") => {
            return json!({ "toast": match nexus::register_handler() {
                Ok(()) => t(ru, "Nexus links now come to Lenta", "Ссылки Nexus теперь открываются в Ленте"),
                Err(e) => e,
            }});
        }
        ("press", id) if id.starts_with("forget:") => {
            if let Some(j) = Job::load(&id[7..]) {
                if !dry() && j.archive.starts_with(nexus::downloads()) {
                    let _ = std::fs::remove_file(&j.archive);
                }
                j.forget();
            }
            return json!({});
        }
        _ => {}
    }
    let mut items = vec![json!({"type": "header", "text": "Nexus Mods"})];
    items.push(json!({
        "type": "input", "id": "key", "secret": true,
        "label": t(ru, "API key", "API-ключ"),
        "value": cfg.apikey,
        "hint": t(ru, "From nexusmods.com → your account → API Keys (Personal API Key)", "С nexusmods.com → аккаунт → API Keys (Personal API Key)"),
    }));
    if !cfg.user.is_empty() {
        items.push(json!({"type": "info", "label": t(ru, "Account", "Аккаунт"),
            "value": format!("{}{}", cfg.user, if cfg.premium { " · Premium" } else { "" })}));
    }
    let ours = nexus::handler() == "lenta-nxm.desktop";
    items.push(json!({
        "type": "button", "id": "nxm",
        "label": t(ru, "\"Mod Manager Download\" opens in Lenta", "«Mod Manager Download» открывается в Ленте"),
        "text": if ours { t(ru, "Again", "Ещё раз") } else { t(ru, "Turn on", "Включить") },
        "hint": if ours { t(ru, "On: links from the site come here", "Включено: ссылки с сайта приходят сюда") }
                else { t(ru, "Without Premium files are downloaded from the site's button", "Без Premium файлы скачиваются кнопкой на сайте") },
    }));
    let dls: Vec<Job> = Job::all()
        .into_iter()
        .filter(|j| j.kind == "download")
        .collect();
    if !dls.is_empty() {
        items.push(json!({"type": "header", "text": t(ru, "Downloads", "Загрузки")}));
        for j in dls {
            items.push(job_item(&j, ru));
            if j.status != "running" {
                items.push(json!({"type": "button", "id": format!("forget:{}", j.id),
                    "label": j.name, "text": t(ru, "Remove", "Убрать"),
                    "hint": j.message}));
            }
        }
    }
    items.push(json!({"type": "text", "text": t(ru,
        "Mods are put in from a game's Mods tab. They're kept in the module's folder and linked into the game, so turning them off brings the game's own files back.",
        "Моды ставятся на вкладке «Моды» игры. Они хранятся в папке модуля и подключаются к игре ссылками, так что выключение возвращает игре её собственные файлы.")}));
    let running = Job::all().iter().any(|j| j.status == "running");
    let mut out = json!({"title": t(ru, "Mods", "Моды"), "items": items});
    if running {
        out["refresh"] = json!(1000);
    }
    out
}

fn job_item(j: &Job, ru: bool) -> Value {
    match j.status.as_str() {
        "running" => json!({"type": "progress", "label": j.name, "value": j.progress,
            "hint": format!("{}%", (j.progress * 100.0) as u32)}),
        "error" => json!({"type": "info", "label": j.name,
            "value": format!("{}: {}", t(ru, "failed", "ошибка"), j.message)}),
        _ => {
            json!({"type": "info", "label": j.name, "value": if j.version.is_empty() { j.message.clone() } else { j.version.clone() }})
        }
    }
}

// ------------------------------------------------------------ game tab

struct Ctx {
    key: String,
    title: String,
    root: PathBuf,
}

fn game_answer(q: &Q) -> Value {
    let ru = q.ru;
    let gs = |k: &str| {
        q.game
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let c = Ctx {
        key: gs("key"),
        title: gs("title"),
        root: PathBuf::from(gs("install_dir")),
    };
    if c.root.as_os_str().is_empty() || !c.root.is_dir() {
        return json!({"title": t(ru, "Mods", "Моды"), "items": [
            {"type": "text", "text": t(ru, "The game's folder isn't known.", "Папка игры неизвестна.")}]});
    }
    let l = Layout::detect(&c.root);
    let mut g = Game::load(&c.key, &c.root);
    if let Some(r) = act(q, &c, &l, &mut g) {
        return r;
    }
    let page = q.page.as_str();
    if page == "add" {
        return add_page(&c, &g, ru);
    }
    if let Some(id) = page.strip_prefix("mod:") {
        return mod_page(&g, id, ru);
    }
    main_page(&c, &l, &g, ru)
}

/// Doing what was asked; an answer when it's not just the page again.
fn act(q: &Q, c: &Ctx, l: &Layout, g: &mut Game) -> Option<Value> {
    let ru = q.ru;
    let id = q.id.as_str();
    let toast = |r: Result<(), String>, ok: String| {
        Some(json!({"toast": match r { Ok(()) => ok, Err(e) => e }}))
    };
    match q.op.as_str() {
        "set" => {
            let on = q.value.as_bool().unwrap_or(false);
            if let Some(m) = id.strip_prefix("m:") {
                return toast(
                    g.set_enabled(m, on),
                    if on {
                        t(ru, "On", "Включён")
                    } else {
                        t(ru, "Off", "Выключен")
                    },
                );
            }
            if let Some(f) = id.strip_prefix("f:") {
                return toast(
                    g.set_found(l, f, on),
                    if on {
                        t(ru, "On", "Включён")
                    } else {
                        t(ru, "Off", "Выключен")
                    },
                );
            }
            if id == "pick" {
                let p = PathBuf::from(q.value.as_str().unwrap_or_default());
                return Some(install(c, &p, None, ru));
            }
            None
        }
        "press" => {
            if let Some(path) = id.strip_prefix("file:") {
                return Some(install(c, Path::new(path), None, ru));
            }
            if let Some(jid) = id.strip_prefix("inst:") {
                let j = Job::load(jid)?;
                let r = install(c, &j.archive, Some(&j), ru);
                if !g.domain.is_empty() || j.nexus.is_some() {
                    j.forget();
                }
                return Some(r);
            }
            if let Some(rest) = id.strip_prefix("opt:") {
                let (jid, i) = rest.rsplit_once(':')?;
                let mut j = Job::load(jid)?;
                j.chosen = i.parse().ok();
                j.status = "running".into();
                j.save();
                let _ = work::spawn(&j.id);
                return Some(json!({}));
            }
            if let Some(jid) = id.strip_prefix("dismiss:") {
                Job::load(jid)?.forget();
                return Some(json!({}));
            }
            if let Some(m) = id.strip_prefix("remove:") {
                let r = g.remove(m);
                return Some(match r {
                    Ok(()) => json!({"toast": t(ru, "Removed", "Удалён"), "back": true}),
                    Err(e) => json!({"toast": e}),
                });
            }
            if let Some(m) = id.strip_prefix("check:") {
                return Some(check(g, m, ru));
            }
            if let Some(m) = id.strip_prefix("site:") {
                let r = g.get(m)?;
                let url = match (&r.nexus, r.nexus_id) {
                    (Some(n), _) => format!(
                        "https://www.nexusmods.com/{}/mods/{}?tab=files",
                        n.domain, n.mod_id
                    ),
                    (None, Some(mid)) if !g.domain.is_empty() => {
                        format!(
                            "https://www.nexusmods.com/{}/mods/{mid}?tab=files",
                            g.domain
                        )
                    }
                    _ => return Some(json!({"toast": t(ru, "Not from Nexus", "Не с Nexus")})),
                };
                open(&url);
                return Some(json!({}));
            }
            if id == "nexus" {
                if g.domain.is_empty() {
                    let cfg = nexus::Config::load();
                    match nexus::find_game(&cfg.apikey, &c.title) {
                        Ok(Some((domain, _))) => {
                            g.domain = domain;
                            let _ = g.save();
                        }
                        Ok(None) => {
                            open(&format!(
                                "https://www.nexusmods.com/search?keyword={}",
                                c.title.replace(' ', "+")
                            ));
                            return Some(
                                json!({"toast": t(ru, "Not found by name: opened the search", "По названию не нашлось: открыт поиск")}),
                            );
                        }
                        Err(e) => return Some(json!({"toast": e})),
                    }
                }
                open(&format!("https://www.nexusmods.com/{}/mods/", g.domain));
                return Some(json!({}));
            }
            None
        }
        _ => None,
    }
}

fn install(c: &Ctx, p: &Path, from: Option<&Job>, ru: bool) -> Value {
    if !p.is_file() {
        return json!({"toast": t(ru, "No such file", "Файла нет")});
    }
    let name = from.map(|j| (j.name.clone(), j.version.clone()));
    match work::start_install(&c.key, &c.root, p, from.and_then(|j| j.nexus.clone()), name) {
        Ok(_) => json!({"toast": t(ru, "Installing…", "Установка…"), "back": true}),
        Err(e) => json!({"toast": e}),
    }
}

fn check(g: &mut Game, id: &str, ru: bool) -> Value {
    let cfg = nexus::Config::load();
    let Some(r) = g.get(id).cloned() else {
        return json!({});
    };
    let (domain, mid) = match (&r.nexus, r.nexus_id) {
        (Some(n), _) => (n.domain.clone(), n.mod_id),
        (None, Some(m)) if !g.domain.is_empty() => (g.domain.clone(), m),
        _ => return json!({"toast": t(ru, "Not known on Nexus", "На Nexus не опознан")}),
    };
    match nexus::latest(&cfg.apikey, &domain, mid) {
        Ok(Some(v)) if !v.is_empty() && v != r.version => {
            if let Some(m) = g.mods.iter_mut().find(|m| m.id == id) {
                m.update = Some(v.clone());
            }
            let _ = g.save();
            json!({"toast": format!("{} {v}", t(ru, "Newer on Nexus:", "На Nexus есть новее:"))})
        }
        Ok(_) => {
            if let Some(m) = g.mods.iter_mut().find(|m| m.id == id) {
                m.update = None;
            }
            let _ = g.save();
            json!({"toast": t(ru, "Up to date", "Обновлений нет")})
        }
        Err(e) => json!({"toast": e}),
    }
}

fn open(url: &str) {
    let _ = std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn main_page(c: &Ctx, l: &Layout, g: &Game, ru: bool) -> Value {
    let mut items = Vec::new();
    let jobs: Vec<Job> = Job::all()
        .into_iter()
        .filter(|j| j.kind == "install" && j.game == c.key)
        .collect();
    let mut busy = false;
    for j in &jobs {
        match j.status.as_str() {
            "running" => {
                busy = true;
                items.push(json!({"type": "progress", "label": format!("{} {}", t(ru, "Installing", "Установка"), j.name),
                    "value": j.progress, "hint": format!("{}%", (j.progress * 100.0) as u32)}));
            }
            "choose" => {
                items.push(json!({"type": "header", "text": format!("{} «{}»", t(ru, "Which variant of", "Какой вариант"), j.name)}));
                for (i, o) in j.options.iter().enumerate() {
                    items.push(json!({"type": "button", "id": format!("opt:{}:{i}", j.id), "label": o, "text": t(ru, "Install", "Поставить")}));
                }
                items.push(json!({"type": "button", "id": format!("dismiss:{}", j.id), "label": t(ru, "None of them", "Никакой"), "text": t(ru, "Cancel", "Отмена")}));
            }
            "error" => {
                items.push(json!({"type": "button", "id": format!("dismiss:{}", j.id),
                    "label": format!("{} {}", t(ru, "Couldn't install", "Не установился"), j.name),
                    "hint": j.message, "text": "OK"}));
            }
            _ => {
                items.push(json!({"type": "button", "id": format!("dismiss:{}", j.id),
                    "label": format!("{} {}", t(ru, "Installed", "Установлен"), j.message), "text": "OK"}));
            }
        }
    }
    let kind = l.kind_name();
    let what = match (kind, l.ue4ss_mods.is_some(), l.bepinex) {
        ("Unreal", true, _) => t(
            ru,
            "Unreal Engine with UE4SS: paks and Lua/C++ mods",
            "Unreal Engine с UE4SS: pak-моды и Lua/C++ моды",
        ),
        ("Unreal", false, _) => t(
            ru,
            "Unreal Engine: pak mods (UE4SS isn't in: its archive installs like a mod)",
            "Unreal Engine: pak-моды (UE4SS нет: его архив ставится как мод)",
        ),
        ("Unity", _, true) => t(ru, "Unity with BepInEx", "Unity с BepInEx"),
        ("Unity", _, false) => t(
            ru,
            "Unity (BepInEx isn't in: its archive installs like a mod)",
            "Unity (BepInEx нет: его архив ставится как мод)",
        ),
        _ => t(
            ru,
            "Archives laid out like the game go in as they are",
            "Архивы, разложенные как игра, ставятся как есть",
        ),
    };
    items.push(json!({"type": "link", "page": "add", "label": t(ru, "Add a mod", "Добавить мод"), "hint": what}));
    if !g.mods.is_empty() {
        let on = g.mods.iter().filter(|m| m.enabled).count();
        items.push(json!({"type": "header", "text": format!("{} · {on}/{}", t(ru, "Installed by Lenta", "Установлены Лентой"), g.mods.len())}));
        let conf = g.conflicts();
        for m in &g.mods {
            let clash = conf.values().any(|ids| ids.contains(&m.id));
            let mut hint = vec![m.kind.clone()];
            if !m.version.is_empty() {
                hint.push(m.version.clone());
            }
            if let Some(u) = &m.update {
                hint.push(format!("{} {u}", t(ru, "newer:", "новее:")));
            }
            if clash {
                hint.push(t(
                    ru,
                    "⚠ shares files with another",
                    "⚠ пересекается с другим",
                ));
            }
            items.push(json!({"type": "toggle", "id": format!("m:{}", m.id), "label": m.name, "value": m.enabled, "hint": hint.join(" · ")}));
        }
        items.push(json!({"type": "header", "text": t(ru, "Each one", "Подробнее")}));
        for m in &g.mods {
            items.push(
                json!({"type": "link", "page": format!("mod:{}", m.id), "label": m.name,
                "value": if m.enabled { t(ru, "on", "вкл") } else { t(ru, "off", "выкл") }}),
            );
        }
    }
    let found = g.found(l);
    if !found.is_empty() {
        items.push(json!({"type": "header", "text": t(ru, "Found in the game", "Найдены в игре")}));
        for f in found {
            items.push(json!({"type": "toggle", "id": format!("f:{}", f.id), "label": f.name, "value": f.enabled, "hint": f.kind}));
        }
    }
    items.push(json!({"type": "header", "text": "Nexus Mods"}));
    items.push(json!({"type": "button", "id": "nexus",
        "label": if g.domain.is_empty() { t(ru, "Find the game on Nexus Mods", "Найти игру на Nexus Mods") } else { format!("nexusmods.com/{}", g.domain) },
        "text": t(ru, "Open", "Открыть"),
        "hint": t(ru, "\"Mod Manager Download\" there downloads into Lenta (turned on in Modules → Mods)", "«Mod Manager Download» там качает в Ленту (включается в Модули → Моды)")}));
    if dry() {
        items.push(json!({"type": "text", "text": t(ru, "Dry run: the game's files aren't changed.", "Пробный запуск: файлы игры не меняются.")}));
    }
    let mut out = json!({"title": t(ru, "Mods", "Моды"), "items": items});
    if busy {
        out["refresh"] = json!(800);
    }
    out
}

/// Archives in Downloads, newest first.
fn archives() -> Vec<(PathBuf, u64)> {
    let mut v = Vec::new();
    for d in [dirs::download_dir(), Some(nexus::downloads())]
        .into_iter()
        .flatten()
    {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let n = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if (n.ends_with(".zip") || n.ends_with(".7z") || n.ends_with(".rar")) && p.is_file() {
                let t = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                v.push((p, t));
            }
        }
    }
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    v.dedup_by(|a, b| a.0 == b.0);
    v
}

fn add_page(c: &Ctx, g: &Game, ru: bool) -> Value {
    let mut items = vec![json!({
        "type": "file", "id": "pick", "label": t(ru, "Choose an archive…", "Выбрать архив…"),
        "exts": [".zip", ".7z", ".rar"],
        "hint": t(ru, "zip, 7z or rar; where it goes is worked out from what's inside", "zip, 7z или rar; куда ставить, Лента поймёт по содержимому"),
    })];
    let dls: Vec<Job> = Job::all()
        .into_iter()
        .filter(|j| j.kind == "download" && j.status == "done")
        .filter(|j| g.domain.is_empty() || j.nexus.as_ref().is_some_and(|n| n.domain == g.domain))
        .collect();
    if !dls.is_empty() {
        items.push(
            json!({"type": "header", "text": t(ru, "Downloaded from Nexus", "Скачаны с Nexus")}),
        );
        for j in dls.iter().rev() {
            items.push(json!({"type": "button", "id": format!("inst:{}", j.id), "label": j.name,
                "hint": format!("{} · {}", j.version, j.message), "text": t(ru, "Install", "Поставить")}));
        }
    }
    let have: Vec<String> = g.mods.iter().map(|m| m.archive.clone()).collect();
    let list = archives();
    if !list.is_empty() {
        items.push(json!({"type": "header", "text": t(ru, "In Downloads", "В «Загрузках»")}));
        for (p, _) in list.into_iter().take(40) {
            let file = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let (name, version, _) = layout::parse_name(&file);
            let done = have.contains(&file);
            items.push(json!({"type": "button", "id": format!("file:{}", p.display()), "label": name,
                "hint": format!("{}{}", if version.is_empty() { String::new() } else { format!("{version} · ") }, file),
                "text": if done { t(ru, "Again", "Заново") } else { t(ru, "Install", "Поставить") }}));
        }
    }
    let _ = c;
    json!({"title": t(ru, "Add a mod", "Добавить мод"), "items": items})
}

fn mod_page(g: &Game, id: &str, ru: bool) -> Value {
    let Some(m) = g.get(id) else {
        return json!({"back": true});
    };
    let mut items = vec![
        json!({"type": "toggle", "id": format!("m:{}", m.id), "label": t(ru, "On", "Включён"), "value": m.enabled}),
        json!({"type": "info", "label": t(ru, "Kind", "Вид"), "value": m.kind}),
    ];
    if !m.version.is_empty() {
        items
            .push(json!({"type": "info", "label": t(ru, "Version", "Версия"), "value": m.version}));
    }
    items.push(json!({"type": "info", "label": t(ru, "From", "Из"), "value": m.archive}));
    if m.nexus.is_some() || m.nexus_id.is_some() {
        items.push(json!({"type": "button", "id": format!("check:{}", m.id), "label": t(ru, "Updates on Nexus", "Обновления на Nexus"),
            "text": t(ru, "Check", "Проверить"),
            "hint": m.update.as_ref().map(|u| format!("{} {u}", t(ru, "Newer:", "Новее:"))).unwrap_or_default()}));
        items.push(json!({"type": "button", "id": format!("site:{}", m.id), "label": t(ru, "Its page on Nexus", "Его страница на Nexus"), "text": t(ru, "Open", "Открыть")}));
    }
    items.push(json!({"type": "header", "text": format!("{} · {}", t(ru, "Files", "Файлы"), m.files.len())}));
    let shown: Vec<&String> = m.files.iter().take(24).collect();
    items.push(json!({"type": "text", "text": shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")}));
    items.push(json!({"type": "button", "id": format!("remove:{}", m.id), "label": t(ru, "Remove the mod", "Удалить мод"),
        "text": t(ru, "Remove", "Удалить"), "danger": true,
        "hint": t(ru, "Its files leave the game; the game's own come back", "Его файлы уйдут из игры, собственные файлы игры вернутся")}));
    json!({"title": m.name, "items": items})
}
