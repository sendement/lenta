//! The versions a game can be rolled back to, from what this machine knows
//! (Valve has no public list of old builds):
//!
//! * appinfo.vdf: the current build of every branch;
//! * content_log "finished update" lines: whole builds this client
//!   installed, with every depot's manifest;
//! * snapshots of installs seen (shared with steam-downgrader);
//! * depotcache/*.manifest: every manifest Steam fetched, with the date its
//!   build was made, used to rebuild older versions;
//! * console_log `download_depot` results, manifests typed in, SteamDB
//!   imports.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use super::game::{self, AppInfo, SteamGame};
use super::manifest::Manifest;
use super::news::{self, Note};
use super::state::State;
use crate::i18n::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Src {
    Installed,
    Branch,
    Log,
    Snapshot,
    Cache,
    Download,
    Manual,
    SteamDb,
}

impl Src {
    pub fn label(self) -> &'static str {
        tr(match self {
            Src::Installed => "src_installed",
            Src::Branch => "src_branch",
            Src::Log => "src_log",
            Src::Snapshot => "src_snapshot",
            Src::Cache => "src_cache",
            Src::Download => "src_download",
            Src::Manual => "src_manual",
            Src::SteamDb => "SteamDB",
        })
    }
}

/// One known manifest of one depot.
#[derive(Clone, Debug, Default)]
pub struct Candidate {
    pub manifest: String,
    /// When its build was made; 0 if unknown.
    pub created: i64,
    pub buildid: i64,
    pub sources: BTreeSet<Src>,
}

/// A whole-game version: a manifest per installed depot (None: unknown,
/// left as installed).
#[derive(Clone, Debug)]
pub struct Version {
    pub title: String,
    pub buildid: i64,
    pub time: i64,
    pub depots: BTreeMap<String, Option<String>>,
    pub source: Src,
    /// The depot set is recorded, not rebuilt from dates.
    pub exact: bool,
    pub game_version: String,
    pub game_version_exact: bool,
    pub version_note: String,
    /// A moment this build is known to have been the live one.
    pub live_at: i64,
}

/// What is really on disk: a locked game's appmanifest lies.
pub fn actual_depots(state: &State, g: &SteamGame) -> BTreeMap<String, String> {
    let mut real: BTreeMap<String, String> = g
        .depots
        .iter()
        .map(|(d, x)| (d.clone(), x.manifest.clone()))
        .collect();
    if let Some(l) = state.lock(&g.app) {
        real.extend(l.actual_depots);
    }
    real
}

pub fn depotcache_manifest(root: &Path, depot: &str, gid: &str) -> Option<std::path::PathBuf> {
    let p = root.join(format!("depotcache/{depot}_{gid}.manifest"));
    p.is_file().then_some(p)
}

pub struct History<'a> {
    pub root: &'a Path,
    pub state: &'a State,
    logged: HashMap<String, Vec<game::Logged>>,
    downloads: HashMap<(String, String), Vec<(String, i64)>>,
}

