//! steam-downgrader's state file, read and written as it does, so locks
//! made here are kept up by its relock service and the other way round:
//! locks, snapshots of installs seen, manifests typed in, SteamDB imports,
//! cached patch notes. Unknown keys are kept as they are.

use std::path::PathBuf;

use serde_json::{Map, Value, json};

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("LENTA_SD_DIR") {
        return PathBuf::from(d);
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
        .unwrap_or_else(|| "/tmp".into())
        .join("steam-downgrader")
}

pub fn staging_dir() -> PathBuf {
    data_dir().join("staging")
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub data: Map<String, Value>,
}

/// A lock as steam-downgrader stores it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lock {
    pub actual_depots: std::collections::BTreeMap<String, String>,
    pub actual_buildid: Option<i64>,
    pub orig_auto_update: i64,
    pub strong: bool,
    pub spoofed_buildid: i64,
    pub locked_at: i64,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl State {
    /// Read afresh (the other tool may have written meanwhile).
    pub fn load() -> State {
        let data = std::fs::read(data_dir().join("state.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .unwrap_or_default();
        let mut s = State { data };
        for k in [
            "locks",
            "snapshots",
            "manual",
            "settings",
            "steamdb",
            "steamdb_prompted",
        ] {
            s.data.entry(k).or_insert_with(|| json!({}));
        }
        s
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(&Value::Object(self.data.clone()))
            .map_err(|e| e.to_string())?;
        let tmp = dir.join(format!(".state-lenta-{}.tmp", std::process::id()));
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join("state.json")).map_err(|e| e.to_string())
    }

    fn obj(&mut self, key: &str) -> &mut Map<String, Value> {
        let v = self.data.entry(key).or_insert_with(|| json!({}));
        if !v.is_object() {
            *v = json!({});
        }
        v.as_object_mut().unwrap()
    }

    fn get(&self, key: &str) -> Option<&Map<String, Value>> {
        self.data.get(key).and_then(Value::as_object)
    }

    // ------------------------------------------------------------- locks

    pub fn lock(&self, app: &str) -> Option<Lock> {
        let l = self.get("locks")?.get(app)?;
        Some(Lock {
            actual_depots: l
                .get("actual_depots")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(d, g)| Some((d.clone(), g.as_str()?.to_string())))
                        .collect()
                })
                .unwrap_or_default(),
            actual_buildid: l.get("actual_buildid").and_then(Value::as_i64),
            orig_auto_update: l
                .get("orig_auto_update")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            strong: l.get("strong").and_then(Value::as_bool).unwrap_or(false),
            spoofed_buildid: l
                .get("spoofed_buildid")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            locked_at: l.get("locked_at").and_then(Value::as_i64).unwrap_or(0),
        })
    }

    pub fn locked_apps(&self) -> Vec<String> {
        self.get("locks")
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn set_lock(&mut self, app: &str, name: &str, l: &Lock) {
        let locked_at = if l.locked_at > 0 { l.locked_at } else { now() };
        self.obj("locks").insert(
            app.to_string(),
            json!({
                "name": name,
                "locked_at": locked_at,
                "actual_depots": l.actual_depots,
                "actual_buildid": l.actual_buildid,
                "orig_auto_update": l.orig_auto_update,
                "strong": l.strong,
                "spoofed_buildid": l.spoofed_buildid,
            }),
        );
    }

    pub fn set_spoofed(&mut self, app: &str, buildid: i64) {
        if let Some(l) = self
            .obj("locks")
            .get_mut(app)
            .and_then(Value::as_object_mut)
        {
            l.insert("spoofed_buildid".into(), json!(buildid));
        }
    }

    pub fn remove_lock(&mut self, app: &str) {
        self.obj("locks").remove(app);
    }

    // --------------------------------------------------------- snapshots

    /// Remember an install seen; true when it's new.
    pub fn record_snapshot(
        &mut self,
        app: &str,
        buildid: i64,
        depots: &std::collections::BTreeMap<String, String>,
    ) -> bool {
        let snaps = self
            .obj("snapshots")
            .entry(app)
            .or_insert_with(|| json!([]));
        let Some(list) = snaps.as_array_mut() else {
            return false;
        };
        let dep = json!(depots);
        if list.iter().any(|s| {
            s.get("buildid").and_then(Value::as_i64) == Some(buildid)
                && s.get("depots") == Some(&dep)
        }) {
            return false;
        }
        list.push(json!({"buildid": buildid, "time": now(), "depots": dep}));
        true
    }

    /// (buildid, time, depot → manifest).
    pub fn snapshots(
        &self,
        app: &str,
    ) -> Vec<(i64, i64, std::collections::BTreeMap<String, String>)> {
        self.get("snapshots")
            .and_then(|m| m.get(app))
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .map(|s| {
                        (
                            s.get("buildid").and_then(Value::as_i64).unwrap_or(0),
                            s.get("time").and_then(Value::as_i64).unwrap_or(0),
                            s.get("depots")
                                .and_then(Value::as_object)
                                .map(|m| {
                                    m.iter()
                                        .filter_map(|(d, g)| {
                                            Some((d.clone(), g.as_str()?.to_string()))
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    // ------------------------------------------- manual / SteamDB / builds

    /// Manifests typed in by hand: gid → label.
    pub fn manual(&self, depot: &str) -> Vec<(String, String)> {
        self.get("manual")
            .and_then(|m| m.get(depot))
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(g, l)| (g.clone(), l.as_str().unwrap_or_default().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Manifests imported from SteamDB: gid → when.
    pub fn steamdb(&self, depot: &str) -> Vec<(String, i64)> {
        self.get("steamdb")
            .and_then(|m| m.get(depot))
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(g, t)| (g.clone(), t.as_i64().unwrap_or(0)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Builds from SteamDB patch notes: buildid → (time, title).
    pub fn builds(&self, app: &str) -> Vec<(i64, i64, String)> {
        self.get("builds")
            .and_then(|m| m.get(app))
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(b, v)| {
                        Some((
                            b.parse().ok()?,
                            v.get("time").and_then(Value::as_i64).unwrap_or(0),
                            v.get("title")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Remember a manifest seen on SteamDB; true if it's new (or got a date).
    pub fn add_steamdb(&mut self, depot: &str, manifest: &str, when: i64) -> bool {
        let d = self.obj("steamdb");
        let m = d.entry(depot.to_string()).or_insert_with(|| json!({}));
        if !m.is_object() {
            *m = json!({});
        }
        let m = m.as_object_mut().unwrap();
        let old = m.get(manifest).and_then(Value::as_i64);
        if old == Some(when) || (old.is_some() && when == 0) {
            return false;
        }
        m.insert(
            manifest.to_string(),
            json!(if when != 0 { when } else { old.unwrap_or(0) }),
        );
        true
    }

    /// Remember a build from SteamDB's patch notes; true if it's new.
    pub fn add_build(&mut self, app: &str, buildid: i64, when: i64, title: &str) -> bool {
        let b = self.obj("builds");
        let a = b.entry(app.to_string()).or_insert_with(|| json!({}));
        if !a.is_object() {
            *a = json!({});
        }
        let a = a.as_object_mut().unwrap();
        let key = buildid.to_string();
        let old = a.get(&key).cloned().unwrap_or(json!({}));
        let new = json!({
            "time": if when != 0 { when } else { old.get("time").and_then(Value::as_i64).unwrap_or(0) },
            "title": if !title.is_empty() { title } else { old.get("title").and_then(Value::as_str).unwrap_or("") },
        });
        if a.get(&key) == Some(&new) {
            return false;
        }
        a.insert(key, new);
        true
    }

    pub fn mark_steamdb_prompted(&mut self, app: &str) {
        self.obj("steamdb_prompted")
            .insert(app.to_string(), json!(now()));
    }

    // ---------------------------------------------------- patch notes cache

    /// Cached notes if younger than `max_age` seconds: (time, version, title).
    pub fn news(&self, app: &str, max_age: i64) -> Option<Vec<(i64, String, String)>> {
        let c = self.data.get("news")?.get(app)?;
        if now() - c.get("fetched")?.as_i64()? > max_age {
            return None;
        }
        Some(
            c.get("items")?
                .as_array()?
                .iter()
                .filter_map(|n| {
                    Some((
                        n.get("time")?.as_i64()?,
                        n.get("version")?.as_str()?.to_string(),
                        n.get("title")?.as_str()?.to_string(),
                    ))
                })
                .collect(),
        )
    }

    pub fn set_news(&mut self, app: &str, notes: &[(i64, String, String, String)]) {
        let items: Vec<Value> = notes
            .iter()
            .map(|(t, v, title, url)| json!({"time": t, "version": v, "title": title, "url": url}))
            .collect();
        self.obj("news")
            .insert(app.to_string(), json!({"fetched": now(), "items": items}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As steam-downgrader keeps them: a date only fills in, never wipes.
    #[test]
    fn steamdb_and_builds_added() {
        let mut s = State::default();
        assert!(s.add_steamdb("11", "123", 0));
        assert!(!s.add_steamdb("11", "123", 0));
        assert!(s.add_steamdb("11", "123", 500));
        assert!(!s.add_steamdb("11", "123", 0));
        assert_eq!(s.steamdb("11"), vec![("123".to_string(), 500)]);
        assert!(s.add_build("7", 900, 100, "Hotfix"));
        assert!(!s.add_build("7", 900, 0, ""));
        assert_eq!(s.builds("7"), vec![(900, 100, "Hotfix".to_string())]);
        s.mark_steamdb_prompted("7");
        assert!(s.data["steamdb_prompted"].get("7").is_some());
    }
}
