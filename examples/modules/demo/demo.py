#!/usr/bin/env python3
"""A lenta module: one question as JSON on stdin, one answer on stdout.

Requests:  {"op": "page"|"set"|"press", "page": name, "id": item id,
            "value": new value, "game": {...} on a game's tab, "lang": "ru"|"en"}
Answers:   {"title": ..., "items": [...]}  (a page), and/or
           {"toast": "..."}, {"open": "page"}, {"back": true}, {"error": "..."}
See docs/modules.md in the lenta repository.
"""
import json
import os
import sys

req = json.loads(sys.stdin.readline() or "{}")
RU = req.get("lang") == "ru"
DATA = os.environ.get("LENTA_MODULE_DATA", ".")
STATE = os.path.join(DATA, "state.json")


def t(en, ru):
    return ru if RU else en


def load():
    try:
        with open(STATE) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def save(s):
    os.makedirs(DATA, exist_ok=True)
    tmp = STATE + ".tmp"
    with open(tmp, "w") as f:
        json.dump(s, f, indent=1)
    os.replace(tmp, STATE)


state = load()
main = state.setdefault("main", {"fast": True, "level": 3, "mode": "balanced", "name": ""})
game = req.get("game")
key = game["key"] if game else None
games = state.setdefault("games", {})
per = games.setdefault(key, {"on": False, "mult": 2, "flow": 1.0}) if key else None


def main_page():
    return {
        "title": t("Example", "Пример"),
        "items": [
            {"type": "header", "text": t("Settings", "Настройки")},
            {"type": "toggle", "id": "fast", "label": t("Fast mode", "Быстрый режим"),
             "value": main["fast"], "hint": t("A switch: A or ← →", "Переключатель: A или ← →")},
            {"type": "slider", "id": "level", "label": t("Level", "Уровень"),
             "value": main["level"], "min": 0, "max": 10, "step": 1},
            {"type": "choice", "id": "mode", "label": t("Mode", "Режим"), "value": main["mode"],
             "options": [
                 {"value": "quiet", "label": t("Quiet", "Тихий")},
                 {"value": "balanced", "label": t("Balanced", "Сбалансированный")},
                 {"value": "loud", "label": t("Loud", "Громкий")},
             ]},
            {"type": "input", "id": "name", "label": t("Name", "Имя"), "value": main["name"]},
            {"type": "header", "text": t("More", "Ещё")},
            {"type": "link", "page": "about", "label": t("About this module", "Об этом модуле"),
             "value": "v1.0"},
            {"type": "button", "id": "hello", "label": t("Say hello", "Поздороваться"),
             "text": t("Hello", "Привет")},
            {"type": "button", "id": "reset", "label": t("Reset everything", "Сбросить всё"),
             "text": t("Reset", "Сбросить"), "danger": True},
            {"type": "text", "text": t("Games set up: ", "Настроено игр: ")
             + str(sum(1 for g in state["games"].values() if g.get("on")))},
        ],
    }


def about_page():
    return {
        "title": t("About", "О модуле"),
        "items": [
            {"type": "text", "text": t(
                "This module is a Python script. lenta asks it for pages and draws them.",
                "Этот модуль — скрипт на Python. Лента спрашивает у него страницы и рисует их сама.")},
            {"type": "text", "text": t("Its things are kept in ", "Свои данные он хранит в ") + DATA},
        ],
    }


def game_page():
    on = per["on"]
    return {
        "title": game.get("title", ""),
        "items": [
            {"type": "toggle", "id": "on", "label": t("On for this game", "Включено для этой игры"),
             "value": on},
            {"type": "slider", "id": "mult", "label": t("Multiplier", "Множитель"), "value": per["mult"],
             "min": 2, "max": 4, "step": 1, "unit": "×", "disabled": not on},
            {"type": "slider", "id": "flow", "label": t("Flow scale", "Масштаб потока"), "value": per["flow"],
             "min": 0.25, "max": 1.0, "step": 0.05, "disabled": not on},
            {"type": "text", "text": t("Game: ", "Игра: ") + game.get("title", "")
             + (" · " + game["install_dir"] if game.get("install_dir") else "")},
        ],
    }


def page(name):
    if game:
        return game_page()
    return about_page() if name == "about" else main_page()


op = req.get("op")
out = {}
if op == "set":
    target = per if game else main
    target[req["id"]] = req["value"]
    save(state)
    out = page(req.get("page"))
elif op == "press":
    if req.get("id") == "hello":
        out = {"toast": t("Hello from the module!", "Привет от модуля!")}
    elif req.get("id") == "reset":
        state = {"games": {}}
        main = state.setdefault("main", {"fast": True, "level": 3, "mode": "balanced", "name": ""})
        save(state)
        out = dict(page("main"), toast=t("Reset", "Сброшено"))
else:
    out = page(req.get("page"))

print(json.dumps(out, ensure_ascii=False))
