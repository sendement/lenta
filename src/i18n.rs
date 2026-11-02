//! Interface text in Russian and English.

use std::sync::atomic::{AtomicU8, Ordering};

static LANG: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum Lang {
    #[default]
    Auto,
    Ru,
    En,
}

/// 0 = English, 1 = Russian.
pub fn set(lang: Lang) {
    let ru = match lang {
        Lang::Ru => true,
        Lang::En => false,
        Lang::Auto => ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .is_some_and(|v| v.starts_with("ru")),
    };
    LANG.store(ru as u8, Ordering::Relaxed);
}

pub fn ru() -> bool {
    LANG.load(Ordering::Relaxed) == 1
}

/// Steam's name for the language (store and achievements).
pub fn steam_lang() -> &'static str {
    if ru() { "russian" } else { "english" }
}

pub fn tr(key: &str) -> &'static str {
    let (en_text, ru_text) = match key {
        "home" => ("Home", "Главная"),
        "library" => ("Library", "Библиотека"),
        "system" => ("System apps", "Системные"),
        "settings" => ("Settings", "Настройки"),
        "about" => ("About", "О приложении"),
        "play" => ("Play", "Играть"),
        "open" => ("Open", "Открыть"),
        "back" => ("Back", "Назад"),
        "menu" => ("Menu", "Меню"),
        "launch" => ("Launch", "Запустить"),
        "change" => ("Change", "Изменить"),
        "filter" => ("Filter", "Фильтр"),
        "scroll" => ("Scroll", "Прокрутка"),
        "pages" => ("Pages", "Разделы"),
        "tab" => ("Tab", "Вкладка"),
        "overview" => ("Overview", "Обзор"),
        "achievements" => ("Achievements", "Достижения"),
        "screenshots" => ("Screenshots", "Скриншоты"),
        "playtime" => ("Played", "Наиграно"),
        "last_played" => ("Last played", "Последний запуск"),
        "never" => ("never", "никогда"),
        "hours" => ("h", "ч"),
        "minutes" => ("min", "мин"),
        "developer" => ("Developer", "Разработчик"),
        "publisher" => ("Publisher", "Издатель"),
        "genres" => ("Genres", "Жанры"),
        "released" => ("Released", "Дата выхода"),
        "metacritic" => ("Metacritic", "Metacritic"),
        "folder" => ("Folder", "Папка"),
        "source" => ("Source", "Источник"),
        "loading" => ("Loading…", "Загрузка…"),
        "no_desc" => ("No description.", "Описания нет."),
        "offline" => ("The store can't be reached.", "Магазин недоступен."),
        "no_ach" => (
            "This game has no achievements, or Steam hasn't cached them yet.",
            "У игры нет достижений, или Steam их ещё не закэшировал.",
        ),
        "hidden_ach" => ("Hidden achievement", "Скрытое достижение"),
        "unlocked" => ("Unlocked", "Получено"),
        "no_games" => (
            "No games found. Steam, Heroic and Lutris are looked up.",
            "Игр не найдено. Ищем в Steam, Heroic и Lutris.",
        ),
        "scanning" => ("Looking for games…", "Ищем игры…"),
        "launching" => ("Launching", "Запуск"),
        "launch_failed" => ("Couldn't launch", "Не удалось запустить"),
        "all" => ("All", "Все"),
        "games_count" => ("games", "игр"),
        "apps_count" => ("apps", "приложений"),
        "no_shots" => ("No screenshots.", "Скриншотов нет."),
        "about_text" => (
            "A gamepad-first launcher: games from Steam, Heroic and Lutris on one ribbon, system programs next to them.",
            "Лаунчер под геймпад: игры из Steam, Heroic и Lutris на одной ленте, а рядом — системные программы.",
        ),
        "license" => ("MIT license", "Лицензия MIT"),
        "found" => ("Found", "Найдено"),
        "controls" => ("Controls", "Управление"),
        "ctl_move" => (
            "D-pad / stick / arrows — move",
            "Крестовина / стик / стрелки — перемещение",
        ),
        "ctl_accept" => (
            "A / ✕ / Enter — open, press",
            "A / ✕ / Enter — открыть, нажать",
        ),
        "ctl_play" => (
            "X / □ / Space — play at once",
            "X / □ / Space — сразу играть",
        ),
        "ctl_back" => ("B / ○ / Esc — back", "B / ○ / Esc — назад"),
        "ctl_pages" => (
            "LB / RB / Q / E — previous / next section",
            "LB / RB / Q / E — предыдущий / следующий раздел",
        ),
        "ctl_side" => ("View / Tab — side panel", "View / Tab — боковая панель"),
        "ctl_trig" => (
            "LT / RT / PgUp / PgDn — filter, tabs",
            "LT / RT / PgUp / PgDn — фильтр, вкладки",
        ),
        "ctl_full" => ("F11 — full screen", "F11 — полный экран"),
        "backlog" => (
            "Coming: Hydra Launcher games.",
            "В планах: игры из Hydra Launcher.",
        ),
        // Settings.
        "s_general" => ("General", "Общие"),
        "s_ribbon" => ("Ribbon", "Лента"),
        "s_cards" => ("Cards", "Карточки"),
        "s_accent" => ("Accent", "Акцент"),
        "s_back" => ("Background", "Фон"),
        "s_sources" => ("Sources", "Источники"),
        "lang" => ("Language", "Язык"),
        "lang_auto" => ("System", "Как в системе"),
        "fullscreen" => ("Full screen", "Полный экран"),
        "sidebar_pinned" => ("Side panel always open", "Боковая панель всегда раскрыта"),
        "orientation" => ("Direction", "Направление"),
        "horizontal" => ("Horizontal", "Горизонтальная"),
        "vertical" => ("Vertical", "Вертикальная"),
        "flow" => ("Flow", "Порядок"),
        "ltr" => ("Left to right", "Слева направо"),
        "rtl" => ("Right to left", "Справа налево"),
        "ttb" => ("Top to bottom", "Сверху вниз"),
        "btt" => ("Bottom to top", "Снизу вверх"),
        "anchor" => ("Selected card", "Выбранная карточка"),
        "anchor_center" => ("In the middle", "По центру"),
        "anchor_start" => ("At the start", "В начале"),
        "sort" => ("Order", "Сортировка"),
        "sort_recent" => ("Recently played", "Недавние"),
        "sort_name" => ("By name", "По названию"),
        "sort_time" => ("Most played", "По времени в игре"),
        "shape" => ("Shape", "Форма"),
        "portrait" => ("Vertical (cover)", "Вертикальные (обложка)"),
        "landscape" => ("Horizontal (Steam)", "Горизонтальные (как в Steam)"),
        "card_size" => ("Size", "Размер"),
        "spacing" => ("Gap", "Отступ"),
        "sel_scale" => ("Selected zoom", "Увеличение выбранной"),
        "dim" => ("Dim the others", "Затемнять остальные"),
        "tilt" => ("Tilt", "Угол наклона"),
        "tilt_mode" => ("Tilt kind", "Вид наклона"),
        "tilt_flow" => ("Towards the middle", "К центру"),
        "tilt_fan" => ("All one way", "Все в одну сторону"),
        "rounding" => ("Rounding", "Скругление"),
        "reflection" => ("Reflection", "Отражение"),
        "titles" => ("Title under the card", "Название под карточкой"),
        "accent_mode" => ("Accent color", "Цвет акцента"),
        "acc_custom" => ("Own color", "Свой цвет"),
        "acc_gradient" => ("Gradient", "Градиент"),
        "acc_card" => ("From the card", "По цвету карточки"),
        "acc_card_grad" => ("Gradient from the card", "Градиент по карточке"),
        "color1" => ("Color", "Цвет"),
        "color2" => ("Second color", "Второй цвет"),
        "glow" => ("Glow", "Свечение"),
        "bg_hero" => ("Game art behind", "Арт игры на фоне"),
        "bg_dim" => ("Darken the background", "Затемнение фона"),
        "bg_tint" => ("Tint with the accent", "Подкрашивать акцентом"),
        "speed" => ("Animation speed", "Скорость анимации"),
        "src_steam" => ("Steam", "Steam"),
        "src_heroic" => ("Heroic (Epic, GOG, Amazon)", "Heroic (Epic, GOG, Amazon)"),
        "src_lutris" => ("Lutris", "Lutris"),
        "rescan" => ("Look for games again", "Найти игры заново"),
        "on" => ("On", "Вкл"),
        "off" => ("Off", "Выкл"),
        "hue" => ("Hue", "Оттенок"),
        "sat" => ("Saturation", "Насыщенность"),
        "val" => ("Brightness", "Яркость"),
        "done" => ("Done", "Готово"),
        // System app groups.
        "grp_games" => ("Games", "Игры"),
        "grp_internet" => ("Internet", "Интернет"),
        "grp_media" => ("Media", "Мультимедиа"),
        "grp_graphics" => ("Graphics", "Графика"),
        "grp_office" => ("Office", "Офис"),
        "grp_dev" => ("Development", "Разработка"),
        "grp_system" => ("System", "Система"),
        "grp_other" => ("Other", "Другое"),
        _ => {
            debug_assert!(false, "no text for {key:?}");
            return "?";
        }
    };
    if ru() { ru_text } else { en_text }
}

/// "12 ч 30 мин" / "45 min".
pub fn duration(min: u64) -> String {
    let (h, m) = (min / 60, min % 60);
    if h == 0 {
        format!("{m} {}", tr("minutes"))
    } else if h >= 10 || m == 0 {
        format!("{h} {}", tr("hours"))
    } else {
        format!("{h} {} {m} {}", tr("hours"), tr("minutes"))
    }
}
