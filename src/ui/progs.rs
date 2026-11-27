//! Own programs from the pad: add one (an installer run in a prefix, or a
//! program as it is), prefixes made and looked after (a Proton or the
//! system's Wine, libraries through winetricks at one press, winecfg,
//! another file run in it), and a program's own settings.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::app::App;
use crate::i18n::tr;
use crate::progs::{self, Found, Prefix, Program, Store};
use crate::ui::choose::Row;

/// Where B goes from a list.
type Back = Box<dyn FnOnce(&mut App)>;

const RUNNABLE: [&str; 3] = [".exe", ".msi", ".bat"];

impl App {
    fn prefix_root(&self) -> PathBuf {
        progs::prefix_root(&self.cfg.prefix_root)
    }

    fn save_store(&mut self, f: impl FnOnce(&mut Store)) -> bool {
        match progs::edit(f) {
            Ok(_) => true,
            Err(e) => {
                self.toast(format!("{}: {e}", tr("mg_failed")));
                false
            }
        }
    }

    // ------------------------------------------------------------ adding

    /// "Add a program": a file, then what it is.
    pub fn add_program(&mut self) {
        let start = dirs::download_dir();
        self.pick_file(tr("pg_pick_file"), &RUNNABLE, start, |app, path| {
            app.program_file(path, None);
        });
    }

    /// An installer to run or a program to add, the likelier first; into
    /// `prefix` when given (from a prefix's own actions).
    pub fn program_file(&mut self, path: PathBuf, prefix: Option<Prefix>) {
        let installer = progs::looks_like_installer(&path);
        let inst = Row::new(tr("pg_is_installer"), tr("pg_is_installer_sub"));
        let prog = Row::new(tr("pg_is_program"), tr("pg_is_program_sub"));
        let rows = if installer {
            vec![inst, prog]
        } else {
            vec![prog, inst]
        };
        let note = path.display().to_string();
        let title = progs::title_for(&path);
        self.choose(tr("pg_what_is_it"), note, rows, move |app, i| {
            let as_installer = (i == 0) == installer;
            let suggest = title.clone();
            let go = move |app: &mut App, p: Prefix| {
                if as_installer {
                    app.run_installer(path, p, false);
                } else {
                    app.add_as_program(&title, path, &p);
                }
            };
            match prefix {
                Some(p) => go(app, p),
                None => app.pick_prefix(suggest, go),
            }
        });
    }

    fn add_as_program(&mut self, title: &str, exe: PathBuf, p: &Prefix) {
        let prog = Program {
            title: title.to_string(),
            exe,
            prefix: p.path.clone(),
            ..Default::default()
        };
        if self.save_store(|s| {
            s.add_program(prog);
        }) {
            self.toast(format!("{}: {title}", tr("pg_added")));
            self.rescan();
        }
    }

    /// A prefix to use: a new one (named, then its runner) or one made
    /// before.
    pub fn pick_prefix(&mut self, suggest: String, then: impl FnOnce(&mut App, Prefix) + 'static) {
        let s = Store::load();
        let counts = progs::counts(&s);
        let mut rows = vec![Row::new(
            tr("pg_new_prefix"),
            format!("{} {}", tr("pg_in"), self.prefix_root().display()),
        )];
        for p in &s.prefixes {
            rows.push(Row::new(
                p.name.clone(),
                format!(
                    "{}  ·  {}: {}",
                    progs::runner_name(&p.runner),
                    tr("pg_programs"),
                    counts.get(&p.path).copied().unwrap_or(0)
                ),
            ));
        }
        let list = s.prefixes.clone();
        self.choose(tr("pg_which_prefix"), "", rows, move |app, i| {
            if i == 0 {
                app.new_prefix(suggest, then);
            } else if let Some(p) = list.get(i - 1).cloned() {
                then(app, p);
            }
        });
    }

