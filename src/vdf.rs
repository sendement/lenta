//! Valve's key/values: the text form (libraryfolders.vdf, appmanifest_*.acf,
//! localconfig.vdf) and the binary one (appcache/stats, shortcuts.vdf,
//! appinfo.vdf), both read into the same tree.

#[derive(Clone, Debug, PartialEq)]
pub enum Kv {
    Map(Vec<(String, Kv)>),
    Str(String),
    Int(i64),
    Float(f32),
}

impl Kv {
    /// A child by key, ignoring case (Steam writes "apps" and "Apps").
    pub fn get(&self, key: &str) -> Option<&Kv> {
        match self {
            Kv::Map(m) => m
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Kv> {
        keys.iter().try_fold(self, |n, k| n.get(k))
    }

    pub fn entries(&self) -> &[(String, Kv)] {
        match self {
            Kv::Map(m) => m,
            _ => &[],
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Kv::Str(s) => Some(s),
            _ => None,
        }
    }

    /// A number, whether stored as one or as text.
    pub fn int(&self) -> Option<i64> {
        match self {
            Kv::Int(i) => Some(*i),
            Kv::Float(f) => Some(*f as i64),
            Kv::Str(s) => s.trim().parse().ok(),
            Kv::Map(_) => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Kv::str)
    }

    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Kv::int)
    }

    /// The child map under `key`, made (empty) when missing. None if this
    /// isn't a map or the child isn't one.
    pub fn map_mut(&mut self, key: &str) -> Option<&mut Kv> {
        let Kv::Map(m) = self else { return None };
        let i = match m.iter().position(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some(i) => i,
            None => {
                m.push((key.to_string(), Kv::Map(Vec::new())));
                m.len() - 1
            }
        };
        let v = &mut m[i].1;
        matches!(v, Kv::Map(_)).then_some(v)
    }

    pub fn path_mut(&mut self, keys: &[&str]) -> Option<&mut Kv> {
        keys.iter().try_fold(self, |n, k| n.map_mut(k))
    }

    /// Set `key` to `v`, in place if it's there.
    pub fn set(&mut self, key: &str, v: Kv) {
        if let Kv::Map(m) = self {
            match m.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                Some(e) => e.1 = v,
                None => m.push((key.to_string(), v)),
            }
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let Kv::Map(m) = self {
            m.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        }
    }

    /// Back to the text form, laid out as Steam writes it.
    pub fn to_text(&self) -> String {
        fn quote(s: &str) -> String {
            let mut o = String::with_capacity(s.len() + 2);
            o.push('"');
            for c in s.chars() {
                match c {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    // Steam keeps line breaks and tabs as they are.
                    c => o.push(c),
                }
            }
            o.push('"');
            o
        }
        fn write(out: &mut String, m: &[(String, Kv)], depth: usize) {
            let tabs = "\t".repeat(depth);
            for (k, v) in m {
                match v {
                    Kv::Map(c) => {
                        out.push_str(&format!("{tabs}{}\n{tabs}{{\n", quote(k)));
                        write(out, c, depth + 1);
                        out.push_str(&format!("{tabs}}}\n"));
                    }
                    Kv::Str(s) => out.push_str(&format!("{tabs}{}\t\t{}\n", quote(k), quote(s))),
                    Kv::Int(i) => out.push_str(&format!("{tabs}{}\t\t\"{i}\"\n", quote(k))),
                    Kv::Float(f) => out.push_str(&format!("{tabs}{}\t\t\"{f}\"\n", quote(k))),
                }
            }
        }
        let mut out = String::new();
        write(&mut out, self.entries(), 0);
        out
    }
}

// ------------------------------------------------------------------ text

/// The text form: `"key" "value"` and `"key" { … }`, `//` comments.
pub fn parse_text(s: &str) -> Kv {
    let mut toks = Tokens {
        s: s.as_bytes(),
        at: 0,
    };
    Kv::Map(text_map(&mut toks, true))
}

