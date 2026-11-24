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
- **Library** — a tab per launcher (Steam, Heroic, Lutris, Hydra, your own
  Windows programs) with shelves on
  the left: all, installed, favourites, collections, third-party,
  soundtracks. The last tab, **My collections**, holds the launcher's own
  collections across all launchers: plain ones (games put in by hand) and
  dynamic ones (every game that fits a rule: launcher, installed,
  favourite, third-party, played last, time played, words in the name).
  Drag a game card onto the panel that comes up on the left, or press Y
  (right click) on a game for its menu and tick a collection. Names are
  typed on Polka's on-screen keyboard (or a real one). Kept in
  `~/.local/share/lenta/collections.json`.
- **System apps** — the system's `.desktop` programs, grouped by category.

## Sources

| Source | Games | Art | Play time | Extra |
|---|---|---|---|---|
| Steam (native, Flatpak) | installed apps from every library, non-Steam shortcuts | `appcache/librarycache`, custom `userdata/*/config/grid`, store CDN | `localconfig.vdf` | achievements from `appcache/stats` (local, no API key), store page via `appdetails` |
| Heroic (native, Flatpak) | Epic, GOG, Amazon, side-loaded, installed per legendary/gogdl/nile's own lists; owned ones not installed too (Play opens Heroic's install) | library caches | `store/timestamp.json` | see below |
| Lutris (native, Flatpak) | installed games from `pga.db` | covers, banners, icons | `pga.db` | see below |
| Hydra | its library from `hydra-db` (LevelDB, read from a copy while Hydra runs); installed when it knows the program; Play on one being downloaded opens Hydra to resume it there (resumed from outside, Hydra 4.4's torrent service loses it), greyed out with nothing downloaded | Steam's store art | its own count | started through Hydra (`hydralauncher://run?…`), its prefix and Proton |

Games from Heroic, Lutris, Hydra and non-Steam shortcuts are matched to Steam's
store: by the app id a Steam emulator's files carry (`steam_appid.txt`,
`steam_emu.ini`, save folders) or else by title. The match brings the store
description and screenshots, and fills missing art. When the game's Wine
prefix holds a Steam emulator's achievement save (CODEX, RUNE, EMPRESS,
Goldberg/GSE, OnlineFix, RLD!…), the Achievements tab shows the full list
from Steam (`IPlayerService/GetGameAchievements`, no key) with the ones in
the save marked as earned, and how many Steam players have each.

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

