#!/usr/bin/env python3
"""lenta module: MangoHud. Common settings in its window (Modules), and a
tab on every Steam game's page: on or off for that game, the common
settings or its own. Each game gets a config file; lenta starts the game
through `mangohud` with MANGOHUD_CONFIGFILE pointing at it (see "launch" in
docs/modules.md). Changes apply on the game's next start.
"""
import json
import os
import shutil
import subprocess
import sys

req = json.loads(sys.stdin.readline() or "{}")
RU = req.get("lang", os.environ.get("LENTA_LANG")) == "ru"
DRY = bool(os.environ.get("LENTA_DRY_RUN"))
DATA = os.environ.get("LENTA_MODULE_DATA", "/tmp/lenta-mangohud")
CONF = os.path.join(DATA, "conf")
STATE = os.path.join(DATA, "state.json")


def t(en, ru):
    return ru if RU else en


DEFAULTS = {"style": "full", "position": "top-left", "fps_limit": 0, "font_size": 24,
            "alpha": 0.5, "gpu_temp": True, "cpu_temp": True, "ram": False, "vram": False,
            "battery": False, "time": False, "hidden": False}


def load():
    try:
        with open(STATE) as f:
            s = json.load(f)
    except (OSError, ValueError):
        s = {}
    s.setdefault("common", {})
    s["common"] = {**DEFAULTS, **s["common"]}
    s.setdefault("games", {})
    return s


def save(s):
    os.makedirs(DATA, exist_ok=True)
    tmp = STATE + ".tmp"
    with open(tmp, "w") as f:
        json.dump(s, f, indent=1, ensure_ascii=False)
    os.replace(tmp, STATE)


def file_id(key):
    return "".join(c if c.isascii() and c.isalnum() else "_" for c in key)


def conf_text(c):
    """A MangoHud config from our settings."""
    lines = ["# Written by lenta's MangoHud module."]
    if c["hidden"]:
        lines.append("no_display")
    if c["style"] == "fps":
        lines.append("fps_only")
    else:
        if c["style"] == "compact":
            lines.append("hud_compact")
        if c["style"] == "row":
            lines += ["horizontal", "hud_compact"]
        for k in ("gpu_temp", "cpu_temp", "ram", "vram", "battery", "time"):
            if c[k]:
                lines.append(k)
    lines.append(f"position={c['position']}")
    lines.append(f"font_size={int(c['font_size'])}")
    lines.append(f"background_alpha={c['alpha']}")
    if c["fps_limit"]:
        lines.append(f"fps_limit={int(c['fps_limit'])}")
    lines.append("toggle_hud=Shift_R+F12")
    return "\n".join(lines) + "\n"


def write_conf(name, c):
    os.makedirs(CONF, exist_ok=True)
    path = os.path.join(CONF, name + ".conf")
    with open(path, "w") as f:
        f.write(conf_text(c))
    return path


def launch_of(s, key):
    """What lenta adds to the game's start ({"game": key} alone: nothing)."""
    g = s["games"].get(key)
    if not g or not g.get("on"):
        return {"game": key}
    if g.get("own"):
        path = write_conf(file_id(key), {**s["common"], **g.get("settings", {})})
    else:
        path = write_conf("common", s["common"])
    return {"game": key, "env": {"MANGOHUD_CONFIGFILE": path}, "wrap": ["mangohud"]}


STYLES = [("full", t("Full", "Полный")), ("compact", t("Compact", "Компактный")),
          ("row", t("In a row", "В строку")), ("fps", t("FPS only", "Только FPS"))]
PLACES = [("top-left", t("Top left", "Слева вверху")), ("top-center", t("Top centre", "Вверху по центру")),
          ("top-right", t("Top right", "Справа вверху")), ("bottom-left", t("Bottom left", "Слева внизу")),
          ("bottom-right", t("Bottom right", "Справа внизу"))]
LIMITS = [0, 30, 40, 45, 60, 72, 90, 120, 144, 165]


def settings_items(c, prefix=""):
    opt = lambda pairs: [{"value": v, "label": l} for v, l in pairs]
    fps_only = c["style"] == "fps"
    items = [
        {"type": "choice", "id": prefix + "style", "label": t("Look", "Вид"), "value": c["style"],
         "options": opt(STYLES)},
        {"type": "choice", "id": prefix + "position", "label": t("Place", "Где"), "value": c["position"],
         "options": opt(PLACES)},
        {"type": "choice", "id": prefix + "fps_limit", "label": t("FPS limit", "Ограничение FPS"),
         "value": c["fps_limit"],
         "options": [{"value": v, "label": str(v) if v else t("Off", "Нет")} for v in LIMITS]},
        {"type": "slider", "id": prefix + "font_size", "label": t("Text size", "Размер текста"),
         "value": c["font_size"], "min": 12, "max": 40, "step": 2},
        {"type": "slider", "id": prefix + "alpha", "label": t("Background", "Фон"),
         "value": c["alpha"], "min": 0, "max": 1, "step": 0.1},
        {"type": "toggle", "id": prefix + "hidden", "label": t("Hidden at start", "Скрыт при запуске"),
         "value": c["hidden"], "hint": t("Right Shift + F12 shows it", "Правый Shift + F12 показывает")},
        {"type": "header", "text": t("Show", "Показывать")},
    ]
    for k, en, ru in (("gpu_temp", "GPU temperature", "Температура видеокарты"),
                      ("cpu_temp", "CPU temperature", "Температура процессора"),
                      ("ram", "Memory", "Память"), ("vram", "Video memory", "Видеопамять"),
                      ("battery", "Battery", "Батарея"), ("time", "Clock", "Часы")):
        items.append({"type": "toggle", "id": prefix + k, "label": t(en, ru), "value": c[k],
                      "disabled": fps_only})
    return items


