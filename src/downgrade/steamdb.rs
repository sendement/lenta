//! Manifest lists and builds pasted from SteamDB (ported from
//! steam-downgrader's steamdb.py, same results). SteamDB won't talk to
//! programs, so the user copies from the browser. Accepted:
//!
//! * the "Previously seen manifests" table of steamdb.info/depot/<id>/manifests/
//!   (a date and a manifest id per row; rows may wrap over several lines);
//! * SteamDB's copy formats: `-app A -depot D -manifest M` (DepotDownloader)
//!   and `download_depot A D M` (Steam console);
//! * bare manifest ids, one per line;
//! * the patch notes table of steamdb.info/app/<id>/patchnotes/: build ids
//!   with their dates and titles.
//!
//! Dates are read in the formats SteamDB uses and taken as UTC.

use std::sync::LazyLock;

use regex::Regex;

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

fn mon_alt() -> String {
    let full = MONTHS.join("|");
    let short: Vec<&str> = MONTHS.iter().map(|m| &m[..3]).collect();
    format!("{full}|{}", short.join("|"))
}

static DATES: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    let mon = mon_alt();
    let time = r"(?:\D{1,5}(?P<H>\d{1,2}):(?P<M>\d\d)(?::(?P<S>\d\d))?)?";
    [
        // 5 September 2025 – 18:23:11 UTC
        Regex::new(&format!(
            r"(?i)\b(?P<d>\d{{1,2}})\s+(?P<mon>{mon})\.?\s+(?P<y>\d{{4}}){time}"
        ))
        .unwrap(),
        // September 5, 2025 18:23:11
        Regex::new(&format!(
            r"(?i)\b(?P<mon>{mon})\.?\s+(?P<d>\d{{1,2}}),?\s+(?P<y>\d{{4}}){time}"
        ))
        .unwrap(),
        // 2025-09-05 18:23:11 / 2025-09-05T18:23:11Z
        Regex::new(
            r"\b(?P<y>\d{4})-(?P<m>\d\d)-(?P<d>\d\d)(?:[ T](?P<H>\d\d):(?P<M>\d\d)(?::(?P<S>\d\d))?)?",
        )
        .unwrap(),
    ]
});
static DD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)-app\s+(\d+).*?-depot\s+(\d+).*?-manifest\s+(\d+)").unwrap());
static CONSOLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)download_depot\s+(\d+)\s+(\d+)\s+(\d+)").unwrap());
static BUILD_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)build(?:\s*id)?\s*[:#]?\s*(\d{4,10})").unwrap());
static AGO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(?:a|an|\d+)\s+\w+\s+ago\b\s*").unwrap());

/// A manifest of a depot; `time` 0 when the paste had no date for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub depot: String,
    pub manifest: String,
    pub time: i64,
}

/// A build from the patch notes.
#[derive(Clone, Debug, PartialEq)]
pub struct Build {
    pub buildid: i64,
    pub time: i64,
    pub title: String,
}

fn days_in(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// UTC seconds, if it's a real date and time.
fn stamp(y: i64, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || d == 0 || d > days_in(y, m) || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hh as i64 * 3600 + mm as i64 * 60 + ss as i64)
}

/// The first date on the line: its time and where it stands (bytes).
fn date(line: &str) -> Option<(i64, (usize, usize))> {
    for rx in DATES.iter() {
        let Some(c) = rx.captures(line) else {
            continue;
        };
        let n = |k: &str| c.name(k).and_then(|m| m.as_str().parse::<u32>().ok());
        let month = match c.name("m") {
            Some(m) => m.as_str().parse().unwrap_or(0),
            None => {
                let name = c
                    .name("mon")
                    .map_or(String::new(), |m| m.as_str().to_lowercase());
                MONTHS
                    .iter()
                    .position(|full| *full == name)
                    .or_else(|| {
                        let p = &name[..name.len().min(3)];
                        MONTHS.iter().position(|full| full.starts_with(p))
                    })
                    .map_or(0, |i| i as u32 + 1)
            }
        };
        let y = c
            .name("y")
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let Some(t) = stamp(
            y,
            month,
            n("d").unwrap_or(0),
            n("H").unwrap_or(0),
            n("M").unwrap_or(0),
            n("S").unwrap_or(0),
        ) else {
            continue;
        };
        let m = c.get(0).unwrap();
        return Some((t, (m.start(), m.end())));
    }
    None
}

