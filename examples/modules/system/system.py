#!/usr/bin/env python3
"""lenta module: system settings — Wi-Fi (NetworkManager), Bluetooth (BlueZ)
and what this device is. One question as JSON on stdin, one answer on stdout;
see docs/modules.md.

Under LENTA_DRY_RUN (lenta's tests) nothing is changed: every action only
says what it would do.
"""
import json
import os
import platform
import shlex
import shutil
import signal
import subprocess
import sys
import time

req = json.loads(sys.stdin.readline() or "{}")
RU = req.get("lang", os.environ.get("LENTA_LANG")) == "ru"
DRY = bool(os.environ.get("LENTA_DRY_RUN"))
DATA = os.environ.get("LENTA_MODULE_DATA", "/tmp/lenta-system")
os.makedirs(DATA, exist_ok=True)
SCAN_PID = os.path.join(DATA, "bt-scan.pid")


def t(en, ru):
    return ru if RU else en


def run(*cmd, timeout=8, input=None):
    """Output of a command ('' when it isn't there or fails)."""
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, input=input)
        return r.stdout if r.returncode == 0 else ""
    except (OSError, subprocess.TimeoutExpired):
        return ""


def act(*cmd, timeout=15):
    """Change something: (ok, message). Only told under a dry run."""
    if DRY:
        return True, t("Dry run: ", "Проверка: ") + " ".join(cmd[:3])
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return False, t("No answer in time", "Нет ответа вовремя")
    except OSError as e:
        return False, str(e)
    out = (r.stderr or r.stdout).strip().splitlines()
    return r.returncode == 0, out[-1] if out else ""


def fields(line):
    """nmcli -t line: fields split on ':', '\\:' kept."""
    out, cur, esc = [], "", False
    for ch in line:
        if esc:
            cur += ch
            esc = False
        elif ch == "\\":
            esc = True
        elif ch == ":":
            out.append(cur)
            cur = ""
        else:
            cur += ch
    out.append(cur)
    return out


# ------------------------------------------------------------------ Wi-Fi

HAS_NM = shutil.which("nmcli") is not None


def wifi_on():
    return run("nmcli", "-t", "radio", "wifi").strip() == "enabled"


