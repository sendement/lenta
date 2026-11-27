# Modules

A module adds to lenta from outside: a section of its own in the side panel
(or a window opened from **Modules**), and a tab on games' pages. The module
is a program in any language; lenta asks it for pages as JSON and draws them
itself, so they work with the pad, the keyboard and the mouse like the rest of
the launcher.

## Where modules live

Each module is a folder with `module.json`:

| Folder | Who puts it there |
|---|---|
| `~/.local/share/lenta/modules/<id>/` | you |
| `/usr/local/share/lenta/modules/<id>/` | locally built packages |
| `/usr/share/lenta/modules/<id>/` | distribution packages |

lenta looks again every few seconds: install a package and its section shows
up; remove it and the section goes. A module in your own folder wins over a
system one with the same id.

In **Modules** every module can be turned off, and shown either as a section
in the side panel or only in Modules (opened there as a window). The module
says which it prefers; your choice is kept in lenta's config.

## module.json

```json
{
  "id": "lsfg",
  "name": { "en": "Frame generation", "ru": "Генерация кадров" },
  "description": "LSFG-VK settings per game",
  "version": "1.0",
  "exec": ["python3", "main.py"],
  "icon": "icon.png",
  "pages": true,
  "show": "section",
  "game": { "label": "LSFG", "installed_only": true, "sources": ["steam"] }
}
```

- `exec`: the program and its arguments. The first is looked for in the
  module's folder first, then on `PATH`. It runs in the module's folder.
- `name`, `description`, `label` and every text in pages: a string, or
  `{"en": …, "ru": …}`.
- `icon`: a PNG in the folder, or one of lenta's icons: `settings`, `system`,
  `sound`, `proton`, `library`, `play`, `clock`, `trophy`, `about`.
- `pages`: the module has its own pages (its `main` page). `false` for a
  module that lives only on games' pages.
- `show`: `"section"` (default) or `"menu"` (only in Modules).
- `game`: `true`, or an object, for a tab on games' pages; `installed_only`
  and `sources` (`steam`, `heroic`, `lutris`, `hydra`, `own`) narrow it down.
  With two or more modules on a game's page their tabs are gathered into one,
  **Modules**, with a row of them on top (LB/RB or a click between them);
  Settings can give each its own tab again.

## The conversation

lenta starts the program once per question, writes one JSON line to its
stdin and reads the answer from stdout (the last line that is a JSON object).
Questions are asked one at a time, so the module can keep its state in plain
files. It has 20 seconds to answer.

Environment:

| Variable | |
|---|---|
| `LENTA_MODULE_DATA` | a folder for the module's own things (`~/.local/share/lenta/module-data/<id>`) |
| `LENTA_MODULE_DIR` | the module's folder |
| `LENTA_LANG` | `ru` or `en` |
| `LENTA_VERSION` | lenta's version |

Questions:

```json
{"op": "page",  "page": "main", "lang": "ru"}
{"op": "set",   "page": "main", "id": "fast", "value": true}
{"op": "press", "page": "main", "id": "reset"}
```

On a game's tab the page is `"game"` and every question has the game:

```json
"game": {"key": "steam:1245620", "title": "ELDEN RING", "source": "steam",
         "installed": true, "steam_appid": 1245620, "store_appid": null,
         "install_dir": "/…/ELDEN RING", "prefix": "/…/compatdata/1245620",
         "compat_tool": "proton_9"}
```

Answers (any of these together):

```json
{"title": "Settings", "items": [ … ]}   // the page (again, as it is now)
{"toast": "Saved"}                       // a word at the bottom
{"open": "advanced"}                     // go to another page
{"back": true}                           // back a page
{"error": "lsfg-vk isn't installed"}     // shown above the page
{"refresh": 5000}                        // ask for this page again in 5 s (while it's on screen)
{"launch": {"env": {…}, "wrap": ["mangohud"]}} // add to the game's start (see below)
{"restart_steam": true}                  // close Steam, write waiting changes, start it again
{"nav": [ … ]}                           // the module's own side panel (see below)
```

After `set` or `press`, an answer without `items` makes lenta ask for the page
again. lenta changes the value on screen at once and sends it; a slider held
down sends only where it stopped.

## Items

Every item may have `"hint"` (a second, smaller line) and `"disabled": true`.

