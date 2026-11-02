//! The system's own programs: shown .desktop entries, each once.

use std::path::{Path, PathBuf};

use freedesktop_desktop_entry::{DesktopEntry, get_languages_from_env};

#[derive(Clone, Debug)]
pub struct App {
    pub id: String,
    pub name: String,
    pub comment: Option<String>,
    pub icon: Option<PathBuf>,
    pub group: Group,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Games,
    Internet,
    Media,
    Graphics,
    Office,
    Development,
    System,
    Other,
}

impl Group {
    pub const ALL: [Group; 8] = [
        Group::Games,
        Group::Internet,
        Group::Media,
        Group::Graphics,
        Group::Office,
        Group::Development,
        Group::System,
        Group::Other,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Group::Games => "grp_games",
            Group::Internet => "grp_internet",
            Group::Media => "grp_media",
            Group::Graphics => "grp_graphics",
            Group::Office => "grp_office",
            Group::Development => "grp_dev",
            Group::System => "grp_system",
            Group::Other => "grp_other",
        }
    }

    fn of(cats: &[&str]) -> Group {
        let has = |c: &str| cats.iter().any(|x| x.eq_ignore_ascii_case(c));
        if has("Game") {
            Group::Games
        } else if has("Network") || has("WebBrowser") {
            Group::Internet
        } else if has("AudioVideo") || has("Audio") || has("Video") {
            Group::Media
        } else if has("Graphics") {
            Group::Graphics
        } else if has("Office") {
            Group::Office
        } else if has("Development") {
            Group::Development
        } else if has("System") || has("Settings") || has("Utility") {
            Group::System
        } else {
            Group::Other
        }
    }
}

fn app_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = dirs::data_dir() {
        v.push(d.join("applications"));
        v.push(d.join("flatpak/exports/share/applications"));
    }
    let sys =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    v.extend(
        sys.split(':')
            .filter(|s| !s.is_empty())
            .map(|s| Path::new(s).join("applications")),
    );
    v.push("/var/lib/flatpak/exports/share/applications".into());
    v
}

/// Icon theme from gsettings, then GTK/KDE config files.
fn icon_theme() -> Option<String> {
    if let Ok(out) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_matches('\'')
            .to_string();
        if out.status.success() && !s.is_empty() {
            return Some(s);
        }
    }
    let cfg = dirs::config_dir()?;
    for (file, key) in [
        ("gtk-3.0/settings.ini", "gtk-icon-theme-name"),
        ("gtk-4.0/settings.ini", "gtk-icon-theme-name"),
        ("kdeglobals", "Theme"),
    ] {
        if let Ok(text) = std::fs::read_to_string(cfg.join(file)) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=')
                    && k.trim() == key
                    && !v.trim().is_empty()
                {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

fn find_icon(name: &str, theme: Option<&str>) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    let mut q = freedesktop_icons::lookup(name).with_size(128).with_cache();
    if let Some(t) = theme {
        q = q.with_theme(t);
    }
    q.find().or_else(|| {
        freedesktop_icons::lookup(name)
            .with_size(128)
            .with_cache()
            .find()
    })
}

pub fn apps() -> Vec<App> {
    let locales = get_languages_from_env();
    let theme = icon_theme();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for dir in app_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "desktop"))
            .collect();
        files.sort();
        for path in files {
            let id = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(de) = DesktopEntry::from_path(&path, Some(&locales)) else {
                continue;
            };
            if de.no_display()
                || de.hidden()
                || de.exec().is_none()
                || de.type_().is_some_and(|t| t != "Application")
                || id.starts_with("lenta")
            {
                continue;
            }
            let shown_here = |list: Option<Vec<&str>>| {
                list.map(|l| {
                    l.iter()
                        .any(|d| desktop.split(':').any(|c| c.eq_ignore_ascii_case(d)))
                })
            };
            if shown_here(de.only_show_in()) == Some(false)
                || shown_here(de.not_show_in()) == Some(true)
            {
                continue;
            }
            let Some(name) = de.name(&locales).map(|n| n.to_string()) else {
                continue;
            };
            let cats = de.categories().unwrap_or_default();
            out.push(App {
                id,
                name,
                comment: de.comment(&locales).map(|c| c.to_string()),
                icon: de.icon().and_then(|i| find_icon(i, theme.as_deref())),
                group: Group::of(&cats),
                path,
            });
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

impl App {
    pub fn launch(&self) -> Result<(), String> {
        let locales = get_languages_from_env();
        let de = DesktopEntry::from_path(&self.path, Some(&locales)).map_err(|e| e.to_string())?;
        let argv = de.parse_exec().map_err(|e| e.to_string())?;
        let argv: Vec<String> = if de.terminal() {
            let term = [
                "kitty",
                "foot",
                "alacritty",
                "wezterm",
                "konsole",
                "gnome-terminal",
                "xterm",
            ]
            .into_iter()
            .find(|t| crate::game::has_program(t))
            .ok_or("no terminal found")?;
            let mut v = vec![term.to_string(), "-e".into()];
            v.extend(argv);
            v
        } else {
            argv
        };
        let (prog, args) = argv.split_first().ok_or("empty Exec")?;
        crate::game::spawn(prog, args)
    }
}
