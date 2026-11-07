//! External modules: a folder with `module.json` and a program. The
//! launcher asks the program for its pages (JSON on stdin, JSON back on
//! stdout) and draws them itself, so a module looks and works like the rest
//! of the launcher, with the pad or the mouse. A module can have its own
//! section (or open only from Modules) and its own tab on a game's page.
//!
//! Modules are looked for in ~/.local/share/lenta/modules,
//! /usr/local/share/lenta/modules and /usr/share/lenta/modules (a package
//! puts its folder there; removing the package takes the section away).
//! The protocol is in docs/modules.md.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use serde_json::{Value, json};

/// How long a module may take to answer.
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq)]
pub struct GameHook {
    /// The tab's name on the game's page.
    pub label: String,
    pub installed_only: bool,
    /// Only for games from these launchers ("steam", "heroic", "lutris");
    /// empty: any.
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub dir: PathBuf,
    pub exec: Vec<String>,
    /// A picture in the module's folder, or one of the launcher's icons by
    /// name.
    pub icon: Option<String>,
    /// It has pages of its own (its "main" page).
    pub pages: bool,
    /// It asks to be a section in the side panel (else only in Modules).
    pub wants_section: bool,
    pub game: Option<GameHook>,
    /// From a system folder (a package), not the user's own.
    pub system: bool,
}

impl Module {
    /// Its tab on this game's page, if it has one there.
    pub fn game_tab(&self, source: &str, installed: bool) -> Option<&GameHook> {
        let h = self.game.as_ref()?;
        if h.installed_only && !installed {
            return None;
        }
        if !h.sources.is_empty() && !h.sources.iter().any(|s| s == source) {
            return None;
        }
        Some(h)
    }

    /// The picture file, when the icon is one.
    pub fn icon_file(&self) -> Option<PathBuf> {
        let i = self.icon.as_ref()?;
        let f = self.dir.join(i);
        f.is_file().then_some(f)
    }
}

/// Where modules are looked for: the user's own first (it wins over a
/// system one with the same id); each with "is it a system folder".
pub fn dirs() -> Vec<(PathBuf, bool)> {
    if let Some(v) = std::env::var_os("LENTA_MODULES") {
        return std::env::split_paths(&v).map(|p| (p, false)).collect();
    }
    let mut v = Vec::new();
    if let Some(d) = ::dirs::data_dir() {
        v.push((d.join("lenta/modules"), false));
    }
    v.push((
        crate::sandbox::system("/usr/local/share/lenta/modules"),
        true,
    ));
    v.push((crate::sandbox::system("/usr/share/lenta/modules"), true));
    v
}

/// The user's modules folder (shown in Modules).
pub fn user_dir() -> Option<PathBuf> {
    dirs().into_iter().find(|d| !d.1).map(|d| d.0)
}

/// A text in the interface's language: a plain string, or
/// {"en": …, "ru": …}.
fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(m)) => {
            let lang = if crate::i18n::ru() { "ru" } else { "en" };
            m.get(lang)
                .or_else(|| m.get("en"))
                .or_else(|| m.values().next())
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        }
        _ => String::new(),
    }
}

fn read_module(dir: &Path, system: bool) -> Option<Module> {
    let v: Value = serde_json::from_slice(&std::fs::read(dir.join("module.json")).ok()?).ok()?;
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| Some(dir.file_name()?.to_string_lossy().into_owned()))?;
    let exec: Vec<String> = match v.get("exec") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|s| s.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    };
    if exec.is_empty() {
        return None;
    }
    let name = match text(v.get("name")) {
        n if n.is_empty() => id.clone(),
        n => n,
    };
    let game = match v.get("game") {
        Some(Value::Bool(true)) => Some(GameHook {
            label: name.clone(),
            installed_only: false,
            sources: Vec::new(),
        }),
        Some(g @ Value::Object(_)) => Some(GameHook {
            label: match text(g.get("label")) {
                l if l.is_empty() => name.clone(),
                l => l,
            },
            installed_only: g
                .get("installed_only")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            sources: g
                .get("sources")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_lowercase))
                        .collect()
                })
                .unwrap_or_default(),
        }),
        _ => None,
    };
    Some(Module {
        description: text(v.get("description")),
        version: v
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        dir: dir.to_path_buf(),
        exec,
        icon: v.get("icon").and_then(Value::as_str).map(str::to_string),
        pages: v.get("pages").and_then(Value::as_bool).unwrap_or(true),
        wants_section: v.get("show").and_then(Value::as_str) != Some("menu"),
        game,
        system,
        id,
        name,
    })
}