    /// Name it, choose what runs it; registered (made on first use).
    pub fn new_prefix(&mut self, suggest: String, then: impl FnOnce(&mut App, Prefix) + 'static) {
        self.type_text(
            tr("pg_prefix_name").to_string(),
            suggest,
            move |app, name| {
                if name.is_empty() {
                    return;
                }
                let shown = name.clone();
                app.pick_runner(&shown, None, move |app, runner| {
                    let root = app.prefix_root();
                    let base = progs::folder_name(&name);
                    let s = Store::load();
                    let mut path = root.join(&base);
                    let mut n = 2;
                    while path.exists() || s.prefix(&path).is_some() {
                        path = root.join(format!("{base} {n}"));
                        n += 1;
                    }
                    let p = Prefix {
                        name: name.clone(),
                        path,
                        runner,
                    };
                    let keep = p.clone();
                    if app.save_store(move |s| s.prefixes.push(keep)) {
                        then(app, p);
                    }
                });
            },
        );
    }

    /// A Proton (newest first), umu's own, the system's Wine last.
    pub fn pick_runner(
        &mut self,
        what: &str,
        now: Option<&str>,
        then: impl FnOnce(&mut App, String) + 'static,
    ) {
        let rs = progs::runners();
        if rs.is_empty() {
            self.toast(tr("pg_no_runner"));
            return;
        }
        let rows: Vec<Row> = rs
            .iter()
            .map(|r| {
                let sub = if r.id == "wine" {
                    tr("rn_wine_sub").to_string()
                } else if r.id.is_empty() {
                    tr("rn_umu_sub").to_string()
                } else if progs::wayland_session() && !progs::proton_wayland(&r.id) {
                    format!("{}  ·  {}", r.id, tr("rn_no_wayland"))
                } else {
                    r.id.clone()
                };
                let mut row = Row::new(r.name.clone(), sub);
                if now == Some(r.id.as_str()) {
                    row.title = format!("{}   ✓", row.title);
                }
                row
            })
            .collect();
        self.choose(
            format!("{} «{what}»", tr("pg_runner_for")),
            "",
            rows,
            move |app, i| {
                if let Some(r) = rs.get(i) {
                    then(app, r.id.clone());
                }
            },
        );
    }

    // -------------------------------------------------------- installing

