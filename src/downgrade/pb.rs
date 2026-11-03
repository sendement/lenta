//! Just enough protobuf for Steam's messages and depot manifests: a writer
//! for the few requests sent, and a reader that walks fields.

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

fn varint(buf: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            buf.push(b);
            return;
        }
        buf.push(b | 0x80);
    }
}

impl Writer {
    fn key(&mut self, field: u32, wire: u8) {
        varint(&mut self.buf, ((field as u64) << 3) | wire as u64);
    }
    pub fn uint(&mut self, field: u32, v: u64) -> &mut Self {
        self.key(field, 0);
        varint(&mut self.buf, v);
        self
    }
    pub fn int(&mut self, field: u32, v: i64) -> &mut Self {
        self.uint(field, v as u64)
    }
    pub fn bool(&mut self, field: u32, v: bool) -> &mut Self {
        self.uint(field, v as u64)
    }
    pub fn fixed64(&mut self, field: u32, v: u64) -> &mut Self {
        self.key(field, 1);
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn fixed32(&mut self, field: u32, v: u32) -> &mut Self {
        self.key(field, 5);
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn bytes(&mut self, field: u32, v: &[u8]) -> &mut Self {
        self.key(field, 2);
        varint(&mut self.buf, v.len() as u64);
        self.buf.extend_from_slice(v);
        self
    }
    pub fn str(&mut self, field: u32, v: &str) -> &mut Self {
        self.bytes(field, v.as_bytes())
    }
    pub fn done(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

/// A field's value as read.
#[derive(Clone, Copy, Debug)]
pub enum Val<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

impl<'a> Val<'a> {
    pub fn int(self) -> u64 {
        match self {
            Val::Int(v) => v,
            Val::Bytes(_) => 0,
        }
    }
    pub fn bytes(self) -> &'a [u8] {
        match self {
            Val::Bytes(b) => b,
            Val::Int(_) => &[],
        }
    }
    pub fn str(self) -> String {
        String::from_utf8_lossy(self.bytes()).into_owned()
    }
}

/// (field number, value) of every field in a message; stops at the first
/// malformed byte.
pub fn fields(buf: &[u8]) -> impl Iterator<Item = (u32, Val<'_>)> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let read_varint = |at: &mut usize| -> Option<u64> {
            let mut v = 0u64;
            let mut shift = 0;
            loop {
                let b = *buf.get(*at)?;
                *at += 1;
                v |= ((b & 0x7f) as u64) << shift;
                if b & 0x80 == 0 {
                    return Some(v);
                }
                shift += 7;
                if shift > 63 {
                    return None;
                }
            }
        };
        if at >= buf.len() {
            return None;
        }
        let key = read_varint(&mut at)?;
        let (field, wire) = ((key >> 3) as u32, key & 7);
        let val = match wire {
            0 => Val::Int(read_varint(&mut at)?),
            1 => {
                let s = buf.get(at..at + 8)?;
                at += 8;
                Val::Int(u64::from_le_bytes(s.try_into().ok()?))
            }
            2 => {
                let n = read_varint(&mut at)? as usize;
                let s = buf.get(at..at.checked_add(n)?)?;
                at += n;
                Val::Bytes(s)
            }
            5 => {
                let s = buf.get(at..at + 4)?;
                at += 4;
                Val::Int(u32::from_le_bytes(s.try_into().ok()?) as u64)
            }
            _ => return None,
        };
        Some((field, val))
    })
}

/// The first value of `field`.
pub fn get(buf: &[u8], field: u32) -> Option<Val<'_>> {
    fields(buf).find(|(f, _)| *f == field).map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut w = Writer::default();
        w.uint(1, 300)
            .fixed64(2, 76561197960265728)
            .str(3, "hé")
            .bool(4, true);
        let b = w.done();
        let got: Vec<(u32, u64)> = fields(&b)
            .map(|(f, v)| {
                (
                    f,
                    if f == 3 {
                        v.bytes().len() as u64
                    } else {
                        v.int()
                    },
                )
            })
            .collect();
        assert_eq!(got, vec![(1, 300), (2, 76561197960265728), (3, 3), (4, 1)]);
        assert_eq!(get(&b, 3).unwrap().str(), "hé");
    }
}