/// Every module in these folders, by name.
pub fn scan(dirs: &[(PathBuf, bool)]) -> Vec<Module> {
    let mut out: Vec<Module> = Vec::new();
    for (d, system) in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        let mut found: Vec<Module> = rd
            .flatten()
            .filter_map(|e| read_module(&e.path(), *system))
            .collect();
        found.sort_by(|a, b| a.dir.cmp(&b.dir));
        for m in found {
            if !out.iter().any(|o| o.id == m.id) {
                out.push(m);
            }
        }
    }
    out.sort_by_key(|m| m.name.to_lowercase());
    out
}

/// Where a module keeps its own things.
pub fn data_dir(id: &str) -> PathBuf {
    if std::env::var_os("LENTA_DRY_RUN").is_some() {
        return std::env::temp_dir().join(format!("lenta-dry-{}/{id}", std::process::id()));
    }
    ::dirs::data_dir()
        .unwrap_or_else(|| "/tmp".into())
        .join("lenta/module-data")
        .join(id)
}

/// Ask the module; the answer comes on the channel.
pub fn call(m: &Module, req: Value) -> Receiver<Result<Value, String>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let m = m.clone();
    std::thread::spawn(move || {
        let _ = tx.send(run(&m, &req));
    });
    rx
}

pub fn run(m: &Module, req: &Value) -> Result<Value, String> {
    run_in(m, req, &data_dir(&m.id))
}

