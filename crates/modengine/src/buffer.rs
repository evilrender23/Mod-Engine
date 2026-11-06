//! GameMaker-style binary buffers (`buffer_create`, `buffer_write`, `buffer_read`...).
//!
//! A buffer is a block of bytes with a read/write position ("seek"). Every value is stored
//! little-endian. Before each read or write the position is padded up to a multiple of the
//! buffer's alignment (1 = no padding), like GameMaker does.
//!
//! Kinds:
//! - `Fixed`: the size never changes; writing past the end is an error.
//! - `Grow`: writing past the end makes the buffer bigger.
//! - `Wrap`: the position wraps around to the start when it passes the end.
//! - `Fast`: like `Fixed` (GameMaker restricts it to u8; here any type works).

use std::collections::HashMap;

pub const MAX_BUFFER_SIZE: usize = 256 * 1024 * 1024;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BufKind {
    Fixed,
    Grow,
    Wrap,
    Fast,
}

impl BufKind {
    pub fn from_num(n: i64) -> Option<BufKind> {
        Some(match n {
            0 => BufKind::Fixed,
            1 => BufKind::Grow,
            2 => BufKind::Wrap,
            3 => BufKind::Fast,
            _ => return None,
        })
    }
    pub fn num(self) -> i64 {
        match self {
            BufKind::Fixed => 0,
            BufKind::Grow => 1,
            BufKind::Wrap => 2,
            BufKind::Fast => 3,
        }
    }
}

/// Data types (the numbers are GameMaker's `buffer_u8`...`buffer_text`).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BufType {
    U8,
    S8,
    U16,
    S16,
    U32,
    S32,
    F16,
    F32,
    F64,
    Bool,
    Str,
    U64,
    Text,
}

impl BufType {
    pub fn from_num(n: i64) -> Option<BufType> {
        use BufType::*;
        Some(match n {
            1 => U8,
            2 => S8,
            3 => U16,
            4 => S16,
            5 => U32,
            6 => S32,
            7 => F16,
            8 => F32,
            9 => F64,
            10 => Bool,
            11 => Str,
            12 => U64,
            13 => Text,
            _ => return None,
        })
    }
    /// Size in bytes; 0 for strings (variable size).
    pub fn size(self) -> usize {
        use BufType::*;
        match self {
            U8 | S8 | Bool => 1,
            U16 | S16 | F16 => 2,
            U32 | S32 | F32 => 4,
            F64 | U64 => 8,
            Str | Text => 0,
        }
    }
    pub const NAMES: &'static [(&'static str, i64)] = &[
        ("buffer_u8", 1),
        ("buffer_s8", 2),
        ("buffer_u16", 3),
        ("buffer_s16", 4),
        ("buffer_u32", 5),
        ("buffer_s32", 6),
        ("buffer_f16", 7),
        ("buffer_f32", 8),
        ("buffer_f64", 9),
        ("buffer_bool", 10),
        ("buffer_string", 11),
        ("buffer_u64", 12),
        ("buffer_text", 13),
    ];
}

/// A value read from / written to a buffer.
#[derive(Clone, Debug, PartialEq)]
pub enum BufVal {
    Num(f64),
    Bool(bool),
    Str(String),
}

#[derive(Clone, Debug)]
pub struct Buffer {
    pub data: Vec<u8>,
    pub kind: BufKind,
    pub align: usize,
    pub pos: usize,
    /// Highest byte ever written + 1 (what `buffer_get_used_size` reports).
    pub used: usize,
}

fn f32_to_f16(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let mut man = x & 0x7f_ffff;
    if exp == 0xff {
        // inf / nan
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        man |= 0x80_0000;
        let shift = (14 - e) as u32;
        let half = man >> shift;
        let round = (man >> (shift - 1)) & 1;
        return sign | (half + round) as u16;
    }
    let half = sign as u32 | ((e as u32) << 10) | (man >> 13);
    let round = (man >> 12) & 1;
    (half + round) as u16
}

fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if man == 0 {
            sign
        } else {
            // subnormal
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x3ff) << 13)
        }
    } else if exp == 0x1f {
        sign | 0x7f80_0000 | (man << 13)
    } else {
        sign | ((exp + 127 - 15) << 23) | (man << 13)
    };
    f32::from_bits(bits)
}

impl Buffer {
    pub fn new(size: usize, kind: BufKind, align: usize) -> Result<Buffer, String> {
        if size > MAX_BUFFER_SIZE {
            return Err(format!("buffer too big ({size} bytes, max {MAX_BUFFER_SIZE})"));
        }
        if kind == BufKind::Wrap && size == 0 {
            return Err("a buffer_wrap needs a size above 0".into());
        }
        Ok(Buffer { data: vec![0; size], kind, align: align.clamp(1, 1024), pos: 0, used: 0 })
    }

