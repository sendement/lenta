# lenta

A gamepad-first game launcher for Linux, written in Rust with egui.

- **Home** — a ribbon of game cards: vertical covers or wide Steam-style
  capsules, running horizontally or vertically in either direction. Card size,
  gap, zoom of the selected card, dimming of the others, 3D tilt (towards the
  middle or all one way), rounding, reflection and glow are all settings, with
  a live preview. The accent is your own colour, a gradient, or taken from the
  selected card's art. The selected game's art fills the background.
- **Game page** — Play, the Steam store description and details (developer,
  publisher, genres, release date, Metacritic), achievements with unlock
  dates, screenshots, play time and last launch.
- **Library** — every game in a grid, filtered by source.
- **System apps** — the system's `.desktop` programs, grouped by category.

## Sources

| Source | Games | Art | Play time | Extra |
|---|---|---|---|---|
| Steam (native, Flatpak) | installed apps from every library, non-Steam shortcuts | `appcache/librarycache`, custom `userdata/*/config/grid`, store CDN | `localconfig.vdf` | achievements from `appcache/stats` (local, no API key), store page via `appdetails` |
| Heroic (native, Flatpak) | installed Epic, GOG, Amazon, side-loaded | library caches | `store/timestamp.json` | |
| Lutris (native, Flatpak) | installed games from `pga.db` | covers, banners, icons | `pga.db` | |

Planned: Hydra Launcher.

Downloads (store pages, art, achievement icons) are kept in `~/.cache/lenta`.
Settings live in `~/.config/lenta/config.toml`.

## Controls

| Pad | Keyboard | |
|---|---|---|
| D-pad / left stick | arrows | move |
| A / ✕ | Enter | open, press |
| X / □ | Space | play at once |
| B / ○ | Esc | back |
| LB / RB | Q / E | previous / next section |
| LT / RT | PgUp / PgDn | filter, game page tabs |
| View | Tab | side panel |
| Menu | F2 | settings |
| | F11 | full screen |

The pad is ignored while another window (a game) has focus.

## Building

    cargo build --release
    ./target/release/lenta           # the launcher
    ./target/release/lenta --list    # what the sources found, in the terminal

Screenshots of every screen, offscreen, on this machine's games:

    LENTA_SHOT_DIR=/tmp/shots cargo test shots -- --ignored

## License

MIT