struct Tokens<'a> {
    s: &'a [u8],
    at: usize,
}

enum Tok {
    Str(String),
    Open,
    Close,
}

impl Tokens<'_> {
    fn next(&mut self) -> Option<Tok> {
        loop {
            let c = *self.s.get(self.at)?;
            if c.is_ascii_whitespace() {
                self.at += 1;
            } else if c == b'/' && self.s.get(self.at + 1) == Some(&b'/') {
                while self.s.get(self.at).is_some_and(|&c| c != b'\n') {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
        let c = self.s[self.at];
        self.at += 1;
        match c {
            b'{' => Some(Tok::Open),
            b'}' => Some(Tok::Close),
            b'"' => {
                let mut out = Vec::new();
                while let Some(&c) = self.s.get(self.at) {
                    self.at += 1;
                    match c {
                        b'"' => break,
                        b'\\' => {
                            let e = self.s.get(self.at).copied().unwrap_or(b'\\');
                            self.at += 1;
                            out.push(match e {
                                b'n' => b'\n',
                                b't' => b'\t',
                                other => other,
                            });
                        }
                        c => out.push(c),
                    }
                }
                Some(Tok::Str(String::from_utf8_lossy(&out).into_owned()))
            }
            _ => {
                // Bare word.
                let start = self.at - 1;
                while self
                    .s
                    .get(self.at)
                    .is_some_and(|c| !c.is_ascii_whitespace() && !b"{}\"".contains(c))
                {
                    self.at += 1;
                }
                Some(Tok::Str(
                    String::from_utf8_lossy(&self.s[start..self.at]).into_owned(),
                ))
            }
        }
    }
}

fn text_map(t: &mut Tokens, top: bool) -> Vec<(String, Kv)> {
    let mut out = Vec::new();
    loop {
        let key = match t.next() {
            Some(Tok::Str(k)) => k,
            Some(Tok::Close) if !top => return out,
            None => return out,
            _ => continue,
        };
        match t.next() {
            Some(Tok::Str(v)) => out.push((key, Kv::Str(v))),
            Some(Tok::Open) => {
                let m = text_map(t, false);
                out.push((key, Kv::Map(m)));
            }
            _ => return out,
        }
    }
}

// ---------------------------------------------------------------- binary

/// The binary form; `table` holds the keys when they are stored as indexes
/// (appinfo.vdf v29).
pub fn parse_binary(b: &[u8], table: Option<&[String]>) -> Option<Kv> {
    let mut r = Reader { b, at: 0 };
    Some(Kv::Map(r.map(table)?))
}

pub struct Reader<'a> {
    pub b: &'a [u8],
    pub at: usize,
}

impl<'a> Reader<'a> {
    pub fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.at)?;
        self.at += 1;
        Some(v)
    }
    pub fn u32(&mut self) -> Option<u32> {
        let s = self.b.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_le_bytes(s.try_into().ok()?))
    }
    pub fn u64(&mut self) -> Option<u64> {
        let s = self.b.get(self.at..self.at + 8)?;
        self.at += 8;
        Some(u64::from_le_bytes(s.try_into().ok()?))
    }
    pub fn cstr(&mut self) -> Option<&'a str> {
        let rest = self.b.get(self.at..)?;
        let end = rest.iter().position(|&c| c == 0)?;
        self.at += end + 1;
        Some(std::str::from_utf8(&rest[..end]).unwrap_or(""))
    }

    pub fn map(&mut self, table: Option<&[String]>) -> Option<Vec<(String, Kv)>> {
        let mut out = Vec::new();
        loop {
            let ty = self.u8()?;
            if ty == 0x08 || ty == 0x0b {
                return Some(out);
            }
            let key = match table {
                Some(t) => t.get(self.u32()? as usize)?.clone(),
                None => self.cstr()?.to_string(),
            };
            let v = match ty {
                0x00 => Kv::Map(self.map(table)?),
                0x01 => Kv::Str(self.cstr()?.to_string()),
                0x02 => Kv::Int(self.u32()? as i32 as i64),
                0x03 => Kv::Float(f32::from_bits(self.u32()?)),
                0x04 | 0x06 => Kv::Int(self.u32()? as i64),
                0x07 => Kv::Int(self.u64()? as i64),
                0x0a => Kv::Int(self.u64()? as i64),
                0x05 => {
                    // UTF-16, ends in two zero bytes.
                    let mut units = Vec::new();
                    loop {
                        let lo = self.u8()?;
                        let hi = self.u8()?;
                        if lo == 0 && hi == 0 {
                            break;
                        }
                        units.push(u16::from_le_bytes([lo, hi]));
                    }
                    Kv::Str(String::from_utf16_lossy(&units))
                }
                _ => return None,
            };
            out.push((key, v));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text() {
        let kv = parse_text(
            r#""UserLocalConfigStore"
{
    // comment
    "Software" { "Valve" { "Steam" { "apps" {
        "570" { "LastPlayed" "1700000000" "Playtime" "125" }
    } } } }
    "path"  "C:\\Games"
}"#,
        );
        let app = kv
            .path(&[
                "UserLocalConfigStore",
                "software",
                "valve",
                "steam",
                "Apps",
                "570",
            ])
            .unwrap();
        assert_eq!(app.get_int("playtime"), Some(125));
        assert_eq!(
            kv.path(&["UserLocalConfigStore", "path"]).and_then(Kv::str),
            Some("C:\\Games")
        );
    }

    #[test]
    fn text_round_trip() {
        let src = "\"InstallConfigStore\"\n{\n\t\"a\"\t\t\"C:\\\\x \\\"q\\\"\"\n\t\"m\"\n\t{\n\t\t\"k\"\t\t\"v\"\n\t}\n}\n";
        let kv = parse_text(src);
        assert_eq!(kv.to_text(), src);
        assert_eq!(parse_text(&kv.to_text()), kv);
    }

    #[test]
    fn edit() {
        let mut kv = parse_text("\"r\" { \"x\" \"1\" }");
        let m = kv.path_mut(&["r", "CompatToolMapping", "570"]).unwrap();
        m.set("name", Kv::Str("proton_9".into()));
        assert_eq!(
            kv.path(&["r", "compattoolmapping", "570", "name"])
                .and_then(Kv::str),
            Some("proton_9")
        );
        kv.path_mut(&["r", "CompatToolMapping"])
            .unwrap()
            .remove("570");
        assert!(kv.path(&["r", "CompatToolMapping", "570"]).is_none());
    }

    /// Steam's own config.vdf comes back byte for byte (read only):
    /// `cargo test real_config -- --ignored`.
    #[test]
    #[ignore]
    fn real_config_round_trip() {
        for root in crate::sources::steam::roots() {
            let f = root.join("config/config.vdf");
            let Ok(src) = std::fs::read_to_string(&f) else {
                continue;
            };
            let out = parse_text(&src).to_text();
            if out != src {
                let at = out
                    .bytes()
                    .zip(src.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(0);
                panic!(
                    "{}: differs at {at}: {:?} vs {:?}",
                    f.display(),
                    &out[at.saturating_sub(40)..(at + 40).min(out.len())],
                    &src[at.saturating_sub(40)..(at + 40).min(src.len())]
                );
            }
        }
    }

    #[test]
    fn binary() {
        let mut b = vec![0x00];
        b.extend(b"root\0");
        b.push(0x01);
        b.extend(b"name\0Game\0");
        b.push(0x02);
        b.extend(b"data\0");
        b.extend(7u32.to_le_bytes());
        b.extend([0x08, 0x08]);
        let kv = parse_binary(&b, None).unwrap();
        assert_eq!(kv.path(&["root", "name"]).and_then(Kv::str), Some("Game"));
        assert_eq!(kv.path(&["root", "data"]).and_then(Kv::int), Some(7));
    }
}