    /// A fixed buffer holding exactly `bytes` (received network data, loaded files).
    pub fn from_bytes(bytes: Vec<u8>, kind: BufKind) -> Buffer {
        let used = bytes.len();
        Buffer { data: bytes, kind, align: 1, pos: 0, used }
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }

    fn aligned(&self, pos: usize) -> usize {
        if self.align <= 1 {
            pos
        } else {
            pos.div_ceil(self.align) * self.align
        }
    }

    /// Makes room for `n` bytes at the (aligned) position and returns where they go.
    fn place(&mut self, n: usize) -> Result<usize, String> {
        let mut at = self.aligned(self.pos);
        match self.kind {
            BufKind::Grow => {
                if at + n > self.data.len() {
                    let need = at + n;
                    if need > MAX_BUFFER_SIZE {
                        return Err(format!("buffer_grow would pass {MAX_BUFFER_SIZE} bytes"));
                    }
                    let new = need.max(self.data.len() * 2).min(MAX_BUFFER_SIZE);
                    self.data.resize(new, 0);
                }
            }
            BufKind::Wrap => {
                if n > self.data.len() {
                    return Err(format!("value of {n} bytes does not fit in a buffer_wrap of {} bytes", self.data.len()));
                }
                if at >= self.data.len() || at + n > self.data.len() {
                    at = 0;
                }
            }
            BufKind::Fixed | BufKind::Fast => {
                if at + n > self.data.len() {
                    return Err(format!("buffer full: writing {n} bytes at {at} in a buffer of {} bytes", self.data.len()));
                }
            }
        }
        Ok(at)
    }

    /// Where `n` bytes are read from, or an error past the end.
    fn take(&self, n: usize) -> Result<usize, String> {
        let mut at = self.aligned(self.pos);
        if self.kind == BufKind::Wrap && at + n > self.data.len() && n <= self.data.len() {
            at = 0;
        }
        if at + n > self.data.len() {
            return Err(format!("reading {n} bytes at {at} passes the end of the buffer ({} bytes)", self.data.len()));
        }
        Ok(at)
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let at = self.place(bytes.len())?;
        self.data[at..at + bytes.len()].copy_from_slice(bytes);
        self.pos = at + bytes.len();
        self.used = self.used.max(self.pos);
        if self.kind == BufKind::Wrap && self.pos >= self.data.len() {
            self.pos = 0;
        }
        Ok(())
    }

    fn read_bytes(&mut self, n: usize) -> Result<&[u8], String> {
        let at = self.take(n)?;
        self.pos = at + n;
        if self.kind == BufKind::Wrap && self.pos >= self.data.len() {
            self.pos = 0;
        }
        Ok(&self.data[at..at + n])
    }

    pub fn encode(t: BufType, v: &BufVal) -> Result<Vec<u8>, String> {
        let num = || match v {
            BufVal::Num(n) => Ok(*n),
            BufVal::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            BufVal::Str(s) => s.trim().parse::<f64>().map_err(|_| format!("a number is needed to write {t:?}, got text")),
        };
        // integers wrap like a C cast (GameMaker truncates too); NaN becomes 0
        let int = |n: f64| if n.is_finite() { n.trunc() as i64 } else { 0 };
        Ok(match t {
            BufType::U8 => vec![int(num()?) as u8],
            BufType::S8 => vec![int(num()?) as i8 as u8],
            BufType::U16 => (int(num()?) as u16).to_le_bytes().to_vec(),
            BufType::S16 => (int(num()?) as i16).to_le_bytes().to_vec(),
            BufType::U32 => (int(num()?) as u32).to_le_bytes().to_vec(),
            BufType::S32 => (int(num()?) as i32).to_le_bytes().to_vec(),
            BufType::U64 => {
                let n = num()?;
                (if n.is_finite() && n > 0.0 { n as u64 } else { int(n) as u64 }).to_le_bytes().to_vec()
            }
            BufType::F16 => f32_to_f16(num()? as f32).to_le_bytes().to_vec(),
            BufType::F32 => (num()? as f32).to_le_bytes().to_vec(),
            BufType::F64 => num()?.to_le_bytes().to_vec(),
            BufType::Bool => vec![match v {
                BufVal::Bool(b) => *b as u8,
                BufVal::Num(n) => (*n != 0.0 && !n.is_nan()) as u8,
                BufVal::Str(s) => (!s.is_empty()) as u8,
            }],
            BufType::Str | BufType::Text => {
                let s = match v {
                    BufVal::Str(s) => s.clone(),
                    BufVal::Num(n) => modscript::fmt_num(*n),
                    BufVal::Bool(b) => b.to_string(),
                };
                let mut b = s.into_bytes();
                if t == BufType::Str {
                    // a NUL inside would cut the string short when read back
                    b.retain(|c| *c != 0);
                    b.push(0);
                }
                b
            }
        })
    }