Builds this machine never had come from SteamDB: "Not in the list?
Versions from SteamDB" at the end of the list opens the depot's manifests
page (or the game's patch notes) in the browser; the table copied there is
pasted back (Ctrl+V, or Paste, which reads the clipboard with `wl-paste`,
`xclip` or `xsel`) and its manifests and builds join the list for good.

### Installing, moving, uninstalling

Y (right click) on a game: **Move to another library…** and
**Uninstall…**; Play on a Steam game that isn't installed asks which
library first. Settings → **Steam libraries** lists them with their free
space: A on one moves all its games elsewhere, A on a drive without one
makes a library there (`<drive>/SteamLibrary`).

- **Install** writes the game's appmanifest ("update required") into the
  library chosen; Steam, started in the tray, downloads it by itself and
  the progress shows on the game's button.
- **Move** takes the game, its Proton prefix, shader cache and Workshop
  items: renamed on the same drive, copied (in the kernel, with progress,
  cancellable from the game's menu) to another one while Steam keeps
  running; Steam is closed only for the switch-over and started again. A
  game locked by steam-downgrader stays read-only. exFAT/FAT drives take
  native games only (no links or modes there for Proton prefixes).
- **Uninstall** (Steam) removes the game, shader cache and Workshop items;
  the Proton prefix stays, saves live in it. Heroic (Epic, GOG, Amazon) and
  Lutris games are uninstalled as those launchers do it — the folder they
  installed into deleted, their lists changed (Heroic is closed first);
  games added to them by hand are only taken off their lists. A Hydra
  game loses the folder it was installed into and Hydra's download of it,
  unless the folder holds other games or is a shared one (`~/Games`);
  Hydra is closed for it (its base is copied to `hydra-db.lenta-backup`
  first), the game marked not installed, and Hydra started again in the
  tray. Prefixes stay.
- From the terminal: `lenta --move APPID FOLDER`.

### Own programs (Wine, Proton)

The Library's **Own** tab holds Windows programs and games of your own,
each in a Wine prefix made here (`~/Games/Lenta/Prefixes` by default,
`prefix_root` in the settings file). **Add** picks a file — in Lenta's own
browser (places, folders, only `.exe`/`.msi`/`.bat` unless Y shows all) or
the system's window (Settings → Choose files in the system's window) —
and asks what it is, the likelier first by its name:

- **an installer**: run in a prefix (a new one, named, or one made
  before), then what it installed is found — the programs its new
  shortcuts point at, else its new programs (uninstallers, setups and
  redistributables left out). One is added at once; several are offered
  to choose from, with "Choose by hand…" in the prefix's drive C.
- **the program itself**: added as it is, run in the prefix chosen.

A prefix runs with a Proton (yours in `compatibilitytools.d`, the
system's, Steam's own) through `umu-run` — with the umu id of the game's
Steam match, so protonfixes apply — or with umu's own UMU-Proton, or the
system's Wine (last in the list). **Prefixes** (that tile, or Settings)
lists them: libraries through winetricks at one press (Visual C++ and
DirectX basics, older Visual C++, .NET 4.8, core fonts, or verbs typed),
a program installed there, another file run once (a patch), winecfg,
Proton or Wine changed, its folder, deleting it (to the trash). A
program's menu has **Settings…**: its name, program, arguments, prefix.
Uninstalling one deletes the folder it was installed into inside the
prefix (never Windows' own folders, nor one holding another program) and
takes it off the list; the prefix stays.

### Proton

The Proton page lists the compatibility tools installed (Steam's, custom,
the system's packages) and how many games use each; installs the newest
GE-Proton and Proton-CachyOS builds from their GitHub releases into
`compatibilitytools.d` (checked against the release's SHA-512) and removes
custom ones; Valve's own are installed and removed through Steam. A game's
Compatibility tab also clears its shader caches (Steam's, DXVK/vkd3d, the
drivers') and resets its prefix, moving the old one to the trash.

### Interface sounds

Moving, opening, the side menu, switches and sliders make sounds: Steam's
own set from the client's `steamui/sounds`, or a Decky AudioLoader pack
(the Sound packs page in the side panel shows the installed packs and
DeckThemes' store as cards, most downloaded first; A installs and uses one). Packs keep AudioLoader's layout and
`pack.json` (`ignore`, `mappings`); AudioLoader's own `~/homebrew/sounds`
are picked up too.

### Theme music

With "Theme music on a game's page" on (Settings → General), a game's page
plays its theme on a loop, fading in and out; it stops when the page
closes or a game takes the focus. The idea comes from
[NonSteamLaunchers](https://github.com/moraroy/NonSteamLaunchers-On-Steam-Deck):
the track is found on YouTube through an Invidious instance ("<game> Game
Theme Music", `LENTA_INVIDIOUS` picks another instance), its audio fetched
through the instance and kept in `~/.cache/lenta/music`.

Downloads (store pages, art, achievement icons) are kept in `~/.cache/lenta`.
Settings live in `~/.config/lenta/config.toml`.

### Launch options

A game's page has a Launch tab with its launch options, kept in step with
Steam's and Lutris's: whoever changed them last wins. A change made in Steam
or Lutris is taken in; one made in lenta is written there. Lutris reads its
game file at every start, so that's immediate. Steam holds launch options in
memory while it runs, so lenta writes them while it's closed, and until then
starts the game with lenta's options through a small script at the front of
the game's Steam options (once; Steam has to be restarted for that the first
time). A copy of Steam's file is kept as `localconfig.vdf.lenta-backup`.

### Modules

Other programs can add to lenta: a section of their own (a package with
system settings, say), or a tab on every game's page (per-game settings for
a frame generation layer). A module is a folder with `module.json` and a
program in any language that answers with its pages as JSON; lenta draws
them, so they work with the pad. Packages put modules in
`/usr/share/lenta/modules`, your own go in `~/.local/share/lenta/modules`;
removing a package takes its section away. In **Modules** each can be turned
off and shown as a section or only there. See [docs/modules.md](docs/modules.md)
and the examples: `examples/modules/system` (Wi-Fi, Bluetooth and About, with
a side panel of its own), `examples/modules/mangohud` (common settings in
Modules, each game's own on its page, applied to the game's start) and
`examples/modules/demo` (every kind of item and a game tab).

## Controls

| Pad | Keyboard | |
|---|---|---|
| D-pad / left stick | arrows | move |
| A / ✕ | Enter | open, press |
| X / □ | Space | play at once |
| B / ○ | Esc | back |
| LB / RB | Q / E | previous / next section |
| LT / RT | PgUp / PgDn | filter, game page tabs (also a sideways two-finger swipe) |
| View | Tab | side panel |
| Menu | F2 | settings |
| | F11 | full screen |

The pad is ignored while another window (a game) has focus. Power off,
restart and log out are at the bottom of the side panel (asked first).

## Installing

**Flatpak**, from Lenta's own repository (updates come with
`flatpak update` or Discover):

    flatpak install --user https://sendement.github.io/lenta/lenta.flatpakref

The Flatpak sees your whole home and drives (Steam libraries live
anywhere), keeps its settings in the same `~/.config/lenta` and
`~/.local/share/lenta` as a native install, and starts games, launchers and
modules on the host through `flatpak-spawn`.

**Arch Linux**: `packaging/arch/PKGBUILD`, or the `.pkg.tar.zst` attached to
each release:

    sudo pacman -U lenta-*-x86_64.pkg.tar.zst

## Building

    cargo build --release
    ./target/release/lenta              # the launcher
    ./target/release/lenta --list       # what the sources found, in the terminal
    ./target/release/lenta --diagnose   # folders, games, processes seen (Flatpak too)

Screenshots of every screen, offscreen, on this machine's games:

    LENTA_SHOT_DIR=/tmp/shots cargo test shots -- --ignored

## License

MIT
