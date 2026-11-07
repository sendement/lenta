//! Running inside Flatpak: one build serves both, and only where the
//! sandbox differs (the user's real folders, the host's programs and
//! processes) does it take another way.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Inside a Flatpak sandbox?
pub fn flatpak() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    *F.get_or_init(|| Path::new("/.flatpak-info").exists())
}

/// Before anything reads the environment: the user's real data, cache and
/// config folders (Flatpak points XDG_* into ~/.var/app; settings, the
/// launch hook Steam runs and modules are shared with a native install),
/// and the host's icons and desktop entries.
pub fn fix_env() {
    if !flatpak() {
        return;
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/".into());
    for (var, host, default) in [
        ("XDG_DATA_HOME", "HOST_XDG_DATA_HOME", ".local/share"),
        ("XDG_CACHE_HOME", "HOST_XDG_CACHE_HOME", ".cache"),
        ("XDG_CONFIG_HOME", "HOST_XDG_CONFIG_HOME", ".config"),
    ] {
        let real = std::env::var_os(host)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(default));
        // Nothing else runs yet: no thread reads the environment.
        unsafe { std::env::set_var(var, real) };
    }
    let mut dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/app/share:/usr/share".into());
    for d in [
        "/run/host/usr/local/share",
        "/run/host/usr/share",
        "/run/host/share",
        "/run/host/user-share",
    ] {
        if !dirs.split(':').any(|x| x == d) {
            dirs.push(':');
            dirs.push_str(d);
        }
    }
    unsafe { std::env::set_var("XDG_DATA_DIRS", dirs) };
}

/// A command run on the host instead (games, launchers and modules live
/// there, not in the sandbox).
pub fn host(cmd: std::process::Command) -> std::process::Command {
    if !flatpak() {
        return cmd;
    }
    let mut c = std::process::Command::new("flatpak-spawn");
    c.arg("--host").arg("--watch-bus");
    if let Some(dir) = cmd.get_current_dir() {
        c.arg(format!("--directory={}", host_path(dir).display()));
    }
    for (k, v) in cmd.get_envs() {
        if let Some(v) = v {
            let mut a = std::ffi::OsString::from("--env=");
            a.push(k);
            a.push("=");
            a.push(host_path(Path::new(v)).as_os_str());
            c.arg(a);
        }
    }
    c.arg(host_path(Path::new(cmd.get_program())))
        .args(cmd.get_args());
    c
}

/// The host's own folders as seen in here (host-os: /usr under /run/host).
pub fn system(p: &str) -> PathBuf {
    if flatpak() {
        Path::new("/run/host").join(p.trim_start_matches('/'))
    } else {
        p.into()
    }
}

/// A path as the host knows it.
pub fn host_path(p: &Path) -> PathBuf {
    if flatpak()
        && let Ok(rest) = p.strip_prefix("/run/host")
        && (rest.starts_with("usr") || rest.starts_with("etc"))
    {
        return Path::new("/").join(rest);
    }
    p.to_path_buf()
}

/// Is this program on the host's PATH?
pub fn has_program(name: &str) -> bool {
    if flatpak() {
        let home = dirs::home_dir().unwrap_or_default();
        return [
            PathBuf::from("/run/host/usr/bin"),
            "/run/host/usr/local/bin".into(),
            "/run/host/usr/games".into(),
            home.join(".local/bin"),
            "/var/lib/flatpak/exports/bin".into(),
            home.join(".local/share/flatpak/exports/bin"),
        ]
        .iter()
        .any(|d| d.join(name).is_file());
    }
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}
