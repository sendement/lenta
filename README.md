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
| Heroic (native, Flatpak) | installed Epic, GOG, Amazon, side-loaded | library caches | `store/timestamp.json` | see below |
| Lutris (native, Flatpak) | installed games from `pga.db` | covers, banners, icons | `pga.db` | see below |

Games from Heroic, Lutris and non-Steam shortcuts are matched to Steam's
store: by the app id a Steam emulator's files carry (`steam_appid.txt`,
`steam_emu.ini`, save folders) or else by title. The match brings the store
description and screenshots, and fills missing art. When the game's Wine
prefix holds a Steam emulator's achievement save (CODEX, RUNE, EMPRESS,
Goldberg/GSE, OnlineFix, RLD!…), the Achievements tab shows the full list
from Steam (`IPlayerService/GetGameAchievements`, no key) with the ones in
the save marked as earned, and how many Steam players have each.

Planned: Hydra Launcher.

### Steam account and client (optional)

- **Collections** from the Steam client's own files show as library
  filters; games in "Hidden" stay out of the ribbon.
- **Compatibility tool** per game (Steam's Protons and those in
  `compatibilitytools.d`) on the game page's Compatibility tab. Steam
  rewrites `config.vdf` when it quits, so a change made while it runs waits
  until it closes, or until "Restart Steam to apply". The old file is kept
  as `config.vdf.lenta-backup`.
- **Sign in** (Settings → Steam account) by scanning a QR code with the
  Steam app, the same sign-in the client uses. Only the refresh token is
  kept, in `~/.local/share/lenta/steam.json` (mode 0600). With it the
  library also lists owned games that aren't installed (Play installs
  them) and play time counted on every device.

### Versions (rollback, from steam-downgrader)

A Steam game's page has a Versions tab: the builds this machine knows
(branches, the client's update log, installs seen, depotcache manifests
rebuilt by date), with version numbers from Steam's patch notes. An old
build is downloaded in the background over Steam's own protocol (a CM
session over WebSocket with the account's sign-in, depot keys, manifests
and chunks from the CDN, decrypted and checked), taking unchanged files and
chunks from the install so a delta costs little. Rolling back closes Steam,
moves the files in (only files the depot lists are removed, so saves stay)
and locks the game: the appmanifest shows the latest build and is
read-only, so Steam leaves the game alone; new builds are followed at each
start. State and downloads are shared with
[steam-downgrader](https://github.com/sendement/steam-downgrader)
(`~/.local/share/steam-downgrader`), so either tool sees the other's locks.

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