    /// Run an installer in the prefix (made first if it's new), then find
    /// what it installed: one program taken at once, several to choose
    /// from, none — picked by hand.
    pub fn run_installer(&mut self, installer: PathBuf, p: Prefix, _again: bool) {
        if self.busy() {
            return;
        }
        let found: Arc<Mutex<Option<Vec<Found>>>> = Arc::default();
        let out = found.clone();
        let label = format!(
            "{} «{}»",
            tr("pg_installing"),
            installer.file_name().unwrap_or_default().to_string_lossy()
        );
        let pfx = p.clone();
        let inst = installer.clone();
        let wayland = progs::wayland_session();
        self.start_job(
            None,
            label,
            move |report, _| {
                let stage = |s: &'static str| {
                    report(crate::steamlib::Progress {
                        done: 0,
                        total: 0,
                        stage: s,
                    })
                };
                if !pfx.path.join("drive_c").is_dir() {
                    stage("pg_making_prefix");
                    progs::make_prefix(&pfx)?;
                }
                let before = progs::snapshot(&pfx.path);
                stage("pg_running_installer");
                let r = progs::run_wait(&progs::on_wayland(
                    progs::command(
                        &pfx.path,
                        &pfx.runner,
                        &inst.display().to_string(),
                        &[],
                        "umu-default",
                    ),
                    &pfx.runner,
                    wayland,
                ));
                let after = progs::snapshot(&pfx.path);
                let f = progs::found(&pfx.path, &before, &after);
                if f.is_empty()
                    && let Err(e) = r
                {
                    return Err(e);
                }
                *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
                Ok(tr("pg_installed").to_string())
            },
            move |app, r| {
                if r.is_err() {
                    return;
                }
                let Some(f) = found.lock().unwrap_or_else(|e| e.into_inner()).take() else {
                    return;
                };
                app.after_install(f, p);
            },
        );
    }

    fn after_install(&mut self, f: Vec<Found>, p: Prefix) {
        if f.len() == 1 {
            let one = f.into_iter().next().unwrap_or_else(|| unreachable!());
            self.add_found(one, &p);
            return;
        }
        let mut rows: Vec<Row> = f
            .iter()
            .map(|x| Row::new(x.title.clone(), x.exe.display().to_string()))
            .collect();
        rows.push(Row::new(tr("pg_pick_by_hand"), tr("pg_pick_by_hand_sub")));
        let note = if f.is_empty() {
            tr("pg_none_found")
        } else {
            tr("pg_several_found")
        };
        self.choose(tr("pg_what_to_run"), note, rows, move |app, i| {
            match f.get(i).cloned() {
                Some(one) => app.add_found(one, &p),
                None => app.pick_exe_in(p),
            }
        });
    }

    fn add_found(&mut self, f: Found, p: &Prefix) {
        let prog = Program {
            title: f.title.clone(),
            exe: f.exe,
            args: f.args,
            prefix: p.path.clone(),
            ..Default::default()
        };
        if self.save_store(|s| {
            s.add_program(prog);
        }) {
            self.toast(format!("{}: {}", tr("pg_added"), f.title));
            self.rescan();
        }
    }

    /// The program picked by hand, in the prefix's drive.
    fn pick_exe_in(&mut self, p: Prefix) {
        let start = p.path.join("drive_c");
        self.pick_file(
            tr("pg_pick_exe"),
            &[".exe", ".bat"],
            Some(start),
            move |app, exe| {
                let title = progs::title_for(&exe);
                app.add_as_program(&title, exe, &p);
            },
        );
    }

    // ---------------------------------------------------------- prefixes

    /// Settings → Prefixes, or the Own tab's tile.
    pub fn open_prefixes(&mut self) {
        let s = Store::load();
        let counts = progs::counts(&s);
        let mut rows = vec![Row::new(
            tr("pg_new_prefix"),
            format!("{} {}", tr("pg_in"), self.prefix_root().display()),
        )];
        for p in &s.prefixes {
            let made = p.path.join("drive_c").is_dir();
            rows.push(Row::new(
                p.name.clone(),
                format!(
                    "{}  ·  {}: {}  ·  {}",
                    progs::runner_name(&p.runner),
                    tr("pg_programs"),
                    counts.get(&p.path).copied().unwrap_or(0),
                    if made {
                        p.path.display().to_string()
                    } else {
                        tr("pg_not_made").to_string()
                    }
                ),
            ));
        }
        let list = s.prefixes.clone();
        self.choose(
            tr("pg_prefixes"),
            tr("pg_prefixes_note"),
            rows,
            move |app, i| {
                if i == 0 {
                    app.new_prefix(String::new(), |app, p| {
                        app.prefix_actions(p);
                    });
                } else if let Some(p) = list.get(i - 1).cloned() {
                    app.prefix_actions(p);
                }
            },
        );
    }

    pub fn prefix_actions(&mut self, p: Prefix) {
        let rows = vec![
            Row::new(tr("pg_tricks"), tr("pg_tricks_sub")),
            Row::new(tr("pg_install_here"), ""),
            Row::new(tr("pg_run_file"), tr("pg_run_file_sub")),
            Row::new(tr("pg_winecfg"), ""),
            Row::new(tr("pg_change_runner"), progs::runner_name(&p.runner)),
            Row::new(tr("pg_open_folder"), p.path.display().to_string()),
            Row::new(tr("pg_delete_prefix"), ""),
        ];
        let title = format!("{} «{}»", tr("pg_prefix"), p.name);
        self.choose(title, "", rows, move |app, i| match i {
            0 => {
                let q = p.clone();
                let t = progs::Tricks::Prefix {
                    name: p.name.clone(),
                    path: p.path.clone(),
                    runner: p.runner.clone(),
                };
                app.pick_tricks(
                    t,
                    Some(Box::new(move |app: &mut App| app.prefix_actions(q))),
                );
            }
            1 => {
                let q = p.clone();
                app.pick_file(
                    tr("pg_pick_file"),
                    &RUNNABLE,
                    dirs::download_dir(),
                    move |app, f| {
                        app.program_file(f, Some(q));
                    },
                );
            }
            2 => {
                let q = p.clone();
                app.pick_file(
                    tr("pg_pick_file"),
                    &RUNNABLE,
                    dirs::download_dir(),
                    move |app, f| {
                        app.run_in_prefix(&q, &f.display().to_string(), &[]);
                    },
                );
            }
            3 => app.run_in_prefix(&p, "winecfg", &[]),
            4 => {
                let path = p.path.clone();
                let name = p.name.clone();
                let now = p.runner.clone();
                app.pick_runner(&name, Some(&now), move |app, runner| {
                    if app.save_store(|s| {
                        if let Some(x) = s.prefixes.iter_mut().find(|x| x.path == path) {
                            x.runner = runner;
                        }
                    }) {
                        app.rescan();
                    }
                });
            }
            5 => {
                let _ = std::fs::create_dir_all(&p.path);
                let _ = crate::game::run_first(&[vec![
                    "xdg-open".into(),
                    p.path.display().to_string(),
                ]]);
            }
            _ => app.delete_prefix(p),
        });
        let back = |app: &mut App| app.open_prefixes();
        self.choose_back(back);
    }

    /// Something else run in the prefix, not waited for (a patch, winecfg).
    fn run_in_prefix(&mut self, p: &Prefix, exe: &str, args: &[String]) {
        let cmd = progs::on_wayland(
            progs::command(&p.path, &p.runner, exe, args, "umu-default"),
            &p.runner,
            progs::wayland_session(),
        );
        match crate::game::run_first(&[cmd]) {
            Ok(()) => self.toast(format!("{} {exe}", tr("pg_started"))),
            Err(e) => self.toast(format!("{}: {e}", tr("launch_failed"))),
        }
    }

    /// DLL mods beside a game (from its menu): what's there; for an own
    /// program they load by themselves, for another launcher's game their
    /// loading is allowed (or taken back) in its prefix.
    pub fn dll_mods(&mut self, key: &str) {
        use crate::dllmods;
        let Some(g) = self.game_by_key(key).cloned() else {
            return;
        };
        let own = g.source == crate::game::Source::Own;
        let exe = if own {
            progs::id_of(key).and_then(|id| Store::load().program(id).map(|p| p.exe.clone()))
        } else {
            None
        };
        let found = dllmods::find(&dllmods::dirs_of(g.install_dir.as_deref(), exe.as_deref()));
        if found.is_empty() {
            self.toast(tr("dm_none"));
            return;
        }
        let what = |f: &dllmods::Found| {
            let kind = if f.kinds.is_empty() {
                tr("dm_unknown").to_string()
            } else {
                f.kinds.join(", ")
            };
            let dlls: Vec<String> = f.dlls.iter().map(|d| format!("{d}.dll")).collect();
            format!("{kind}: {}", dlls.join(", "))
        };
        let title = format!("{} «{}»", tr("dm_title"), g.title);
        if own {
            let rows: Vec<Row> = found
                .iter()
                .map(|f| Row::new(format!("{}   ✓", what(f)), f.dir.display().to_string()))
                .collect();
            self.choose(title, tr("dm_own_note"), rows, |_, _| {});
            return;
        }
        let Some(prefix) = crate::maint::prefix(&g) else {
            self.toast(tr("tk_no_prefix"));
            return;
        };
        let reg = dllmods::user_reg(&prefix)
            .and_then(|r| std::fs::read_to_string(r).ok())
            .unwrap_or_default();
        let all = found.iter().all(|f| dllmods::allowed(&reg, f));
        let mut rows: Vec<Row> = found
            .iter()
            .map(|f| {
                let on = dllmods::allowed(&reg, f);
                Row::new(
                    format!("{}{}", what(f), if on { "   ✓" } else { "" }),
                    format!("{}  ·  {}", f.dir.display(), f.exes.join(", ")),
                )
            })
            .collect();
        let n = rows.len();
        rows.push(Row::new(
            if all { tr("dm_off") } else { tr("dm_on") },
            if all {
                tr("dm_off_sub")
            } else {
                tr("dm_on_sub")
            },
        ));
        let key = key.to_string();
        self.choose(title, tr("dm_note"), rows, move |app, i| {
            if i < n {
                return;
            }
            if app.is_running(&key) {
                app.toast(tr("mg_close_game"));
                return;
            }
            match dllmods::apply(&prefix, &found, !all) {
                Ok(()) => app.toast(if all {
                    tr("dm_taken")
                } else {
                    tr("dm_allowed")
                }),
                Err(e) => app.toast(format!("{}: {e}", tr("mg_failed"))),
            }
        });
    }

    pub fn can_dll_mods(&self, key: &str) -> bool {
        let Some(g) = self.game_by_key(key) else {
            return false;
        };
        if !g.installed {
            return false;
        }
        let exe = (g.source == crate::game::Source::Own)
            .then(|| progs::id_of(key))
            .flatten()
            .and_then(|id| Store::load().program(id).map(|p| p.exe.clone()));
        crate::dllmods::any(crate::dllmods::dirs_of(
            g.install_dir.as_deref(),
            exe.as_deref(),
        ))
    }

    /// Libraries for a game's prefix, from its menu.
    pub fn game_tricks(&mut self, key: &str) {
        let Some(g) = self.game_by_key(key).cloned() else {
            return;
        };
        match progs::tricks_for(&g) {
            Some(t) => self.pick_tricks(t, None),
            None => self.toast(tr("tk_no_prefix")),
        }
    }

    pub fn can_tricks(&self, key: &str) -> bool {
        self.game_by_key(key).is_some_and(|g| {
            g.installed
                && (g.prefix.is_some()
                    || g.compat_id.is_some()
                    || g.source == crate::game::Source::Own)
        })
    }

    /// The sets at one press (marked when winetricks' log has them all),
    /// or verbs typed.
    fn pick_tricks(&mut self, t: progs::Tricks, back: Option<Back>) {
        if !crate::sandbox::has_program(t.tool()) {
            self.toast(format!("{}: {}", tr("tk_missing"), t.tool()));
            return;
        }
        let have = progs::installed_verbs(t.prefix());
        let mut rows: Vec<Row> = progs::TRICKS
            .iter()
            .map(|(k, verbs)| {
                let done = verbs.iter().all(|v| have.contains(*v));
                let title = if done {
                    format!("{}   ✓", tr(k))
                } else {
                    tr(k).to_string()
                };
                Row::new(title, verbs.join(" "))
            })
            .collect();
        rows.push(Row::new(tr("tk_own"), tr("tk_own_sub")));
        let title = format!("{} «{}»", tr("pg_tricks"), t.name());
        let note = match &t {
            progs::Tricks::Steam { .. } => tr("tk_note_steam"),
            _ => tr("tk_note"),
        };
        self.choose(title, note, rows, move |app, i| {
            match progs::TRICKS.get(i) {
                Some((k, verbs)) => {
                    let v: Vec<String> = verbs.iter().map(|s| s.to_string()).collect();
                    app.run_tricks(t, v, tr(k).to_string());
                }
                None => {
                    app.type_text(tr("tk_own").to_string(), String::new(), move |app, txt| {
                        let v = progs::split_args(&txt);
                        if !v.is_empty() {
                            let label = v.join(" ");
                            app.run_tricks(t, v, label);
                        }
                    });
                }
            }
        });
        if let Some(b) = back {
            self.choose_back(b);
        }
    }

    fn run_tricks(&mut self, t: progs::Tricks, verbs: Vec<String>, what: String) {
        if self.busy() {
            return;
        }
        let label = format!("{} «{}»: {what}", tr("tk_installing"), t.name());
        self.start_job(
            None,
            label,
            move |report, _| {
                if let progs::Tricks::Prefix { name, path, runner } = &t
                    && !path.join("drive_c").is_dir()
                {
                    report(crate::steamlib::Progress {
                        done: 0,
                        total: 0,
                        stage: "pg_making_prefix",
                    });
                    progs::make_prefix(&Prefix {
                        name: name.clone(),
                        path: path.clone(),
                        runner: runner.clone(),
                    })?;
                }
                report(crate::steamlib::Progress {
                    done: 0,
                    total: 0,
                    stage: "tk_running",
                });
                progs::run_wait(&t.argv(&verbs))?;
                Ok(tr("tk_done").to_string())
            },
            |_, _| {},
        );
    }

    fn delete_prefix(&mut self, p: Prefix) {
        let s = Store::load();
        let users: Vec<String> = s
            .programs
            .iter()
            .filter(|x| x.prefix == p.path)
            .map(|x| x.title.clone())
            .collect();
        let mut lines = Vec::new();
        if p.path.exists() {
            lines.push(format!(
                "{}: {} ({})",
                tr("pg_to_trash"),
                p.path.display(),
                crate::maint::human(crate::maint::size(&p.path))
            ));
        }
        if !users.is_empty() {
            lines.push(format!("{}: {}", tr("pg_also_off_list"), users.join(", ")));
        }
        let title = format!("{} «{}»?", tr("pg_delete_prefix"), p.name);
        self.ask(title, lines, tr("mg_uninstall"), move |app| {
            if p.path.exists()
                && std::env::var_os("LENTA_DRY_RUN").is_none()
                && let Err(e) = crate::maint::trash(&p.path)
            {
                app.toast(format!("{}: {e}", tr("mg_failed")));
                return;
            }
            let path = p.path.clone();
            if app.save_store(|s| {
                s.prefixes.retain(|x| x.path != path);
                s.programs.retain(|x| x.prefix != path);
            }) {
                app.toast(tr("pg_prefix_deleted"));
                app.rescan();
            }
        });
    }

    // ---------------------------------------------------------- a program

    /// A program's own settings (from its menu).
    pub fn program_settings(&mut self, key: &str) {
        let Some(id) = progs::id_of(key) else { return };
        let s = Store::load();
        let Some(prog) = s.program(id).cloned() else {
            return;
        };
        let pfx = s.prefix(&prog.prefix).cloned().unwrap_or(Prefix {
            name: prog
                .prefix
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            path: prog.prefix.clone(),
            runner: String::new(),
        });
        let rows = vec![
            Row::new(tr("pg_rename"), prog.title.clone()),
            Row::new(tr("pg_exe"), prog.exe.display().to_string()),
            Row::new(
                tr("pg_args"),
                if prog.args.is_empty() {
                    tr("pg_none").to_string()
                } else {
                    prog.args.clone()
                },
            ),
            Row::new(
                tr("pg_env"),
                if prog.env.is_empty() {
                    tr("pg_env_sub").to_string()
                } else {
                    prog.env.clone()
                },
            ),
            Row::new(
                format!("{}: {}", tr("pg_prefix"), pfx.name),
                progs::runner_name(&pfx.runner),
            ),
            Row::new(tr("pg_other_prefix"), ""),
        ];
        let title = prog.title.clone();
        self.choose(title, "", rows, move |app, i| {
            let set = move |app: &mut App, f: Box<dyn FnOnce(&mut Program)>| {
                if app.save_store(|s| {
                    if let Some(x) = s.programs.iter_mut().find(|x| x.id == id) {
                        f(x);
                    }
                }) {
                    app.rescan();
                }
            };
            match i {
                0 => app.type_text(
                    tr("pg_rename").to_string(),
                    prog.title.clone(),
                    move |app, t| {
                        if !t.is_empty() {
                            set(app, Box::new(move |x| x.title = t));
                        }
                    },
                ),
                1 => {
                    let start = prog.exe.parent().map(Path::to_path_buf);
                    app.pick_file(
                        tr("pg_pick_exe"),
                        &[".exe", ".bat", ".msi"],
                        start,
                        move |app, f| {
                            set(app, Box::new(move |x| x.exe = f));
                        },
                    );
                }
                2 => app.type_text(
                    tr("pg_args").to_string(),
                    prog.args.clone(),
                    move |app, t| {
                        set(app, Box::new(move |x| x.args = t));
                    },
                ),
                3 => app.type_text(tr("pg_env").to_string(), prog.env.clone(), move |app, t| {
                    set(app, Box::new(move |x| x.env = t));
                }),
                4 => app.prefix_actions(pfx),
                _ => app.pick_prefix(pfx.name.clone(), move |app, p| {
                    set(app, Box::new(move |x| x.prefix = p.path));
                }),
            }
        });
    }
}