    pub fn write(&mut self, t: BufType, v: &BufVal) -> Result<(), String> {
        let bytes = Self::encode(t, v)?;
        self.write_bytes(&bytes)
    }

    pub fn read(&mut self, t: BufType) -> Result<BufVal, String> {
        let le = |b: &[u8]| {
            let mut a = [0u8; 8];
            a[..b.len()].copy_from_slice(b);
            a
        };
        Ok(match t {
            BufType::U8 => BufVal::Num(self.read_bytes(1)?[0] as f64),
            BufType::S8 => BufVal::Num(self.read_bytes(1)?[0] as i8 as f64),
            BufType::Bool => BufVal::Bool(self.read_bytes(1)?[0] != 0),
            BufType::U16 => BufVal::Num(u16::from_le_bytes(le(self.read_bytes(2)?)[..2].try_into().unwrap()) as f64),
            BufType::S16 => BufVal::Num(i16::from_le_bytes(le(self.read_bytes(2)?)[..2].try_into().unwrap()) as f64),
            BufType::F16 => BufVal::Num(f16_to_f32(u16::from_le_bytes(le(self.read_bytes(2)?)[..2].try_into().unwrap())) as f64),
            BufType::U32 => BufVal::Num(u32::from_le_bytes(le(self.read_bytes(4)?)[..4].try_into().unwrap()) as f64),
            BufType::S32 => BufVal::Num(i32::from_le_bytes(le(self.read_bytes(4)?)[..4].try_into().unwrap()) as f64),
            BufType::F32 => BufVal::Num(f32::from_le_bytes(le(self.read_bytes(4)?)[..4].try_into().unwrap()) as f64),
            BufType::F64 => BufVal::Num(f64::from_le_bytes(le(self.read_bytes(8)?))),
            BufType::U64 => BufVal::Num(u64::from_le_bytes(le(self.read_bytes(8)?)) as f64),
            BufType::Str => {
                let at = self.take(0)?;
                let end = self.data[at..].iter().position(|b| *b == 0);
                let Some(len) = end else {
                    return Err(format!("buffer_string at {at} has no end (no 0 byte before the end of the buffer)"));
                };
                let s = String::from_utf8_lossy(&self.data[at..at + len]).into_owned();
                self.pos = at + len + 1;
                if self.kind == BufKind::Wrap && self.pos >= self.data.len() {
                    self.pos = 0;
                }
                BufVal::Str(s)
            }
            BufType::Text => {
                // the rest of the written data (up to a 0 byte if there is one)
                let at = self.take(0)?;
                let end = self.used.max(at).min(self.data.len());
                let len = self.data[at..end].iter().position(|b| *b == 0).unwrap_or(end - at);
                let s = String::from_utf8_lossy(&self.data[at..at + len]).into_owned();
                self.pos = at + len;
                BufVal::Str(s)
            }
        })
    }

    /// `buffer_peek`: reads at `offset` without moving the position.
    pub fn peek(&mut self, offset: usize, t: BufType) -> Result<BufVal, String> {
        let (pos, align) = (self.pos, self.align);
        self.pos = offset;
        self.align = 1;
        let r = self.read(t);
        self.pos = pos;
        self.align = align;
        r
    }

    /// `buffer_poke`: writes at `offset` without moving the position (never grows).
    pub fn poke(&mut self, offset: usize, t: BufType, v: &BufVal) -> Result<(), String> {
        let bytes = Self::encode(t, v)?;
        if offset + bytes.len() > self.data.len() {
            return Err(format!("buffer_poke of {} bytes at {offset} passes the end ({} bytes)", bytes.len(), self.data.len()));
        }
        self.data[offset..offset + bytes.len()].copy_from_slice(&bytes);
        self.used = self.used.max(offset + bytes.len());
        Ok(())
    }