/// Ask with this folder for its things.
pub fn run_in(m: &Module, req: &Value, data: &Path) -> Result<Value, String> {
    let _ = std::fs::create_dir_all(data);
    // A program in the module's folder, or one on PATH.
    let prog = {
        let own = m.dir.join(&m.exec[0]);
        if own.is_file() {
            own.into_os_string()
        } else {
            m.exec[0].clone().into()
        }
    };
    // Modules are the host's programs: inside Flatpak they run there.
    let mut cmd = Command::new(prog);
    cmd.args(&m.exec[1..])
        .current_dir(&m.dir)
        .env("LENTA_LANG", if crate::i18n::ru() { "ru" } else { "en" })
        .env("LENTA_MODULE_DIR", &m.dir)
        .env("LENTA_MODULE_DATA", data)
        .env("LENTA_VERSION", env!("CARGO_PKG_VERSION"));
    if let Some(v) = std::env::var_os("LENTA_DRY_RUN") {
        cmd.env("LENTA_DRY_RUN", v);
    }
    let mut ch = crate::sandbox::host(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", m.exec[0]))?;
    if let Some(mut i) = ch.stdin.take() {
        let _ = i.write_all(format!("{req}\n").as_bytes());
    }
    // Read both pipes aside: a chatty module mustn't stall on a full one.
    let reader = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut b);
            }
            b
        })
    };
    let out = reader(
        ch.stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let err = reader(
        ch.stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(s) = ch.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if start.elapsed() > TIMEOUT {
            let _ = ch.kill();
            let _ = ch.wait();
            return Err(crate::i18n::tr("mod_timeout").to_string());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    let text = String::from_utf8_lossy(&out);
    // The last line that is JSON: a module may print other things first.
    let answer = text
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .and_then(|l| serde_json::from_str::<Value>(l).ok())
        .or_else(|| serde_json::from_str::<Value>(text.trim()).ok());
    match answer {
        Some(v) => Ok(v),
        None => {
            let e = String::from_utf8_lossy(&err);
            let last = e.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
            Err(if !status.success() {
                format!("{status}: {last}")
            } else if last.is_empty() {
                crate::i18n::tr("mod_no_answer").to_string()
            } else {
                last.to_string()
            })
        }
    }
}

// ------------------------------------------------------------------ pages

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Header,
    Text,
    /// A name and its value, to read (a fact about the system).
    Info(String),
    Toggle(bool),
    Slider {
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        unit: String,
    },
    Choice {
        value: Value,
        options: Vec<(Value, String)>,
    },
    Button {
        text: String,
        danger: bool,
    },
    /// Another page of the module.
    Link {
        page: String,
        value: String,
    },
    /// A text typed on the on-screen keyboard (a password: dots).
    Input {
        value: String,
        secret: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub id: String,
    pub label: String,
    pub hint: String,
    pub disabled: bool,
}

impl Item {
    /// What it is, to find it again in a fresh page.
    pub fn key(&self) -> String {
        match &self.kind {
            Kind::Link { page, .. } => format!("link:{page}"),
            _ if !self.id.is_empty() => format!("id:{}", self.id),
            _ => format!("label:{}", self.label),
        }
    }

    pub fn selectable(&self) -> bool {
        !self.disabled && !matches!(self.kind, Kind::Header | Kind::Text | Kind::Info(_))
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ModPage {
    pub title: String,
    pub items: Vec<Item>,
}

fn num(v: &Value, k: &str, or: f64) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(or)
}

fn item(v: &Value) -> Option<Item> {
    let s = |k: &str| text(v.get(k));
    let kind = match v.get("type")?.as_str()? {
        "header" => Kind::Header,
        "text" => Kind::Text,
        "info" => Kind::Info(s("value")),
        "toggle" => Kind::Toggle(v.get("value").and_then(Value::as_bool).unwrap_or(false)),
        "slider" => {
            let min = num(v, "min", 0.0);
            let max = num(v, "max", 100.0).max(min + f64::EPSILON);
            Kind::Slider {
                value: num(v, "value", min).clamp(min, max),
                min,
                max,
                step: num(v, "step", 1.0).abs().max(f64::EPSILON),
                unit: s("unit"),
            }
        }
        "choice" => Kind::Choice {
            value: v.get("value").cloned().unwrap_or(Value::Null),
            options: v
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|o| match o {
                    Value::Object(_) => {
                        let val = o.get("value").cloned().unwrap_or(Value::Null);
                        let l = text(o.get("label"));
                        let l = if l.is_empty() { show(&val) } else { l };
                        (val, l)
                    }
                    other => (other.clone(), show(other)),
                })
                .collect(),
        },
        "button" => Kind::Button {
            text: s("text"),
            danger: v.get("danger").and_then(Value::as_bool).unwrap_or(false),
        },
        "link" => Kind::Link {
            page: v.get("page")?.as_str()?.to_string(),
            value: s("value"),
        },
        "input" => Kind::Input {
            value: s("value"),
            secret: v.get("secret").and_then(Value::as_bool).unwrap_or(false),
        },
        _ => return None,
    };
    let label = match s("label") {
        l if l.is_empty() => s("text"),
        l => l,
    };
    Some(Item {
        id: v
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        label,
        hint: s("hint"),
        disabled: v.get("disabled").and_then(Value::as_bool).unwrap_or(false),
        kind,
    })
}

/// A value as words.
pub fn show(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A page from an answer, if it has one.
pub fn page_of(v: &Value) -> Option<ModPage> {
    let items = v.get("items")?.as_array()?;
    Some(ModPage {
        title: text(v.get("title")),
        items: items.iter().filter_map(item).collect(),
    })
}

/// The slider's value moved by `d` steps.
pub fn slide(value: f64, min: f64, max: f64, step: f64, d: i32) -> f64 {
    // From off the grid of steps, the first step lands on it.
    let k = (value - min) / step;
    let k = match d.signum() {
        1 => (k + 1e-9).floor() + d as f64,
        -1 => (k - 1e-9).ceil() + d as f64,
        _ => k.round(),
    };
    (min + k * step).clamp(min, max)
}

/// A number as the module would write it.
pub fn num_value(v: f64, step: f64) -> Value {
    if step.fract() == 0.0 && v.fract() == 0.0 {
        json!(v as i64)
    } else {
        json!((v * 1e6).round() / 1e6)
    }
}

/// What an answer asks of the launcher.
#[derive(Default, Debug)]
pub struct Said {
    pub toast: Option<String>,
    /// A game's start changed: its key and the module's part (None: gone).
    pub launch: Option<(String, Option<crate::launch::Part>)>,
    pub restart_steam: bool,
}

/// A section in a module's own side panel.
#[derive(Clone, Debug, PartialEq)]
pub struct NavItem {
    pub page: String,
    pub label: String,
    /// A word beside it (a state: "On", a count).
    pub value: String,
    pub icon: Option<String>,
}

fn nav_of(v: &Value) -> Option<Vec<NavItem>> {
    Some(
        v.get("nav")?
            .as_array()?
            .iter()
            .filter_map(|n| {
                Some(NavItem {
                    page: n.get("page")?.as_str()?.to_string(),
                    label: text(n.get("label")),
                    value: text(n.get("value")),
                    icon: n.get("icon").and_then(Value::as_str).map(str::to_string),
                })
            })
            .collect(),
    )
}

// ------------------------------------------------------------------ a view

/// A question out: its page and op, and the answer to come.
type Running = (String, String, Receiver<Result<Value, String>>);

/// One module's page as shown: its pages so far, the choice, and the
/// questions to it, asked one at a time.
pub struct ModView {
    pub module: String,
    /// Which game it's about (the game's tab), as the module gets it.
    pub game: Option<Value>,
    pub game_key: Option<String>,
    /// Pages opened, the shown one last.
    pub stack: Vec<String>,
    pub page: Option<ModPage>,
    pub error: Option<String>,
    pub sel: usize,
    pub scroll: f32,
    pub follow: bool,
    queue: VecDeque<Value>,
    /// The question out now: its page and op, and the answer to come.
    running: Option<Running>,
    pub since: Instant,
    /// The module's own side panel (kept until it sends another).
    pub nav: Vec<NavItem>,
    /// The choice is in that panel.
    pub side: bool,
    /// Ask for the page again then (the module wants it fresh).
    refresh: Option<Instant>,
}

impl ModView {
    pub fn new(module: &str, game: Option<(String, Value)>, page: &str) -> ModView {
        let (game_key, game) = match game {
            Some((k, v)) => (Some(k), Some(v)),
            None => (None, None),
        };
        let mut v = ModView {
            module: module.to_string(),
            game,
            game_key,
            stack: vec![page.to_string()],
            page: None,
            error: None,
            sel: 0,
            scroll: 0.0,
            follow: false,
            queue: VecDeque::new(),
            running: None,
            since: Instant::now(),
            nav: Vec::new(),
            side: false,
            refresh: None,
        };
        v.fetch();
        v
    }

    pub fn current(&self) -> &str {
        self.stack.last().map(String::as_str).unwrap_or("main")
    }

    fn req(&self, op: &str) -> Value {
        let mut r = json!({
            "op": op,
            "page": self.current(),
            "lang": if crate::i18n::ru() { "ru" } else { "en" },
        });
        if let Some(g) = &self.game {
            r["game"] = g.clone();
        }
        r
    }

    /// Questions about pages no longer shown are left unasked (flicking
    /// through the panel mustn't queue every part on the way).
    fn drop_stale(&mut self) {
        let cur = self.current().to_string();
        self.queue
            .retain(|q| q["op"] != "page" || q["page"].as_str() == Some(cur.as_str()));
    }

    pub fn fetch(&mut self) {
        self.drop_stale();
        // One look is enough.
        if self
            .queue
            .iter()
            .any(|q| q["op"] == "page" && q["page"].as_str() == Some(self.current()))
        {
            return;
        }
        let r = self.req("page");
        self.queue.push_back(r);
    }

    pub fn set(&mut self, id: &str, value: Value) {
        // Its last value is all that matters (a slider held down).
        let cur = self.current().to_string();
        if let Some(q) = self.queue.iter_mut().rev().find(|q| {
            q["op"] == "set" && q["id"].as_str() == Some(id) && q["page"].as_str() == Some(&cur)
        }) {
            q["value"] = value;
            return;
        }
        let mut r = self.req("set");
        r["id"] = json!(id);
        r["value"] = value;
        self.queue.push_back(r);
    }

    pub fn press(&mut self, id: &str) {
        let mut r = self.req("press");
        r["id"] = json!(id);
        self.queue.push_back(r);
    }

    /// Open another of its pages.
    pub fn open(&mut self, page: &str) {
        self.stack.push(page.to_string());
        self.page = None;
        self.error = None;
        self.sel = 0;
        self.scroll = 0.0;
        self.fetch();
    }

    /// One of its side panel's sections: the pages start over from it.
    pub fn root(&mut self, page: &str) {
        if self.stack.len() == 1 && self.current() == page {
            return;
        }
        self.stack = vec![page.to_string()];
        self.page = None;
        self.error = None;
        self.sel = 0;
        self.scroll = 0.0;
        self.refresh = None;
        self.fetch();
    }

    /// Back a page; false when this is its first.
    pub fn back(&mut self) -> bool {
        if self.stack.len() < 2 {
            return false;
        }
        self.stack.pop();
        self.page = None;
        self.error = None;
        self.sel = 0;
        self.scroll = 0.0;
        self.fetch();
        true
    }

    /// When it wants the page asked again.
    pub fn refresh_in(&self) -> Option<Duration> {
        self.refresh
            .map(|t| t.saturating_duration_since(Instant::now()))
    }

    /// Waiting on the module.
    pub fn busy(&self) -> bool {
        self.running.is_some() || !self.queue.is_empty()
    }

    /// Take an answer, ask the next question; what the module asks of the
    /// launcher, if anything.
    pub fn pump(&mut self, m: &Module, shown: bool) -> Said {
        let mut said = Said::default();
        let mut toast = None;
        if let Some((page, op, rx)) = &self.running
            && let Ok(r) = rx.try_recv()
        {
            let (page, op) = (page.clone(), op.clone());
            self.running = None;
            match r {
                Err(e) => self.error = Some(e),
                Ok(v) => {
                    if let Some(e) = v.get("error").and_then(Value::as_str) {
                        self.error = Some(e.to_string());
                    } else if op == "page" {
                        self.error = None;
                    }
                    if let Some(n) = nav_of(&v) {
                        // The first time: the choice starts in the panel.
                        if self.nav.is_empty() && !n.is_empty() && self.stack.len() == 1 {
                            self.side = true;
                        }
                        self.nav = n;
                    }
                    if page == self.current() {
                        self.refresh = v
                            .get("refresh")
                            .and_then(Value::as_u64)
                            .map(|ms| Instant::now() + Duration::from_millis(ms.max(300)));
                    }
                    // A change to a game's start (null: its part gone).
                    if let Some(l) = v.get("launch") {
                        let key = l
                            .get("game")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| self.game_key.clone());
                        if let Some(key) = key {
                            // Nothing to add (null, or no env/wrap): the part goes.
                            let part = l.as_object().map(|_| crate::launch::Part {
                                env: l
                                    .get("env")
                                    .and_then(Value::as_object)
                                    .map(|e| e.iter().map(|(k, v)| (k.clone(), show(v))).collect())
                                    .unwrap_or_default(),
                                wrap: l
                                    .get("wrap")
                                    .and_then(Value::as_array)
                                    .map(|a| {
                                        a.iter()
                                            .filter_map(|w| w.as_str().map(str::to_string))
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                            });
                            let part = part.filter(|p| !p.env.is_empty() || !p.wrap.is_empty());
                            said.launch = Some((key, part));
                        }
                    }
                    said.restart_steam =
                        v.get("restart_steam").and_then(Value::as_bool) == Some(true);
                    toast = v
                        .get("toast")
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                        .map(str::to_string);
                    if let Some(to) = v.get("open").and_then(Value::as_str) {
                        self.open(to);
                    } else if v.get("back").and_then(Value::as_bool) == Some(true) {
                        self.back();
                    } else if let Some(p) = page_of(&v) {
                        // An answer about a page since left is old news.
                        if page == self.current() {
                            // The choice stays on the same thing, though
                            // the list moved (a fresh scan).
                            let was = self
                                .page
                                .as_ref()
                                .and_then(|o| o.items.get(self.sel))
                                .map(Item::key);
                            if let Some(i) =
                                was.and_then(|k| p.items.iter().position(|it| it.key() == k))
                            {
                                self.sel = i;
                            }
                            self.page = Some(p);
                            self.fix_sel();
                        }
                    } else if op != "page" {
                        self.fetch();
                    }
                }
            }
        }
        if shown
            && self.running.is_none()
            && self.queue.is_empty()
            && self.refresh.is_some_and(|t| Instant::now() >= t)
        {
            self.refresh = None;
            self.fetch();
        }
        if self.running.is_none()
            && let Some(q) = self.queue.pop_front()
        {
            let page = q["page"].as_str().unwrap_or_default().to_string();
            let op = q["op"].as_str().unwrap_or_default().to_string();
            self.since = Instant::now();
            self.running = Some((page, op, call(m, q)));
        }
        said.toast = toast;
        said
    }

    /// The choice on something that can be chosen.
    pub fn fix_sel(&mut self) {
        let Some(p) = &self.page else { return };
        let n = p.items.len();
        if n == 0 {
            self.sel = 0;
            return;
        }
        self.sel = self.sel.min(n - 1);
        if p.items[self.sel].selectable() {
            return;
        }
        if let Some(i) = (self.sel..n)
            .chain((0..self.sel).rev())
            .find(|i| p.items[*i].selectable())
        {
            self.sel = i;
        }
    }

    /// Move the choice by one, over what can't be chosen.
    pub fn step(&mut self, down: bool) {
        let Some(p) = &self.page else { return };
        let n = p.items.len();
        let mut i = self.sel;
        loop {
            if down {
                if i + 1 >= n {
                    return;
                }
                i += 1;
            } else {
                if i == 0 {
                    return;
                }
                i -= 1;
            }
            if p.items[i].selectable() {
                self.sel = i;
                self.follow = true;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/modules")
    }

    #[test]
    fn reads_modules_and_items() {
        let ms = scan(&[(demo_dir(), false)]);
        let m = ms.iter().find(|m| m.id == "demo").expect("demo module");
        assert!(m.pages && m.wants_section);
        assert!(m.game_tab("steam", true).is_some());
        let p = page_of(&json!({
            "title": "T",
            "items": [
                {"type": "header", "text": "H"},
                {"type": "toggle", "id": "a", "label": "A", "value": true},
                {"type": "slider", "id": "s", "label": "S", "value": 7, "min": 0, "max": 10, "step": 2},
                {"type": "choice", "id": "c", "label": "C", "value": "x",
                 "options": ["x", {"value": "y", "label": "Why"}]},
                {"type": "bogus"}
            ]
        }))
        .unwrap();
        assert_eq!(p.items.len(), 4);
        assert!(!p.items[0].selectable() && p.items[1].selectable());
        match &p.items[3].kind {
            Kind::Choice { options, .. } => assert_eq!(options[1].1, "Why"),
            k => panic!("{k:?}"),
        }
        assert_eq!(slide(7.0, 0.0, 10.0, 2.0, 1), 8.0);
        assert_eq!(slide(9.0, 0.0, 10.0, 2.0, 1), 10.0);
    }

    #[test]
    fn talks_to_the_demo() {
        let ms = scan(&[(demo_dir(), false)]);
        let m = ms.iter().find(|m| m.id == "demo").unwrap();
        let data = std::env::temp_dir().join(format!("lenta-mod-test-{}", std::process::id()));
        let v = run_in(
            m,
            &json!({"op": "page", "page": "main", "lang": "en"}),
            &data,
        )
        .unwrap();
        let p = page_of(&v).unwrap();
        assert!(!p.items.is_empty());
        let g = json!({"key": "steam:1", "title": "X"});
        let v = run_in(
            m,
            &json!({"op": "set", "page": "game", "id": "on", "value": true, "game": g}),
            &data,
        )
        .unwrap();
        let p = page_of(&v).unwrap();
        assert_eq!(p.items[0].kind, Kind::Toggle(true));
        let _ = std::fs::remove_dir_all(data);
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    #[test]
    fn flicking_asks_only_the_last() {
        let mut v = ModView::new("x", None, "a");
        // "a" is asked first (nothing runs yet: no pump), then three more.
        for p in ["b", "c", "d"] {
            v.root(p);
        }
        let pages: Vec<_> = v.queue.iter().map(|q| q["page"].clone()).collect();
        assert_eq!(pages, vec![json!("d")]);
        // A change waits its turn; a look at another page doesn't stay.
        v.set("on", json!(true));
        v.root("e");
        let ops: Vec<_> = v
            .queue
            .iter()
            .map(|q| {
                format!(
                    "{}:{}",
                    q["op"].as_str().unwrap(),
                    q["page"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(ops, vec!["set:d", "page:e"]);
    }
}