/// Whole runs of digits `lo..=hi` long, not right after one of `before`
/// nor right before one of `after` (what a look-around regex would do).
fn digit_runs<'a>(
    s: &'a str,
    lo: usize,
    hi: usize,
    before: &str,
    after: &str,
) -> Vec<(usize, usize, &'a str)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let len = i - start;
        let prev = s[..start].chars().next_back();
        let next = s[i..].chars().next();
        if (lo..=hi).contains(&len)
            && !prev.is_some_and(|c| before.contains(c))
            && !next.is_some_and(|c| after.contains(c))
        {
            out.push((start, i, &s[start..i]));
        }
    }
    out
}

fn manifest_ids(s: &str) -> Vec<&str> {
    digit_runs(s, 12, 20, ".:-", ".:")
        .into_iter()
        .map(|r| r.2)
        .collect()
}

fn build_id(s: &str) -> Option<(usize, usize, &str)> {
    digit_runs(s, 5, 10, ".:,/-", ".:,/").into_iter().next()
}

fn without(line: &str, span: (usize, usize), with: &str) -> String {
    format!("{}{with}{}", &line[..span.0], &line[span.1..])
}

/// Manifests in the text; those without a depot of their own go to
/// `default_depot`.
pub fn parse(text: &str, default_depot: &str) -> Vec<Entry> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    // (manifest, depot, date) per line, and the line's own date.
    let mut rows: Vec<(String, Option<String>, Option<i64>)> = Vec::new();
    let mut dates: Vec<Option<i64>> = Vec::new();
    for ln in &lines {
        let d = date(ln);
        let stripped = match d {
            Some((_, span)) => without(ln, span, " "),
            None => ln.to_string(),
        };
        dates.push(d.map(|d| d.0));
        if let Some(c) = DD.captures(ln).or_else(|| CONSOLE.captures(ln)) {
            rows.push((c[3].to_string(), Some(c[2].to_string()), d.map(|d| d.0)));
            continue;
        }
        match manifest_ids(&stripped).first() {
            Some(id) => rows.push((id.to_string(), None, d.map(|d| d.0))),
            None => rows.push((String::new(), None, None)),
        }
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, (manifest, depot, when)) in rows.iter().enumerate() {
        if manifest.is_empty() {
            continue;
        }
        let mut when = *when;
        if when.is_none() {
            // Table cells that wrapped: a date-only line just above or below.
            for j in [i as i64 - 1, i as i64 + 1, i as i64 - 2, i as i64 + 2] {
                if j >= 0
                    && (j as usize) < rows.len()
                    && dates[j as usize].is_some()
                    && rows[j as usize].0.is_empty()
                {
                    when = dates[j as usize];
                    break;
                }
            }
        }
        let depot = depot.clone().unwrap_or_else(|| default_depot.to_string());
        if !seen.insert((depot.clone(), manifest.clone())) {
            continue;
        }
        out.push(Entry {
            depot,
            manifest: manifest.clone(),
            time: when.unwrap_or(0),
        });
    }
    out
}