def wifi_dev():
    for line in run("nmcli", "-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "dev").splitlines():
        f = fields(line)
        if len(f) >= 4 and f[1] == "wifi":
            return {"dev": f[0], "state": f[2], "conn": f[3]}
    return None


def saved_wifi():
    out = set()
    for line in run("nmcli", "-t", "-f", "NAME,TYPE", "con", "show").splitlines():
        f = fields(line)
        if len(f) >= 2 and f[1] == "802-11-wireless":
            out.add(f[0])
    return out


def networks():
    nets = {}
    lines = run("nmcli", "-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY,CHAN,FREQ",
                "dev", "wifi", "list", "--rescan", "no").splitlines()
    for line in lines:
        f = fields(line)
        if len(f) < 6 or not f[1]:
            continue
        n = {"active": f[0] == "*", "ssid": f[1], "signal": int(f[2] or 0),
             "security": f[3], "chan": f[4], "freq": f[5]}
        old = nets.get(n["ssid"])
        if old is None or n["active"] or (not old["active"] and n["signal"] > old["signal"]):
            nets[n["ssid"]] = n
    return sorted(nets.values(), key=lambda n: (not n["active"], -n["signal"]))


def ip_of(dev):
    for line in run("nmcli", "-t", "-f", "IP4.ADDRESS,GENERAL.HWADDR", "dev", "show", dev).splitlines():
        if line.startswith("IP4.ADDRESS"):
            return line.split(":", 1)[1].split("/")[0]
    return ""


def wifi_page():
    if not HAS_NM:
        return {"title": "Wi-Fi", "items": [{"type": "text", "text": t(
            "NetworkManager (nmcli) isn't installed.", "NetworkManager (nmcli) не установлен.")}]}
    on = wifi_on()
    items = [{"type": "toggle", "id": "wifi", "label": "Wi-Fi", "value": on}]
    dev = wifi_dev()
    if on:
        saved = saved_wifi()
        nets = networks()
        if dev and dev["state"] == "connected":
            items.append({"type": "info", "label": t("Connected to", "Подключено к"),
                          "value": dev["conn"]})
            ip = ip_of(dev["dev"])
            if ip:
                items.append({"type": "info", "label": "IP", "value": ip})
        items.append({"type": "header", "text": t("Networks", "Сети")})
        if not nets:
            items.append({"type": "text", "text": t("No networks found yet.", "Сетей пока не найдено.")})
        for n in nets:
            if n["active"]:
                state = t("Connected", "Подключено")
            elif n["ssid"] in saved:
                state = t("Saved", "Сохранена")
            elif n["security"]:
                state = t("Secured", "Защищена")
            else:
                state = t("Open", "Открытая")
            items.append({"type": "link", "page": "net:" + n["ssid"], "label": n["ssid"],
                          "value": f"{state} · {n['signal']}%"})
        items.append({"type": "button", "id": "rescan", "label": t("Look for networks", "Искать сети"),
                      "text": t("Search", "Искать")})
    items.append({"type": "header", "text": t("Adapter", "Адаптер")})
    if dev:
        items.append({"type": "info", "label": t("Device", "Устройство"), "value": dev["dev"]})
    items.append({"type": "button", "id": "wifi_restart", "label": t("Reconnect the adapter", "Переподключить адаптер"),
                  "hint": t("Turns Wi-Fi off and on again", "Выключает и снова включает Wi-Fi"),
                  "text": t("Reconnect", "Переподключить"), "danger": True})
    return {"title": "Wi-Fi", "items": items, "refresh": 5000}


def net_page(ssid):
    n = next((x for x in networks() if x["ssid"] == ssid), None)
    saved = ssid in saved_wifi()
    items = []
    if n:
        items += [
            {"type": "info", "label": t("Signal", "Сигнал"), "value": f"{n['signal']}%"},
            {"type": "info", "label": t("Security", "Защита"), "value": n["security"] or t("None", "Нет")},
            {"type": "info", "label": t("Channel", "Канал"), "value": f"{n['chan']} · {n['freq']}"},
        ]
        if n["active"]:
            dev = wifi_dev()
            ip = ip_of(dev["dev"]) if dev else ""
            if ip:
                items.append({"type": "info", "label": "IP", "value": ip})
    else:
        items.append({"type": "text", "text": t("The network isn't in reach now.", "Сеть сейчас не видна.")})
    items.append({"type": "header", "text": ""})
    if n and n["active"]:
        items.append({"type": "button", "id": "disconnect", "label": t("Disconnect", "Отключиться"),
                      "text": t("Disconnect", "Отключить")})
    elif n and (saved or not n["security"]):
        items.append({"type": "button", "id": "connect", "label": t("Connect", "Подключиться"),
                      "text": t("Connect", "Подключить")})
    elif n:
        items.append({"type": "input", "id": "password", "label": t("Password", "Пароль"),
                      "value": "", "secret": True,
                      "hint": t("Type it to connect", "Введите, чтобы подключиться")})
    if saved:
        items.append({"type": "button", "id": "forget", "label": t("Forget this network", "Забыть сеть"),
                      "text": t("Forget", "Забыть"), "danger": True})
    return {"title": ssid, "items": items, "refresh": 5000}


def wifi_connect(ssid, password=None):
    if ssid in saved_wifi():
        ok, msg = act("nmcli", "--wait", "15", "con", "up", "id", ssid, timeout=18)
    elif password is not None:
        # The password goes on nmcli's command line (as nmcli wants it).
        ok, msg = act("nmcli", "--wait", "15", "dev", "wifi", "connect", ssid, "password", password,
                      timeout=18)
    else:
        ok, msg = act("nmcli", "--wait", "15", "dev", "wifi", "connect", ssid, timeout=18)
    if ok:
        return {"toast": msg if DRY else t("Connected to ", "Подключено к ") + ssid, "back": True}
    return {"error": msg or t("Couldn't connect", "Не удалось подключиться")}


# ------------------------------------------------------------------ Bluetooth

HAS_BT = shutil.which("bluetoothctl") is not None


def bt(*args, timeout=5):
    return run("bluetoothctl", *args, timeout=timeout)


def bt_powered():
    return "Powered: yes" in bt("show")


def bt_blocked():
    return "blocked" in run("rfkill", "list", "bluetooth").replace("Soft blocked: no", "").replace(
        "Hard blocked: no", "")


def bt_list(kind=None):
    args = ["devices"] + ([kind] if kind else [])
    out = []
    for line in bt(*args).splitlines():
        p = line.split(" ", 2)
        if len(p) >= 2 and p[0] == "Device":
            out.append((p[1], p[2] if len(p) > 2 else p[1]))
    return out


def bt_info(mac):
    info = {}
    for line in bt("info", mac).splitlines():
        line = line.strip()
        if ": " in line:
            k, v = line.split(": ", 1)
            info.setdefault(k, v)
    return info


def scanning():
    try:
        pid = int(open(SCAN_PID).read())
        os.kill(pid, 0)
        return True
    except (OSError, ValueError):
        return False


def battery(info):
    b = info.get("Battery Percentage", "")
    # "0x50 (80)"
    return b.split("(")[-1].rstrip(")") + "%" if "(" in b else ""


def bt_page():
    if not HAS_BT:
        return {"title": "Bluetooth", "items": [{"type": "text", "text": t(
            "BlueZ (bluetoothctl) isn't installed.", "BlueZ (bluetoothctl) не установлен.")}]}
    on = bt_powered()
    items = [{"type": "toggle", "id": "bt", "label": "Bluetooth", "value": on}]
    if on:
        paired = bt_list("Paired")
        pmacs = {m for m, _ in paired}
        items.append({"type": "header", "text": t("My devices", "Мои устройства")})
        if not paired:
            items.append({"type": "text", "text": t("Nothing paired yet.", "Пока ничего не сопряжено.")})
        for mac, name in paired:
            info = bt_info(mac)
            state = t("Connected", "Подключено") if info.get("Connected") == "yes" else t("Not connected", "Не подключено")
            bat = battery(info)
            items.append({"type": "link", "page": "bt:" + mac, "label": info.get("Alias", name),
                          "value": state + (" · " + bat if bat else "")})
        near = [(m, n) for m, n in bt_list() if m not in pmacs]
        # Those with a name first; a bare address says little.
        near.sort(key=lambda d: d[1].replace(":", "-") == d[0].replace(":", "-"))
        items.append({"type": "header", "text": t("Nearby", "Рядом")})
        if not near:
            items.append({"type": "text", "text": t("Search to find devices.", "Нажмите «Искать», чтобы найти устройства.")})
        for mac, name in near[:20]:
            items.append({"type": "link", "page": "bt:" + mac, "label": name, "value": mac})
        if scanning():
            items.append({"type": "button", "id": "scan_stop", "label": t("Searching…", "Идёт поиск…"),
                          "text": t("Stop", "Остановить")})
        else:
            items.append({"type": "button", "id": "scan", "label": t("Look for devices", "Искать устройства"),
                          "text": t("Search", "Искать")})
    elif bt_blocked():
        items.append({"type": "text", "text": t("Bluetooth is blocked (rfkill).", "Bluetooth заблокирован (rfkill).")})
    items.append({"type": "header", "text": t("Adapter", "Адаптер")})
    show = bt("show")
    for line in show.splitlines():
        line = line.strip()
        if line.startswith("Alias: "):
            items.append({"type": "info", "label": t("Name", "Имя"), "value": line[7:]})
    items.append({"type": "button", "id": "bt_restart", "label": t("Reconnect the adapter", "Переподключить адаптер"),
                  "hint": t("Turns Bluetooth off and on again", "Выключает и снова включает Bluetooth"),
                  "text": t("Reconnect", "Переподключить"), "danger": True})
    return {"title": "Bluetooth", "items": items, "refresh": 2000 if scanning() else 5000}


def dev_page(mac):
    info = bt_info(mac)
    name = info.get("Alias") or info.get("Name") or mac
    items = [{"type": "info", "label": t("Address", "Адрес"), "value": mac}]
    kinds = {"input-gaming": t("Gamepad", "Геймпад"), "audio-headset": t("Headset", "Гарнитура"),
             "audio-headphones": t("Headphones", "Наушники"), "audio-card": t("Speaker", "Колонка"),
             "input-keyboard": t("Keyboard", "Клавиатура"), "input-mouse": t("Mouse", "Мышь"),
             "phone": t("Phone", "Телефон"), "computer": t("Computer", "Компьютер")}
    if info.get("Icon"):
        items.append({"type": "info", "label": t("Kind", "Тип"), "value": kinds.get(info["Icon"], info["Icon"])})
    bat = battery(info)
    if bat:
        items.append({"type": "info", "label": t("Battery", "Заряд"), "value": bat})
    paired = info.get("Paired") == "yes"
    connected = info.get("Connected") == "yes"
    items.append({"type": "info", "label": t("State", "Состояние"), "value":
                  t("Connected", "Подключено") if connected else
                  t("Paired", "Сопряжено") if paired else t("Not paired", "Не сопряжено")})
    items.append({"type": "header", "text": ""})
    if connected:
        items.append({"type": "button", "id": "bt_disconnect", "label": t("Disconnect", "Отключить"),
                      "text": t("Disconnect", "Отключить")})
    elif paired:
        items.append({"type": "button", "id": "bt_connect", "label": t("Connect", "Подключить"),
                      "text": t("Connect", "Подключить")})
    else:
        items.append({"type": "button", "id": "bt_pair", "label": t("Pair and connect", "Сопрячь и подключить"),
                      "hint": t("Put the device in pairing mode first", "Сначала включите на устройстве режим сопряжения"),
                      "text": t("Pair", "Сопрячь")})
    if paired:
        items.append({"type": "button", "id": "bt_forget", "label": t("Forget this device", "Забыть устройство"),
                      "text": t("Forget", "Забыть"), "danger": True})
    return {"title": name, "items": items, "refresh": 4000}


def scan_start():
    if DRY:
        return {"toast": t("Dry run: bluetoothctl scan on", "Проверка: bluetoothctl scan on")}
    p = subprocess.Popen(["bluetoothctl", "--timeout", "30", "scan", "on"],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                         stdin=subprocess.DEVNULL, start_new_session=True)
    open(SCAN_PID, "w").write(str(p.pid))
    return {}


def scan_stop():
    try:
        os.kill(int(open(SCAN_PID).read()), signal.SIGTERM)
    except (OSError, ValueError):
        pass
    try:
        os.remove(SCAN_PID)
    except OSError:
        pass
    return {}


# ------------------------------------------------------------------ About

def read(path):
    try:
        return open(path).read().strip()
    except OSError:
        return ""


def size(b):
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if b < 1024 or unit == "TB":
            return f"{b:.1f} {unit}" if unit not in ("B", "KB") else f"{b:.0f} {unit}"
        b /= 1024
    return ""


def about_page():
    items = []
    vendor = read("/sys/class/dmi/id/sys_vendor")
    product = read("/sys/class/dmi/id/product_version") if vendor.lower().startswith("lenovo") else ""
    product = product or read("/sys/class/dmi/id/product_name")
    if vendor or product:
        items.append({"type": "info", "label": t("Device", "Устройство"), "value": f"{vendor} {product}".strip()})
    os_name = ""
    for line in read("/etc/os-release").splitlines():
        if line.startswith("PRETTY_NAME="):
            os_name = line.split("=", 1)[1].strip('"')
    items.append({"type": "info", "label": t("System", "Система"), "value": os_name or platform.system()})
    items.append({"type": "info", "label": t("Kernel", "Ядро"), "value": platform.release()})
    desk = os.environ.get("XDG_CURRENT_DESKTOP", "")
    if desk:
        items.append({"type": "info", "label": t("Desktop", "Окружение"), "value": desk})
    items.append({"type": "info", "label": t("Name", "Имя"), "value": platform.node()})
    items.append({"type": "header", "text": t("Hardware", "Железо")})
    cpu = ""
    for line in read("/proc/cpuinfo").splitlines():
        if line.startswith("model name"):
            cpu = line.split(":", 1)[1].strip()
            break
    items.append({"type": "info", "label": t("Processor", "Процессор"),
                  "value": f"{cpu} · {os.cpu_count()} " + t("threads", "потоков")})
    for line in run("lspci", "-mm").splitlines():
        try:
            f = shlex.split(line)
        except ValueError:
            continue
        # slot, class, vendor, device…
        if len(f) >= 4 and any(k in f[1] for k in ("VGA", "3D", "Display")) \
                and not f[3].startswith("Device "):
            vendor = f[2].replace(" Corporation", "").replace("Advanced Micro Devices, Inc. ", "")
            items.append({"type": "info", "label": t("Graphics", "Видеокарта"), "value": f"{vendor} {f[3]}"})
    mem = {}
    for line in read("/proc/meminfo").splitlines():
        k, _, v = line.partition(":")
        mem[k] = int(v.split()[0]) * 1024 if v.split() else 0
    if mem.get("MemTotal"):
        used = mem["MemTotal"] - mem.get("MemAvailable", 0)
        items.append({"type": "info", "label": t("Memory", "Память"),
                      "value": f"{size(used)} / {size(mem['MemTotal'])}"})
    du = shutil.disk_usage("/")
    items.append({"type": "info", "label": t("Disk", "Диск"),
                  "value": t(f"{size(du.free)} free of {size(du.total)}", f"свободно {size(du.free)} из {size(du.total)}")})
    for bat in sorted(os.listdir("/sys/class/power_supply")) if os.path.isdir("/sys/class/power_supply") else []:
        base = "/sys/class/power_supply/" + bat
        if read(base + "/type") == "Battery":
            cap, st = read(base + "/capacity"), read(base + "/status")
            states = {"Charging": t("charging", "заряжается"), "Discharging": t("on battery", "от батареи"),
                      "Full": t("full", "полная"), "Not charging": t("not charging", "не заряжается")}
            items.append({"type": "info", "label": t("Battery", "Батарея"),
                          "value": f"{cap}% · {states.get(st, st.lower())}"})
    up = float(read("/proc/uptime").split()[0] or 0) if read("/proc/uptime") else 0
    h, m = int(up // 3600), int(up % 3600 // 60)
    items.append({"type": "info", "label": t("Up for", "Работает"),
                  "value": t(f"{h} h {m} min", f"{h} ч {m} мин")})
    items.append({"type": "info", "label": "lenta", "value": os.environ.get("LENTA_VERSION", "")})
    return {"title": t("About", "О системе"), "items": items, "refresh": 10000}


# ------------------------------------------------------------------ the answer

def nav():
    wifi = t("Off", "Выкл")
    if HAS_NM and wifi_on():
        d = wifi_dev()
        wifi = d["conn"] if d and d["state"] == "connected" else t("On", "Вкл")
    btv = t("On", "Вкл") if HAS_BT and bt_powered() else t("Off", "Выкл")
    return [
        {"page": "wifi", "label": "Wi-Fi", "icon": "wifi", "value": wifi},
        {"page": "bluetooth", "label": "Bluetooth", "icon": "bluetooth", "value": btv},
        {"page": "about", "label": t("About", "О системе"), "icon": "about"},
    ]


def page(name):
    if name.startswith("net:"):
        return net_page(name[4:])
    if name.startswith("bt:"):
        return dev_page(name[3:])
    if name == "bluetooth":
        return bt_page()
    if name == "about":
        return about_page()
    return wifi_page()


def answer():
    op, name = req.get("op"), req.get("page") or "wifi"
    if name == "main":
        name = "wifi"
    iid, val = req.get("id"), req.get("value")
    if op == "set":
        if iid == "wifi":
            ok, msg = act("nmcli", "radio", "wifi", "on" if val else "off")
            return {"toast": msg} if DRY or not ok else {}
        if iid == "bt":
            if val and bt_blocked():
                act("rfkill", "unblock", "bluetooth")
                time.sleep(0.5)
            ok, msg = act("bluetoothctl", "power", "on" if val else "off")
            return {"toast": msg} if DRY else ({} if ok else {"error": msg})
        if iid == "password" and name.startswith("net:"):
            return wifi_connect(name[4:], val)
        return {}
    if op == "press":
        if iid == "rescan":
            act("nmcli", "dev", "wifi", "rescan")
            return {"toast": t("Looking for networks…", "Ищем сети…")}
        if iid == "wifi_restart":
            act("nmcli", "radio", "wifi", "off")
            time.sleep(1)
            ok, msg = act("nmcli", "radio", "wifi", "on")
            return {"toast": msg if DRY else t("Wi-Fi reconnected", "Wi-Fi переподключён")}
        if iid == "connect":
            return wifi_connect(name[4:])
        if iid == "disconnect":
            d = wifi_dev()
            ok, msg = act("nmcli", "dev", "disconnect", d["dev"]) if d else (False, "")
            return {"toast": msg} if DRY or not ok else {"back": True}
        if iid == "forget":
            ok, msg = act("nmcli", "con", "delete", "id", name[4:])
            return {"toast": msg, "back": True} if ok else {"error": msg}
        if iid == "scan":
            return scan_start()
        if iid == "scan_stop":
            return scan_stop()
        if iid == "bt_restart":
            act("bluetoothctl", "power", "off")
            time.sleep(1)
            ok, msg = act("bluetoothctl", "power", "on")
            return {"toast": msg if DRY else t("Bluetooth reconnected", "Bluetooth переподключён")}
        mac = name[3:] if name.startswith("bt:") else ""
        if iid == "bt_connect":
            ok, msg = act("bluetoothctl", "--timeout", "12", "connect", mac)
            return {"toast": msg} if ok else {"error": msg or t("Couldn't connect", "Не удалось подключить")}
        if iid == "bt_disconnect":
            ok, msg = act("bluetoothctl", "disconnect", mac)
            return {"toast": msg} if DRY or not ok else {}
        if iid == "bt_pair":
            ok, msg = act("bluetoothctl", "--timeout", "12", "pair", mac)
            if ok and not DRY:
                act("bluetoothctl", "trust", mac)
                act("bluetoothctl", "--timeout", "8", "connect", mac)
            return {"toast": msg} if ok else {"error": msg or t("Couldn't pair", "Не удалось сопрячь")}
        if iid == "bt_forget":
            ok, msg = act("bluetoothctl", "remove", mac)
            return {"toast": msg, "back": True} if ok else {"error": msg}
        return {}
    return page(name)


out = answer()
# Every answer about a page brings the panel along (its states change).
if "items" in out or req.get("op") == "page":
    out["nav"] = nav()
print(json.dumps(out, ensure_ascii=False))