    /// `buffer_seek(buf, base, offset)`: base 0 = start, 1 = relative, 2 = end.
    pub fn seek(&mut self, base: i64, offset: i64) -> Result<(), String> {
        let from = match base {
            0 => 0,
            1 => self.pos as i64,
            2 => self.data.len() as i64,
            _ => return Err("buffer_seek base must be buffer_seek_start, buffer_seek_relative or buffer_seek_end".into()),
        };
        let len = self.data.len() as i64;
        let p = from + offset;
        self.pos = if self.kind == BufKind::Wrap && len > 0 { p.rem_euclid(len) } else { p.clamp(0, len) } as usize;
        Ok(())
    }

    pub fn resize(&mut self, size: usize) -> Result<(), String> {
        if size > MAX_BUFFER_SIZE {
            return Err(format!("buffer too big ({size} bytes)"));
        }
        self.data.resize(size, 0);
        self.pos = self.pos.min(size);
        self.used = self.used.min(size);
        Ok(())
    }

    /// Bytes `[offset, offset + size)` clamped to the buffer.
    pub fn slice(&self, offset: usize, size: usize) -> &[u8] {
        let a = offset.min(self.data.len());
        let b = offset.saturating_add(size).min(self.data.len());
        &self.data[a..b]
    }
}

/// Every buffer of a running game, by id (ids are numbers in scripts, like in GameMaker).
#[derive(Default)]
pub struct Buffers {
    pub list: HashMap<u32, Buffer>,
    next: u32,
}

impl Buffers {
    pub fn add(&mut self, b: Buffer) -> u32 {
        // id 0 is valid in GameMaker too, but scripts often test `if buf` — start at 1
        self.next += 1;
        while self.list.contains_key(&self.next) || self.next == 0 {
            self.next = self.next.wrapping_add(1);
        }
        self.list.insert(self.next, b);
        self.next
    }
    pub fn get(&self, id: u32) -> Option<&Buffer> {
        self.list.get(&id)
    }
    pub fn get_mut(&mut self, id: u32) -> Option<&mut Buffer> {
        self.list.get_mut(&id)
    }
    pub fn remove(&mut self, id: u32) -> bool {
        self.list.remove(&id).is_some()
    }
    pub fn clear(&mut self) {
        self.list.clear();
    }
}

