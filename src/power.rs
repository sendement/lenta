//! Power off, restart and log out, from the pad. On the host (inside
//! Flatpak through flatpak-spawn); the session's own way out first, so
//! its programs are told and close cleanly.

use crate::game::{has_program, run_first};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Power {
    Off,
    Reboot,
    Logout,
}

impl Power {
    pub const ALL: [Power; 3] = [Power::Off, Power::Reboot, Power::Logout];

    pub fn key(self) -> &'static str {
        match self {
            Power::Off => "power_off",
            Power::Reboot => "power_reboot",
            Power::Logout => "power_logout",
        }
    }

    /// What to run, in order: the first that starts is it.
    pub fn commands(self, desktop: &str, session: Option<&str>) -> Vec<Vec<String>> {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        let mut out = Vec::new();
        match self {
            Power::Off => {
                out.push(v(&["systemctl", "poweroff"]));
                out.push(v(&["loginctl", "poweroff"]));
            }
            Power::Reboot => {
                out.push(v(&["systemctl", "reboot"]));
                out.push(v(&["loginctl", "reboot"]));
            }
            Power::Logout => {
                let d = desktop.to_ascii_lowercase();
                let own: &[&[&str]] = if d.contains("hyprland") {
                    &[&["hyprctl", "dispatch", "exit"]]
                } else if d.contains("kde") {
                    &[
                        &[
                            "qdbus6",
                            "org.kde.Shutdown",
                            "/Shutdown",
                            "org.kde.Shutdown.logout",
                        ],
                        &[
                            "qdbus",
                            "org.kde.Shutdown",
                            "/Shutdown",
                            "org.kde.Shutdown.logout",
                        ],
                    ]
                } else if d.contains("gnome") {
                    &[&["gnome-session-quit", "--logout", "--no-prompt"]]
                } else if d.contains("xfce") {
                    &[&["xfce4-session-logout", "--logout"]]
                } else if d.contains("sway") {
                    &[&["swaymsg", "exit"]]
                } else if d.contains("niri") {
                    &[&["niri", "msg", "action", "quit", "--skip-confirmation"]]
                } else {
                    &[]
                };
                out.extend(own.iter().map(|a| v(a)));
                if let Some(s) = session.filter(|s| !s.is_empty()) {
                    out.push(v(&["loginctl", "terminate-session", s]));
                }
            }
        }
        out
    }

    /// Do it (nothing under LENTA_DRY_RUN: spawn skips it).
    pub fn run(self) -> Result<(), String> {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let session = std::env::var("XDG_SESSION_ID").ok();
        let cmds: Vec<Vec<String>> = self
            .commands(&desktop, session.as_deref())
            .into_iter()
            .filter(|c| has_program(&c[0]))
            .collect();
        if cmds.is_empty() {
            return Err("no way found".into());
        }
        run_first(&cmds)
    }
}

impl crate::app::App {
    pub fn ask_power(&mut self, p: Power) {
        let t = crate::i18n::tr;
        let (q, line, yes) = match p {
            Power::Off => ("power_off_q", "power_save_note", "power_off"),
            Power::Reboot => ("power_reboot_q", "power_save_note", "power_reboot"),
            Power::Logout => ("power_logout_q", "power_save_note", "power_logout"),
        };
        let mut lines = vec![t(line).to_string()];
        if !self.was_running.is_empty() {
            lines.insert(0, t("power_game_running").to_string());
        }
        self.ask(t(q).to_string(), lines, t(yes), move |app| {
            app.cfg.save();
            if let Err(e) = p.run() {
                app.toast(format!("{}: {e}", t(p.key())));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ways_out() {
        let first = |p: Power, d: &str, s| p.commands(d, s)[0].join(" ");
        assert_eq!(first(Power::Off, "", None), "systemctl poweroff");
        assert_eq!(first(Power::Reboot, "KDE", None), "systemctl reboot");
        assert_eq!(
            first(Power::Logout, "Hyprland", Some("3")),
            "hyprctl dispatch exit"
        );
        assert_eq!(
            first(Power::Logout, "ubuntu:GNOME", None),
            "gnome-session-quit --logout --no-prompt"
        );
        let other = Power::Logout.commands("Weird", Some("7"));
        assert_eq!(other, vec![vec!["loginctl", "terminate-session", "7"]]);
        assert!(Power::Logout.commands("Weird", None).is_empty());
    }
}