def try_button():
    if shutil.which("vkcube"):
        return [{"type": "button", "id": "try", "label": t("Try it on a test cube", "Проверить на тестовом кубе"),
                 "hint": t("Opens vkcube with these settings", "Откроет vkcube с этими настройками"),
                 "text": t("Try", "Проверить")}]
    return []


def hub(s):
    """Modules → MangoHud: common settings, the games it's on for."""
    if not shutil.which("mangohud"):
        return {"title": "MangoHud", "items": [{"type": "text", "text": t(
            "MangoHud isn't installed (the mangohud package).",
            "MangoHud не установлен (пакет mangohud).")}]}
    items = [{"type": "text", "text": t(
        "Turn it on for a game on the game's page (the MangoHud tab). These settings are for every game "
        "that doesn't have its own.",
        "Включается для игры на её странице, во вкладке MangoHud. Эти настройки — для всех игр, "
        "у которых нет своих.")},
             {"type": "header", "text": t("Common settings", "Общие настройки")}]
    items += settings_items(s["common"])
    items += try_button()
    on = [(k, g) for k, g in s["games"].items() if g.get("on")]
    items.append({"type": "header", "text": t("On for", "Включён в играх")})
    if not on:
        items.append({"type": "text", "text": t("No game yet.", "Пока ни в одной.")})
    for k, g in sorted(on, key=lambda x: x[1].get("title", "").lower()):
        items.append({"type": "link", "page": "g:" + k, "label": g.get("title", k),
                      "value": t("Own settings", "Свои настройки") if g.get("own") else t("Common", "Общие")})
    return {"title": "MangoHud", "items": items}


def game_page(s, key, title, launch_state):
    g = s["games"].get(key, {})
    on = bool(g.get("on"))
    items = [{"type": "toggle", "id": "on", "label": t("MangoHud in this game", "MangoHud в этой игре"),
              "value": on}]
    if on and launch_state == "pending":
        items += [{"type": "text", "text": t(
            "Steam has to be restarted once for it to take effect (it keeps launch options while it runs).",
            "Чтобы заработало, Steam нужно один раз перезапустить: пока он запущен, параметры запуска не меняются.")},
                  {"type": "button", "id": "restart", "label": t("Restart Steam", "Перезапустить Steam"),
                   "text": t("Restart", "Перезапустить")}]
    if on:
        items.append({"type": "choice", "id": "own", "label": t("Settings", "Настройки"),
                      "value": bool(g.get("own")),
                      "options": [{"value": False, "label": t("Common", "Общие")},
                                  {"value": True, "label": t("Its own", "Свои")}]})
        if g.get("own"):
            items.append({"type": "header", "text": t("This game's", "Для этой игры")})
            items += settings_items({**s["common"], **g.get("settings", {})}, "g.")
            items.append({"type": "button", "id": "copy", "label": t("Start over from the common ones", "Взять общие заново"),
                          "text": t("Reset", "Сбросить")})
        else:
            items.append({"type": "text", "text": t(
                "The common settings are changed in Modules → MangoHud.",
                "Общие настройки меняются в разделе Модули → MangoHud.")})
        items += try_button()
    return {"title": title, "items": items}


def try_it(path):
    if DRY:
        return {"toast": t("Dry run: mangohud vkcube", "Проверка: mangohud vkcube")}
    env = dict(os.environ, MANGOHUD_CONFIGFILE=path)
    subprocess.Popen(["mangohud", "vkcube"], env=env, start_new_session=True,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
    return {"toast": t("vkcube with MangoHud: close its window when done", "vkcube с MangoHud: закройте окно, когда посмотрите")}


def answer():
    s = load()
    op, page, iid, val = req.get("op"), req.get("page") or "main", req.get("id"), req.get("value")
    game = req.get("game")
    if game:
        key, title, state = game["key"], game.get("title", ""), game.get("launch", "none")
    elif page.startswith("g:"):
        key = page[2:]
        title, state = s["games"].get(key, {}).get("title", key), None
    else:
        key = title = state = None

    out = {}
    if op == "set":
        if key and iid == "on":
            g = s["games"].setdefault(key, {})
            g["on"], g["title"] = bool(val), title
            save(s)
            out["launch"] = launch_of(s, key)
        elif key and iid == "own":
            g = s["games"].setdefault(key, {"on": True, "title": title})
            g["own"] = bool(val)
            if val and "settings" not in g:
                g["settings"] = dict(s["common"])
            save(s)
            out["launch"] = launch_of(s, key)
        elif key and iid and iid.startswith("g."):
            g = s["games"].setdefault(key, {"on": True, "title": title})
            g.setdefault("settings", dict(s["common"]))[iid[2:]] = val
            save(s)
            out["launch"] = launch_of(s, key)
        elif iid in DEFAULTS:
            s["common"][iid] = val
            save(s)
            # Every game on the common settings reads the one file.
            write_conf("common", s["common"])
    elif op == "press":
        if iid == "restart":
            return {"restart_steam": True}
        if iid == "copy" and key:
            s["games"].setdefault(key, {})["settings"] = dict(s["common"])
            save(s)
            out["launch"] = launch_of(s, key)
            out["toast"] = t("Common settings taken", "Взяты общие настройки")
        if iid == "try":
            g = s["games"].get(key or "", {})
            c = {**s["common"], **g.get("settings", {})} if g.get("own") else s["common"]
            return try_it(write_conf("try", c))
    if key:
        out.update(game_page(s, key, title, state) if game or page.startswith("g:") else hub(s))
    else:
        out.update(hub(s))
    return out


print(json.dumps(answer(), ensure_ascii=False))