// ---------------------------------------------------------------- base64

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Decodes standard or URL-safe base64; whitespace is ignored, padding optional.
pub fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in s.bytes() {
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return Err(format!("'{}' is not a base64 character", ch as char)),
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(t: BufType, v: BufVal) -> BufVal {
        let mut b = Buffer::new(1, BufKind::Grow, 1).unwrap();
        b.write(t, &v).unwrap();
        b.write(BufType::U8, &BufVal::Num(7.0)).unwrap();
        b.seek(0, 0).unwrap();
        let r = b.read(t).unwrap();
        // the next value is still in its place
        assert_eq!(b.read(BufType::U8).unwrap(), BufVal::Num(7.0), "{t:?}");
        r
    }

    #[test]
    fn every_type_round_trips() {
        use BufType::*;
        let n = BufVal::Num;
        assert_eq!(rt(U8, n(200.0)), n(200.0));
        assert_eq!(rt(U8, n(300.0)), n(44.0)); // wraps like a C cast
        assert_eq!(rt(S8, n(-100.0)), n(-100.0));
        assert_eq!(rt(U16, n(65000.0)), n(65000.0));
        assert_eq!(rt(S16, n(-32000.0)), n(-32000.0));
        assert_eq!(rt(U32, n(4_000_000_000.0)), n(4_000_000_000.0));
        assert_eq!(rt(S32, n(-2_000_000_000.0)), n(-2_000_000_000.0));
        assert_eq!(rt(U64, n(9_007_199_254_740_992.0)), n(9_007_199_254_740_992.0));
        assert_eq!(rt(F16, n(1.5)), n(1.5));
        assert_eq!(rt(F16, n(-0.25)), n(-0.25));
        assert_eq!(rt(F16, n(65504.0)), n(65504.0));
        assert_eq!(rt(F32, n(3.25)), n(3.25));
        assert_eq!(rt(F64, n(std::f64::consts::PI)), n(std::f64::consts::PI));
        assert_eq!(rt(Bool, BufVal::Bool(true)), BufVal::Bool(true));
        assert_eq!(rt(Bool, n(0.0)), BufVal::Bool(false));
        assert_eq!(rt(Str, BufVal::Str("hola ñandú".into())), BufVal::Str("hola ñandú".into()));
        assert_eq!(rt(Str, BufVal::Str(String::new())), BufVal::Str(String::new()));
        // text has no terminator: it reads up to the end of what was written
        let mut b = Buffer::new(0, BufKind::Grow, 1).unwrap();
        b.write(Text, &BufVal::Str("abc".into())).unwrap();
        b.seek(0, 0).unwrap();
        assert_eq!(b.read(Text).unwrap(), BufVal::Str("abc".into()));
        assert_eq!(b.used, 3);
    }

    #[test]
    fn fixed_buffers_refuse_to_overflow_and_reads_stop_at_the_end() {
        let mut b = Buffer::new(4, BufKind::Fixed, 1).unwrap();
        b.write(BufType::U16, &BufVal::Num(1.0)).unwrap();
        b.write(BufType::U16, &BufVal::Num(2.0)).unwrap();
        assert!(b.write(BufType::U8, &BufVal::Num(3.0)).is_err());
        assert!(b.read(BufType::U8).is_err());
        b.seek(0, 0).unwrap();
        assert!(b.read(BufType::F64).is_err());
        // a string without its 0 byte is an error, not a panic
        let mut s = Buffer::from_bytes(b"abc".to_vec(), BufKind::Fixed);
        assert!(s.read(BufType::Str).is_err());
    }

    #[test]
    fn grow_buffers_grow() {
        let mut b = Buffer::new(1, BufKind::Grow, 1).unwrap();
        for i in 0..100 {
            b.write(BufType::U32, &BufVal::Num(i as f64)).unwrap();
        }
        assert!(b.size() >= 400);
        assert_eq!(b.used, 400);
        b.seek(0, 396).unwrap();
        assert_eq!(b.read(BufType::U32).unwrap(), BufVal::Num(99.0));
    }

    #[test]
    fn wrap_buffers_wrap_around() {
        let mut b = Buffer::new(4, BufKind::Wrap, 1).unwrap();
        for i in 0..6 {
            b.write(BufType::U8, &BufVal::Num(i as f64)).unwrap();
        }
        // 4 and 5 overwrote the first two bytes
        assert_eq!(b.data, vec![4, 5, 2, 3]);
        assert_eq!(b.pos, 2);
        // a u16 that does not fit before the end goes to the start
        b.seek(0, 3).unwrap();
        b.write(BufType::U16, &BufVal::Num(0x0908 as f64)).unwrap();
        assert_eq!(&b.data[..2], &[8, 9]);
        b.seek(1, -1).unwrap();
        assert_eq!(b.pos, 1);
        b.seek(1, -6).unwrap();
        assert_eq!(b.pos, 3);
        assert!(b.write(BufType::F64, &BufVal::Num(1.0)).is_err());
    }

    #[test]
    fn alignment_pads_before_each_value() {
        let mut b = Buffer::new(16, BufKind::Fixed, 4).unwrap();
        b.write(BufType::U8, &BufVal::Num(1.0)).unwrap();
        b.write(BufType::U8, &BufVal::Num(2.0)).unwrap();
        assert_eq!(b.pos, 5);
        assert_eq!(&b.data[..5], &[1, 0, 0, 0, 2]);
        b.seek(0, 0).unwrap();
        assert_eq!(b.read(BufType::U8).unwrap(), BufVal::Num(1.0));
        assert_eq!(b.read(BufType::U8).unwrap(), BufVal::Num(2.0));
    }

    #[test]
    fn seek_tell_peek_poke() {
        let mut b = Buffer::new(8, BufKind::Fixed, 1).unwrap();
        b.poke(4, BufType::U16, &BufVal::Num(513.0)).unwrap();
        assert_eq!(b.pos, 0);
        assert_eq!(b.peek(4, BufType::U16).unwrap(), BufVal::Num(513.0));
        assert_eq!(b.peek(4, BufType::U8).unwrap(), BufVal::Num(1.0));
        b.seek(2, -2).unwrap();
        assert_eq!(b.pos, 6);
        b.seek(1, 100).unwrap();
        assert_eq!(b.pos, 8);
        b.seek(0, -5).unwrap();
        assert_eq!(b.pos, 0);
        assert!(b.poke(7, BufType::U16, &BufVal::Num(1.0)).is_err());
        assert!(b.seek(9, 0).is_err());
    }

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7]] {
            let s = base64_encode(data);
            assert_eq!(base64_decode(&s).unwrap(), data);
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert!(base64_decode("a$b").is_err());
    }

    #[test]
    fn half_floats() {
        for v in [0.0f32, 1.0, -2.0, 0.5, 1024.0, 6.1035156e-5, 5.9604645e-8] {
            assert_eq!(f16_to_f32(f32_to_f16(v)), v, "{v}");
        }
        assert!(f16_to_f32(f32_to_f16(1e9)).is_infinite());
        assert!(f16_to_f32(f32_to_f16(f32::NAN)).is_nan());
    }
}