impl<'a> History<'a> {
    pub fn new(root: &'a Path, state: &'a State) -> History<'a> {
        History {
            root,
            state,
            logged: game::finished_updates(root),
            downloads: game::depot_downloads(root),
        }
    }

    pub fn downloads(&self, app: &str, depot: &str) -> &[(String, i64)] {
        self.downloads
            .get(&(app.to_string(), depot.to_string()))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn logged(&self, app: &str) -> &[game::Logged] {
        self.logged.get(app).map(Vec::as_slice).unwrap_or_default()
    }

    /// Every known manifest per depot, newest first.
    pub fn candidates(
        &self,
        g: &SteamGame,
        info: Option<&AppInfo>,
        actual: &BTreeMap<String, String>,
    ) -> BTreeMap<String, Vec<Candidate>> {
        let mut out: BTreeMap<String, BTreeMap<String, Candidate>> = g
            .depots
            .keys()
            .map(|d| (d.clone(), BTreeMap::new()))
            .collect();
        let mut add = |depot: &str, gid: &str, src: Src, created: i64, buildid: i64| {
            let Some(m) = out.get_mut(depot) else { return };
            if gid.is_empty() || gid == "0" {
                return;
            }
            let c = m.entry(gid.to_string()).or_insert_with(|| Candidate {
                manifest: gid.to_string(),
                ..Default::default()
            });
            c.sources.insert(src);
            if c.created == 0 {
                c.created = created;
            }
            if c.buildid == 0 {
                c.buildid = buildid;
            }
        };
        for (d, gid) in actual {
            add(d, gid, Src::Installed, 0, 0);
        }
        let cache = self.root.join("depotcache");
        let cached: Vec<(String, String, std::path::PathBuf)> = std::fs::read_dir(&cache)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let n = e.file_name().to_string_lossy().into_owned();
                        let stem = n.strip_suffix(".manifest")?;
                        let (d, gid) = stem.split_once('_')?;
                        Some((d.to_string(), gid.to_string(), e.path()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for d in g.depots.keys() {
            for (cd, gid, p) in cached.iter().filter(|(cd, _, _)| cd == d) {
                add(cd, gid, Src::Cache, created(p), 0);
            }
            for (gid, _) in self.downloads(&g.app, d) {
                add(d, gid, Src::Download, 0, 0);
            }
            for (gid, _) in self.state.manual(d) {
                add(d, &gid, Src::Manual, 0, 0);
            }
            for (gid, when) in self.state.steamdb(d) {
                add(d, &gid, Src::SteamDb, when, 0);
            }
        }
        if let Some(info) = info {
            for (d, per) in &info.depots {
                for (branch, (gid, _)) in per {
                    let b = info.branches.get(branch).map_or(0, |b| b.buildid);
                    add(d, gid, Src::Branch, 0, b);
                }
            }
        }
        for (_t, bid, depots) in self.logged(&g.app) {
            for (d, gid) in depots {
                add(d, gid, Src::Log, 0, *bid);
            }
        }
        for (bid, _t, depots) in self.state.snapshots(&g.app) {
            for (d, gid) in &depots {
                add(d, gid, Src::Snapshot, 0, bid);
            }
        }
        out.into_iter()
            .map(|(d, m)| {
                let mut v: Vec<Candidate> = m.into_values().collect();
                v.sort_by_key(|c| std::cmp::Reverse((c.created, c.buildid)));
                (d, v)
            })
            .collect()
    }

    /// Whole-game versions, newest first.
    pub fn versions(
        &self,
        g: &SteamGame,
        info: Option<&AppInfo>,
        cands: &BTreeMap<String, Vec<Candidate>>,
        notes: &[Note],
    ) -> Vec<Version> {
        let ids: Vec<String> = g.depots.keys().cloned().collect();
        let mut found: Vec<Version> = Vec::new();
        let ver = |title: String, buildid, time, depots, source, exact, live_at| Version {
            title,
            buildid,
            time,
            depots,
            source,
            exact,
            game_version: String::new(),
            game_version_exact: false,
            version_note: String::new(),
            live_at,
        };
        if let Some(info) = info {
            for (name, b) in &info.branches {
                let depots: BTreeMap<String, Option<String>> = ids
                    .iter()
                    .map(|d| {
                        let m = info
                            .depots
                            .get(d)
                            .and_then(|per| per.get(name))
                            .map(|x| x.0.clone());
                        (d.clone(), m)
                    })
                    .collect();
                if depots.values().all(Option::is_none) {
                    continue;
                }
                let mut title = if name == "public" {
                    tr("ver_latest").to_string()
                } else {
                    format!("{} «{name}»", tr("ver_branch"))
                };
                if !b.description.is_empty() {
                    title += &format!(" — {}", b.description);
                }
                found.push(ver(
                    title,
                    b.buildid,
                    b.time_updated,
                    depots,
                    Src::Branch,
                    true,
                    b.time_updated,
                ));
            }
        }
        let build = |b: i64| format!("{} {b}", tr("ver_build"));
        for (t, bid, depots) in self.logged(&g.app) {
            let d = ids
                .iter()
                .map(|x| (x.clone(), depots.get(x).cloned()))
                .collect();
            found.push(ver(build(*bid), *bid, *t, d, Src::Log, true, *t));
        }
        for (bid, t, depots) in self.state.snapshots(&g.app) {
            let d = ids
                .iter()
                .map(|x| (x.clone(), depots.get(x).cloned()))
                .collect();
            found.push(ver(build(bid), bid, t, d, Src::Snapshot, true, t));
        }
        let main = ids
            .iter()
            .max_by_key(|d| g.depots.get(*d).map_or(0, |x| x.size))
            .cloned();
        let newest_before = |depot: &str, t: i64| -> Option<String> {
            cands
                .get(depot)?
                .iter()
                .find(|c| c.created != 0 && c.created <= t + 7200)
                .map(|c| c.manifest.clone())
        };
        // Builds from SteamDB patch notes: each depot as it was then.
        let imported = self.state.builds(&g.app);
        let have: BTreeSet<i64> = found.iter().map(|v| v.buildid).collect();
        let mut imp_sorted = imported.clone();
        imp_sorted.sort_by_key(|b| std::cmp::Reverse(b.0));
        if let Some(main) = &main {
            for (bid, time, _) in &imp_sorted {
                if have.contains(bid) || *time == 0 {
                    continue;
                }
                let depots: BTreeMap<String, Option<String>> = ids
                    .iter()
                    .map(|d| (d.clone(), newest_before(d, *time)))
                    .collect();
                if depots.get(main).is_some_and(Option::is_some) {
                    found.push(ver(
                        build(*bid),
                        *bid,
                        *time,
                        depots,
                        Src::SteamDb,
                        false,
                        0,
                    ));
                }
            }
            // Builds rebuilt from dated manifests, anchored on the biggest
            // depot: every other depot at its newest manifest by then (+2h,
            // a build's depots are made minutes apart).
            let known: BTreeSet<Option<String>> = found
                .iter()
                .map(|v| v.depots.get(main).cloned().flatten())
                .collect();
            for c in cands.get(main).into_iter().flatten() {
                if known.contains(&Some(c.manifest.clone())) || c.created == 0 {
                    continue;
                }
                let mut depots: BTreeMap<String, Option<String>> = ids
                    .iter()
                    .map(|d| (d.clone(), newest_before(d, c.created)))
                    .collect();
                depots.insert(main.clone(), Some(c.manifest.clone()));
                let title = if c.buildid != 0 {
                    build(c.buildid)
                } else {
                    format!(
                        "{} {}",
                        tr("ver_build_from"),
                        crate::game::date(c.created as u64)
                    )
                };
                let src = if c.sources.contains(&Src::Cache) {
                    Src::Cache
                } else if c.sources.contains(&Src::SteamDb) {
                    Src::SteamDb
                } else {
                    *c.sources.iter().next().unwrap_or(&Src::Cache)
                };
                found.push(ver(title, c.buildid, c.created, depots, src, false, 0));
            }
        }
        // A build logged several times: keep its latest record.
        let mut latest: HashMap<i64, i64> = HashMap::new();
        for v in &found {
            if v.exact && v.buildid != 0 && matches!(v.source, Src::Log | Src::Snapshot) {
                let e = latest.entry(v.buildid).or_insert(v.time);
                *e = (*e).max(v.time);
            }
        }
        let mut kept_once: BTreeSet<i64> = BTreeSet::new();
        found.retain(|v| {
            if !(v.exact && v.buildid != 0 && matches!(v.source, Src::Log | Src::Snapshot)) {
                return true;
            }
            v.time == latest[&v.buildid] && kept_once.insert(v.buildid)
        });
        // One entry per depot set: keep the richest.
        let mut merged: Vec<Version> = Vec::new();
        for v in found {
            let key: Vec<Option<String>> = ids
                .iter()
                .map(|d| v.depots.get(d).cloned().flatten())
                .collect();
            let at = merged.iter().position(|m| {
                ids.iter()
                    .map(|d| m.depots.get(d).cloned().flatten())
                    .collect::<Vec<_>>()
                    == key
            });
            match at {
                Some(i) => {
                    let old = &mut merged[i];
                    if v.source == Src::Branch && old.source != Src::Branch {
                        let (t, live) = (old.time, old.live_at);
                        let mut v = v;
                        if v.time == 0 {
                            v.time = t;
                        }
                        v.live_at = v.live_at.max(live);
                        merged[i] = v;
                    } else {
                        if old.buildid == 0 {
                            old.buildid = v.buildid;
                        }
                        old.live_at = old.live_at.max(v.live_at);
                    }
                }
                None => merged.push(v),
            }
        }
        // Build dates from manifest creation where only a log time is known.
        let created_of = |d: &str, gid: &str| {
            cands
                .get(d)
                .and_then(|cs| cs.iter().find(|c| c.manifest == gid))
                .map_or(0, |c| c.created)
        };
        if let Some(main) = &main {
            for v in &mut merged {
                if let Some(Some(m)) = v.depots.get(main) {
                    let ct = created_of(main, m);
                    if ct != 0 {
                        v.time = ct;
                    }
                }
            }
        }
        label(g, main.as_deref(), &mut merged, cands, &imported, notes);
        merged.sort_by_key(|v| std::cmp::Reverse((v.time, v.buildid)));
        merged
    }
}

fn created(p: &Path) -> i64 {
    // Only the header is read; depotcache can hold hundreds of these.
    Manifest::read(p, false).map_or(0, |m| m.created as i64)
}

/// Human version numbers. A SteamDB title for the very build is exact;
/// otherwise each patch note is pinned to the main-depot manifest that was
/// newest when it went out (builds are made days before release).
fn label(
    g: &SteamGame,
    main: Option<&str>,
    versions: &mut [Version],
    cands: &BTreeMap<String, Vec<Candidate>>,
    imported: &[(i64, i64, String)],
    notes: &[Note],
) {
    let mut timeline: Vec<Note> = notes.to_vec();
    for (_b, t, title) in imported {
        if let Some(v) = news::extract_version(title, &g.name)
            && *t != 0
        {
            timeline.push(Note {
                time: *t,
                version: v,
                title: title.clone(),
            });
        }
    }
    timeline.sort_by_key(|n| n.time);
    let mut dated: Vec<&Candidate> = main
        .and_then(|m| cands.get(m))
        .map(|cs| cs.iter().filter(|c| c.created != 0).collect())
        .unwrap_or_default();
    dated.sort_by_key(|c| c.created);
    let created: HashMap<&str, i64> = dated
        .iter()
        .map(|c| (c.manifest.as_str(), c.created))
        .collect();
    let mut pinned: HashMap<String, Vec<Note>> = HashMap::new();
    for n in &timeline {
        let live = dated
            .iter()
            .take_while(|c| c.created <= n.time + 86400)
            .last();
        if let Some(c) = live {
            pinned
                .entry(c.manifest.clone())
                .or_default()
                .push(n.clone());
        }
    }
    for v in versions.iter_mut() {
        let own = imported
            .iter()
            .find(|(b, _, _)| *b == v.buildid && v.buildid != 0)
            .and_then(|(_, _, t)| news::extract_version(t, &g.name).map(|x| (x, t.clone())));
        if let Some((x, t)) = own {
            v.game_version = x;
            v.game_version_exact = true;
            v.version_note = t;
            continue;
        }
        let m = main.and_then(|m| v.depots.get(m).cloned().flatten());
        let mut ns: Vec<Note> = m
            .as_ref()
            .and_then(|m| pinned.get(m))
            .cloned()
            .unwrap_or_default();
        if v.live_at != 0 {
            // Known live then: later notes may be builds never seen here.
            ns.retain(|n| n.time <= v.live_at + 86400);
        }
        if let (Some(first), Some(last)) = (ns.first(), ns.last()) {
            v.game_version = if ns.len() == 1 {
                first.version.clone()
            } else {
                format!("{}–{}", first.version, last.version)
            };
            v.version_note = ns
                .iter()
                .map(|n| n.title.as_str())
                .collect::<Vec<_>>()
                .join(" / ");
        } else if v.live_at != 0
            && let Some(n) = news::version_at(v.live_at, &timeline, 0)
        {
            v.game_version = n.version.clone();
            v.version_note = n.title.clone();
        } else if let Some(t) = m.as_deref().and_then(|m| created.get(m)) {
            if let Some(n) = news::version_at(*t, &timeline, 0) {
                v.game_version = format!("{}+", n.version);
                v.version_note = n.title.clone();
            }
        } else if let Some(n) = news::version_at(v.time, &timeline, 0) {
            v.game_version = n.version.clone();
            v.version_note = n.title.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Versions of the real installed games come out (read only; no news).
    #[test]
    fn real_versions() {
        let state = State::load();
        for root in crate::sources::steam::roots() {
            let hist = History::new(&root, &state);
            let Ok(rd) = std::fs::read_dir(root.join("steamapps")) else {
                continue;
            };
            for e in rd.flatten().take(60) {
                let n = e.file_name().to_string_lossy().into_owned();
                let Some(app) = n
                    .strip_prefix("appmanifest_")
                    .and_then(|s| s.strip_suffix(".acf"))
                else {
                    continue;
                };
                let Some(g) = SteamGame::find(&root, app) else {
                    continue;
                };
                let info = game::appinfo(&root, app);
                let actual = actual_depots(&state, &g);
                let cands = hist.candidates(&g, info.as_ref(), &actual);
                let vs = hist.versions(&g, info.as_ref(), &cands, &[]);
                eprintln!("{}: {} versions", g.name, vs.len());
                for v in vs.iter().take(4) {
                    eprintln!(
                        "   {} {} {:?}",
                        v.title,
                        crate::game::date(v.time as u64),
                        v.source
                    );
                }
            }
        }
    }
}