```json
{"type": "header", "text": "Video"}
{"type": "text",   "text": "Any text, wrapped."}
{"type": "toggle", "id": "on",   "label": "Enabled", "value": true}
{"type": "slider", "id": "mult", "label": "Multiplier", "value": 2,
 "min": 2, "max": 4, "step": 1, "unit": "×"}
{"type": "choice", "id": "mode", "label": "Mode", "value": "fifo",
 "options": ["fifo", {"value": "mailbox", "label": "Mailbox"}]}
{"type": "input",  "id": "exe",  "label": "Executable", "value": "game.exe"}
{"type": "button", "id": "test", "label": "Try it", "text": "Run"}
{"type": "button", "id": "wipe", "label": "Forget everything", "text": "Reset", "danger": true}
{"type": "link",   "page": "advanced", "label": "Advanced", "value": "3 set"}
{"type": "file",   "id": "archive", "label": "Choose an archive…", "exts": [".zip", ".7z"], "start": "/home/me/Downloads"}
{"type": "progress", "label": "Unpacking", "value": 0.4, "hint": "40%"}
```

- `info` is a name and a value to read: `{"type": "info", "label": "Kernel", "value": "6.12"}`.
- `input` opens the on-screen keyboard; the text comes as `set`. With
  `"secret": true` it shows dots and the text isn't trimmed (passwords).
- `danger` buttons ask first.
- `link` opens the module's page by name; B goes back.
- `file` opens lenta's file browser (or the system's window, as set): the
  path chosen comes as `set`. `exts` narrows it to those files (Y shows all),
  `start` is the folder it opens in (Downloads otherwise).
- `progress` is a bar (`value` 0…1) with words beside it; answer with
  `refresh` to have it asked again while the work goes on (work longer than
  the 20 s an answer may take belongs to a program of its own the module
  starts, its stdio not tied to lenta's pipes).

## Changing how a game starts

On a game's tab (or with `"game": "<key>"` from anywhere) a module can add
environment variables and wrapper programs to the game's start:

```json
{"launch": {"env": {"MANGOHUD_CONFIGFILE": "/…/elden.conf"}, "wrap": ["mangohud"]}}
{"launch": {"game": "steam:1245620"}}     // nothing to add any more
```

Steam starts its games itself, so lenta puts a small script at the front of
the game's Steam launch options, once:
`~/.local/share/lenta/bin/lenta-run <game> <yours> --lenta-cmd %command% --lenta-end <yours>`
(your own options stay where they were around the game). The script reads
`~/.local/share/lenta/launch/<game>.sh`, which lenta rewrites whenever a
module changes its part, so later changes take effect on the next start
without touching Steam. Several modules can add to the same game. The same
script carries launch options changed in lenta's Launch tab while Steam runs.

Steam keeps launch options in memory while it runs, so the first time lenta
waits until Steam is closed, or restarts it when asked (`restart_steam`).
The game's JSON carries `"launch": "none" | "pending" | "ready"` so the
module can say so. When the last part goes, the word is taken out again.
For now this is for Steam games (`"sources": ["steam"]`).

## A side panel of its own

A module with several parts (Wi-Fi, Bluetooth, About…) answers with `nav`:

```json
"nav": [
  {"page": "wifi", "label": "Wi-Fi", "icon": "wifi", "value": "Home"},
  {"page": "bluetooth", "label": "Bluetooth", "icon": "bluetooth", "value": "On"},
  {"page": "about", "label": "About", "icon": "about"}
]
```

lenta draws it left of the page, the chosen part lit. The panel stays until
an answer brings a new one, so send it with every page to keep its values
fresh. The first page (`main`) counts as the panel's first part.
Pad: ↑ ↓ in the panel open the parts at once, → or A goes into the page,
← (on something without a value) or B comes back to the panel; LT/RT switch
parts from anywhere. Icons: `wifi`, `bluetooth`, `sliders` and the ones
above.

## Modules that come with lenta

`modules/mods` (the **Mods** module, a Rust program, `lenta-mods`) ships with
the packages: in `/usr/share/lenta/modules/mods`, or inside the Flatpak,
copied to your modules folder at start (modules run on the host).

## Trying it

Three examples:

- `examples/modules/mangohud` lives in Modules (its common settings) and on
  Steam games' pages: on or off per game, the common settings or its own,
  a try-out on `vkcube`.

- `examples/modules/demo` shows every item and a game tab.
- `examples/modules/system` is a settings section: Wi-Fi (NetworkManager:
  networks, connecting with a password, forgetting, turning the adapter off
  and on), Bluetooth (BlueZ: devices, searching, pairing, connecting) and
  About (the device, system, processor, graphics, memory, disk, battery).
  Under `LENTA_DRY_RUN` it changes nothing, only says what it would run.

Link one in:

```sh
mkdir -p ~/.local/share/lenta/modules
ln -s "$PWD/examples/modules/system" ~/.local/share/lenta/modules/system
```

A question can be tried by hand:

```sh
echo '{"op":"page","page":"main","lang":"en"}' | python3 examples/modules/demo/demo.py
```