/// Rows of the patch notes table: date, build id, title. Lines with a
/// manifest id are left to [`parse`].
pub fn parse_builds(text: &str) -> Vec<Build> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut out: Vec<Build> = Vec::new();
    let dash = |c: char| " \t–—-|".contains(c);
    for (i, ln) in lines.iter().enumerate() {
        if ln.is_empty() || DD.is_match(ln) || CONSOLE.is_match(ln) {
            continue;
        }
        let d = date(ln);
        let rest = match d {
            Some((_, span)) => without(ln, span, "\t"),
            None => ln.to_string(),
        };
        if !manifest_ids(&rest).is_empty() {
            continue;
        }
        let (id, end) = if let Some(c) = BUILD_WORD.captures(&rest) {
            (c[1].to_string(), c.get(0).unwrap().end())
        } else if let Some((_, e, s)) = build_id(&rest) {
            (s.to_string(), e)
        } else {
            continue;
        };
        let Ok(buildid) = id.parse::<i64>() else {
            continue;
        };
        let mut when = d.map_or(0, |d| d.0);
        if when == 0 {
            // A wrapped row: the date on a neighbouring line.
            for j in [i as i64 - 1, i as i64 - 2, i as i64 + 1, i as i64 + 2] {
                if j < 0 || j as usize >= lines.len() {
                    continue;
                }
                let l = lines[j as usize];
                if let Some((t, span)) = date(l)
                    && build_id(&without(l, span, "")).is_none()
                {
                    when = t;
                    break;
                }
            }
        }
        let tail = rest[end..].trim_matches(dash);
        let title = AGO.replace(tail, "").trim_matches(dash).to_string();
        if !out.iter().any(|b| b.buildid == buildid) {
            out.push(Build {
                buildid,
                time: when,
                title,
            });
        }
    }
    out
}

pub fn manifests_url(depot: &str) -> String {
    format!("https://steamdb.info/depot/{depot}/manifests/")
}

pub fn patchnotes_url(app: &str) -> String {
    format!("https://steamdb.info/app/{app}/patchnotes/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(depot: &str, manifest: &str, time: i64) -> Entry {
        Entry {
            depot: depot.into(),
            manifest: manifest.into(),
            time,
        }
    }

    /// The same pastes as steam-downgrader's parser, the same results.
    #[test]
    fn same_as_steam_downgrader() {
        let table = "Seen Date\tRelative\tManifest ID\n1 March 2019 – 10:00:00 UTC\t7 years ago\t9876543210987654321\n13 September 2020 – 12:00:00 UTC\t6 years ago\t1234567890123456789\n\nDate\tBuild ID\tPatch notes\n13 September 2020 – 14:00:00 UTC\t5001000\tHotfix 1.0.5\n";
        assert_eq!(
            parse(table, "D"),
            vec![
                e("D", "9876543210987654321", 1551434400),
                e("D", "1234567890123456789", 1599998400)
            ]
        );
        assert_eq!(
            parse_builds(table),
            vec![Build {
                buildid: 5001000,
                time: 1600005600,
                title: "Hotfix 1.0.5".into()
            }]
        );
        let wrapped = "5 September 2025 – 18:23:11 UTC\n3 days ago\n4410000123456789012\nSeptember 1, 2025 09:00:00\n4410000123456789099\n";
        assert_eq!(
            parse(wrapped, "D"),
            vec![
                e("D", "4410000123456789012", 1756717200),
                e("D", "4410000123456789099", 1756717200)
            ]
        );
        assert!(parse_builds(wrapped).is_empty());
        let cmds = "download_depot 100 101 5550000000000000001\n-app 100 -depot 102 -manifest 5550000000000000002\n2024-02-03 04:05:06 7770000000000000003\n";
        assert_eq!(
            parse(cmds, "D"),
            vec![
                e("101", "5550000000000000001", 0),
                e("102", "5550000000000000002", 0),
                e("D", "7770000000000000003", 1706933106)
            ]
        );
        let builds = "Date\tBuild ID\tPatch notes\n12 May 2025 – 10:11:12 UTC\n2 months ago\t18734567\tPatch 1.2.3 is live\n3 Feb 2025\tBuildID 17000001 — Big update\n";
        assert!(parse(builds, "D").is_empty());
        assert_eq!(
            parse_builds(builds),
            vec![
                Build {
                    buildid: 18734567,
                    time: 1747044672,
                    title: "Patch 1.2.3 is live".into()
                },
                Build {
                    buildid: 17000001,
                    time: 1738540800,
                    title: "Big update".into()
                }
            ]
        );
    }
}
