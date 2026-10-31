//! Port of `src/engine/nbt.mjs` — minimal NBT codec for Minecraft `servers.dat`.
//!
//! Modern format (1.21.2+): root TAG_Compound containing a TAG_List named
//! `servers` of compounds `{ hidden: byte, ip: string, name: string, icon:
//! string, acceptTextures: byte }`. Legacy (<1.21.2) payloads may be prefixed
//! by a version byte 0x02/0x03 and carry only `{ name, ip, icon? }`.
//!
//! [`write_servers_dat`] always emits the modern format; `ServerEntry` carries
//! no icon bytes (only `has_icon`), so a written icon is always `""`.
//!
//! Reader quirks are reproduced from the JS module on purpose (they are
//! pinned by tests/nbt.test.mjs): strings whose declared length runs past EOF
//! decode clamped; a compound may end at EOF without TAG_End; negative list
//! counts decode as empty; TAG_Long values never count as "1" for flags.

use indexmap::IndexMap;

use crate::types::ServerEntry;

pub const TAG_END: u8 = 0x00;
pub const TAG_BYTE: u8 = 0x01;
pub const TAG_SHORT: u8 = 0x02;
pub const TAG_INT: u8 = 0x03;
pub const TAG_LONG: u8 = 0x04;
pub const TAG_FLOAT: u8 = 0x05;
pub const TAG_DOUBLE: u8 = 0x06;
pub const TAG_BYTE_ARRAY: u8 = 0x07;
pub const TAG_STRING: u8 = 0x08;
pub const TAG_LIST: u8 = 0x09;
pub const TAG_COMPOUND: u8 = 0x0a;
pub const TAG_INT_ARRAY: u8 = 0x0b;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum NbtError {
    /// JS `RangeError [ERR_OUT_OF_RANGE]`: a read or write past a bound.
    #[error("NBT read/write out of range")]
    OutOfRange,
    /// `unsupported NBT tag 0x… at offset …`
    #[error("{0}")]
    Unsupported(String),
    /// `not a servers.dat NBT payload (first byte …)`
    #[error("{0}")]
    NotServersDat(String),
}

impl From<NbtError> for crate::EngineError {
    fn from(e: NbtError) -> Self {
        crate::EngineError::Other(e.to_string())
    }
}

/// A decoded NBT value.
#[derive(Debug, Clone, PartialEq)]
pub enum NbtValue {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    List(Vec<NbtValue>),
    Compound(IndexMap<String, NbtValue>),
    ByteArray(Vec<u8>),
    IntArray(Vec<i32>),
}

impl NbtValue {
    /// JS `v === 1` (numbers only; a BigInt long is never `=== 1`).
    fn is_js_one(&self) -> bool {
        match self {
            NbtValue::Byte(v) => *v == 1,
            NbtValue::Short(v) => *v == 1,
            NbtValue::Int(v) => *v == 1,
            NbtValue::Float(v) => *v == 1.0,
            NbtValue::Double(v) => *v == 1.0,
            _ => false,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            NbtValue::String(s) => Some(s),
            _ => None,
        }
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    off: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], NbtError> {
        let end = self.off.checked_add(n).ok_or(NbtError::OutOfRange)?;
        let s = self.buf.get(self.off..end).ok_or(NbtError::OutOfRange)?;
        self.off = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, NbtError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Result<i32, NbtError> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    /// Length-prefixed UTF-8, clamped at EOF (Buffer#toString semantics).
    fn string(&mut self) -> Result<String, NbtError> {
        let len = self.u16()? as usize;
        let start = self.off.min(self.buf.len());
        let end = (self.off + len).min(self.buf.len());
        let s = String::from_utf8_lossy(&self.buf[start..end]).into_owned();
        self.off += len;
        Ok(s)
    }

    fn compound(&mut self) -> Result<IndexMap<String, NbtValue>, NbtError> {
        let mut out = IndexMap::new();
        while self.off < self.buf.len() {
            let ty = self.u8()?;
            if ty == TAG_END {
                break;
            }
            let name = self.string()?;
            let v = self.value(ty)?;
            out.insert(name, v);
        }
        Ok(out)
    }

    fn value(&mut self, ty: u8) -> Result<NbtValue, NbtError> {
        Ok(match ty {
            TAG_BYTE => NbtValue::Byte(self.u8()? as i8),
            TAG_SHORT => {
                let b = self.take(2)?;
                NbtValue::Short(i16::from_be_bytes([b[0], b[1]]))
            }
            TAG_INT => NbtValue::Int(self.i32()?),
            TAG_LONG => {
                let b = self.take(8)?;
                NbtValue::Long(i64::from_be_bytes(b.try_into().expect("8 bytes")))
            }
            TAG_FLOAT => {
                let b = self.take(4)?;
                NbtValue::Float(f32::from_be_bytes(b.try_into().expect("4 bytes")))
            }
            TAG_DOUBLE => {
                let b = self.take(8)?;
                NbtValue::Double(f64::from_be_bytes(b.try_into().expect("8 bytes")))
            }
            TAG_STRING => NbtValue::String(self.string()?),
            TAG_LIST => {
                let elem = self.u8()?;
                let count = self.i32()?;
                let mut arr = Vec::new();
                for _ in 0..count.max(0) {
                    arr.push(self.value(elem)?);
                }
                NbtValue::List(arr)
            }
            TAG_COMPOUND => NbtValue::Compound(self.compound()?),
            TAG_BYTE_ARRAY => {
                let n = self.i32()?.max(0) as usize;
                let start = self.off.min(self.buf.len());
                let end = (self.off + n).min(self.buf.len());
                let v = self.buf[start..end].to_vec();
                self.off += n;
                NbtValue::ByteArray(v)
            }
            TAG_INT_ARRAY => {
                let n = self.i32()?.max(0);
                let mut v = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    v.push(self.i32()?);
                }
                NbtValue::IntArray(v)
            }
            other => {
                return Err(NbtError::Unsupported(format!("unsupported NBT tag 0x{other:x} at offset {}", self.off)))
            }
        })
    }
}

/// Parse a servers.dat payload into ServerEntry[] (legacy prefix detected;
/// missing modern fields default).
pub fn parse_servers_dat(buf: &[u8]) -> Result<Vec<ServerEntry>, NbtError> {
    let mut offset = 0;
    if !buf.is_empty() && (buf[0] == 0x02 || buf[0] == 0x03) {
        offset = 1;
    }
    if buf.len() <= offset || buf[offset] != TAG_COMPOUND {
        let seen = if buf.len() > offset { format!("0x{:x}", buf[offset]) } else { "end-of-buffer".into() };
        return Err(NbtError::NotServersDat(format!("not a servers.dat NBT payload (first byte {seen})")));
    }
    let mut r = Reader { buf, off: offset + 1 };
    r.string()?; // root name ("" in servers.dat)
    let root = r.compound()?;
    let list = match root.get("servers") {
        Some(NbtValue::List(l)) => l.as_slice(),
        _ => &[],
    };
    Ok(list.iter().map(entry_from_value).collect())
}

fn entry_from_value(v: &NbtValue) -> ServerEntry {
    let empty = IndexMap::new();
    let c = match v {
        NbtValue::Compound(c) => c,
        _ => &empty,
    };
    let s = |k: &str| c.get(k).and_then(NbtValue::as_str).unwrap_or("").to_string();
    let one = |k: &str| c.get(k).map(NbtValue::is_js_one).unwrap_or(false);
    let icon = s("icon");
    ServerEntry {
        name: s("name"),
        ip: s("ip"),
        hidden: one("hidden"),
        accept_textures: one("acceptTextures"),
        has_icon: !icon.is_empty(),
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) -> Result<(), NbtError> {
    let len = u16::try_from(s.len()).map_err(|_| NbtError::OutOfRange)?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

/// Serialize ServerEntry[] into a modern-format servers.dat payload.
/// Fails with [`NbtError::OutOfRange`] for a string over 65535 UTF-8 bytes.
pub fn write_servers_dat(entries: &[ServerEntry]) -> Result<Vec<u8>, NbtError> {
    let mut out = vec![TAG_COMPOUND];
    out.extend_from_slice(&0u16.to_be_bytes());
    out.push(TAG_LIST);
    put_str(&mut out, "servers")?;
    out.push(TAG_COMPOUND);
    let count = i32::try_from(entries.len()).map_err(|_| NbtError::OutOfRange)?;
    out.extend_from_slice(&count.to_be_bytes());
    for e in entries {
        out.push(TAG_BYTE);
        put_str(&mut out, "hidden")?;
        out.push(u8::from(e.hidden));
        out.push(TAG_STRING);
        put_str(&mut out, "ip")?;
        put_str(&mut out, &e.ip)?;
        out.push(TAG_STRING);
        put_str(&mut out, "name")?;
        put_str(&mut out, &e.name)?;
        out.push(TAG_STRING);
        put_str(&mut out, "icon")?;
        put_str(&mut out, "")?;
        out.push(TAG_BYTE);
        put_str(&mut out, "acceptTextures")?;
        out.push(u8::from(e.accept_textures));
        out.push(TAG_END);
    }
    out.push(TAG_END);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- wire helpers (mirror tests/nbt.test.mjs) ----
    fn nstr(s: &str) -> Vec<u8> {
        let mut v = (s.len() as u16).to_be_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v
    }
    fn i32b(n: i32) -> Vec<u8> {
        n.to_be_bytes().to_vec()
    }
    fn compound(pairs: Vec<(u8, &str, Vec<u8>)>) -> Vec<u8> {
        let mut v = vec![];
        for (t, n, b) in pairs {
            v.push(t);
            v.extend(nstr(n));
            v.extend(b);
        }
        v.push(TAG_END);
        v
    }
    fn payload(root_name: &str, legacy: u8, servers: Vec<Vec<u8>>) -> Vec<u8> {
        let mut v = vec![];
        if legacy != 0 {
            v.push(legacy);
        }
        v.push(TAG_COMPOUND);
        v.extend(nstr(root_name));
        v.push(TAG_LIST);
        v.extend(nstr("servers"));
        v.push(TAG_COMPOUND);
        v.extend(i32b(servers.len() as i32));
        for s in servers {
            v.extend(s);
        }
        v.push(TAG_END);
        v
    }
    fn se(name: &str, ip: &str, hidden: bool, accept: bool) -> ServerEntry {
        ServerEntry { name: name.into(), ip: ip.into(), hidden, accept_textures: accept, has_icon: false }
    }
    fn default_entry() -> ServerEntry {
        se("", "", false, false)
    }

    #[test]
    fn golden_bytes() {
        assert_eq!(hex::encode(write_servers_dat(&[]).unwrap()), "0a0000090007736572766572730a0000000000");
        assert_eq!(
            hex::encode(write_servers_dat(&[se("Test", "localhost", false, false)]).unwrap()),
            concat!(
                "0a0000090007736572766572730a00000001",
                "01000668696464656e00",
                "080002697000096c6f63616c686f7374",
                "0800046e616d65000454657374",
                "08000469636f6e0000",
                "01000e616363657074546578747572657300",
                "00",
                "00"
            )
        );
    }

    #[test]
    fn round_trips() {
        assert!(parse_servers_dat(&write_servers_dat(&[]).unwrap()).unwrap().is_empty());
        let input = vec![se("Test", "localhost", true, true)];
        assert_eq!(parse_servers_dat(&write_servers_dat(&input).unwrap()).unwrap(), input);
        let input = vec![
            se("español 😀", "mc.example.com", true, false),
            se("", "", false, true),
            se(&"A".repeat(60000), "x", true, true),
        ];
        assert_eq!(parse_servers_dat(&write_servers_dat(&input).unwrap()).unwrap(), input);
        let mut with_icon = se("x", "y", false, false);
        with_icon.has_icon = true;
        assert_eq!(
            parse_servers_dat(&write_servers_dat(&[with_icon]).unwrap()).unwrap(),
            vec![se("x", "y", false, false)]
        );
        assert_eq!(write_servers_dat(&[se(&"x".repeat(70000), "", false, false)]), Err(NbtError::OutOfRange));
    }

    #[test]
    fn every_supported_type_decodes() {
        let servers = vec![compound(vec![
            (TAG_BYTE, "b", vec![0x80]),
            (TAG_BYTE, "B", vec![0x7f]),
            (TAG_SHORT, "s", (-32768i16).to_be_bytes().to_vec()),
            (TAG_INT, "i", i32b(i32::MIN)),
            (TAG_LONG, "l", i64::MIN.to_be_bytes().to_vec()),
            (TAG_FLOAT, "f", 0.5f32.to_be_bytes().to_vec()),
            (TAG_DOUBLE, "d", std::f64::consts::PI.to_be_bytes().to_vec()),
            (TAG_STRING, "emptyStr", nstr("")),
            (TAG_LIST, "emptyList", [vec![TAG_COMPOUND], i32b(0)].concat()),
            (TAG_LIST, "intList", [vec![TAG_INT], i32b(2), i32b(1), i32b(-2)].concat()),
            (TAG_COMPOUND, "nested", compound(vec![(TAG_STRING, "inner", nstr("x"))])),
            (TAG_BYTE_ARRAY, "ba", [i32b(3), vec![0, 127, 255]].concat()),
            (TAG_INT_ARRAY, "ia", [i32b(2), i32b(i32::MIN), i32b(i32::MAX)].concat()),
            (TAG_STRING, "name", nstr("ABC")),
            (TAG_BYTE, "hidden", vec![1]),
        ])];
        assert_eq!(parse_servers_dat(&payload("", 0, servers)).unwrap(), vec![se("ABC", "", true, false)]);
    }

    #[test]
    fn long_array_is_unsupported() {
        let buf = payload("", 0, vec![compound(vec![(0x0c, "la", vec![])])]);
        let e = parse_servers_dat(&buf).unwrap_err().to_string();
        assert!(e.starts_with("unsupported NBT tag 0xc at offset "), "{e}");
    }

    #[test]
    fn hidden_flag_follows_js_strict_equality() {
        let mk = |t: u8, b: Vec<u8>| payload("", 0, vec![compound(vec![(t, "hidden", b)])]);
        let hidden = |v: bool| vec![ServerEntry { hidden: v, ..default_entry() }];
        assert_eq!(parse_servers_dat(&mk(TAG_BYTE, vec![1])).unwrap(), hidden(true));
        assert_eq!(parse_servers_dat(&mk(TAG_SHORT, 1i16.to_be_bytes().to_vec())).unwrap(), hidden(true));
        assert_eq!(parse_servers_dat(&mk(TAG_INT, i32b(1))).unwrap(), hidden(true));
        assert_eq!(parse_servers_dat(&mk(TAG_FLOAT, 1f32.to_be_bytes().to_vec())).unwrap(), hidden(true));
        assert_eq!(parse_servers_dat(&mk(TAG_DOUBLE, 1f64.to_be_bytes().to_vec())).unwrap(), hidden(true));
        assert_eq!(parse_servers_dat(&mk(TAG_LONG, 1i64.to_be_bytes().to_vec())).unwrap(), hidden(false));
        assert_eq!(parse_servers_dat(&mk(TAG_BYTE, vec![2])).unwrap(), hidden(false));
        assert_eq!(parse_servers_dat(&mk(TAG_BYTE, vec![0xff])).unwrap(), hidden(false));
        assert_eq!(parse_servers_dat(&mk(TAG_FLOAT, f32::NAN.to_be_bytes().to_vec())).unwrap(), hidden(false));
    }

    #[test]
    fn root_naming_and_legacy_prefix() {
        let s = || vec![compound(vec![(TAG_STRING, "name", nstr("A")), (TAG_STRING, "ip", nstr("b"))])];
        assert_eq!(
            parse_servers_dat(&payload("hello", 0, s())).unwrap(),
            parse_servers_dat(&payload("", 0, s())).unwrap()
        );
        assert!(parse_servers_dat(&payload("servers", 0, vec![])).unwrap().is_empty());
        let base = write_servers_dat(&[se("A", "b", true, true)]).unwrap();
        let modern = parse_servers_dat(&base).unwrap();
        assert_eq!(parse_servers_dat(&[vec![0x02], base.clone()].concat()).unwrap(), modern);
        assert_eq!(parse_servers_dat(&[vec![0x03], base].concat()).unwrap(), modern);
        let legacy = payload(
            "",
            0x02,
            vec![compound(vec![(TAG_STRING, "name", nstr("Old")), (TAG_STRING, "ip", nstr("1.2.3.4"))])],
        );
        assert_eq!(parse_servers_dat(&legacy).unwrap(), vec![se("Old", "1.2.3.4", false, false)]);
        let icon = payload(
            "",
            0x03,
            vec![compound(vec![
                (TAG_STRING, "name", nstr("N")),
                (TAG_STRING, "icon", nstr("data:image/png;base64,AAAA")),
            ])],
        );
        assert_eq!(
            parse_servers_dat(&icon).unwrap(),
            vec![ServerEntry { has_icon: true, ..se("N", "", false, false) }]
        );
    }

    #[test]
    fn truncated_and_garbage() {
        let msg = |b: &[u8]| parse_servers_dat(b).unwrap_err().to_string();
        assert_eq!(msg(&[]), "not a servers.dat NBT payload (first byte end-of-buffer)");
        assert_eq!(msg(&[0x7f]), "not a servers.dat NBT payload (first byte 0x7f)");
        assert!(msg(&[0x09, 0, 0]).contains("first byte 0x9"));
        assert!(msg(&[0x02, 0x7f]).contains("first byte 0x7f"));
        assert!(msg(&[0x02]).contains("first byte end-of-buffer"));
        assert!(parse_servers_dat(&[0x0a, 0, 0]).unwrap().is_empty());
        let full = write_servers_dat(&[]).unwrap();
        assert!(parse_servers_dat(&full[..full.len() - 1]).unwrap().is_empty());
        let full = write_servers_dat(&[se("Ok", "h", true, true)]).unwrap();
        assert_eq!(parse_servers_dat(&full[..full.len() - 2]).unwrap(), vec![se("Ok", "h", true, true)]);
        assert_eq!(parse_servers_dat(&[0x0a, 0, 0, 0x01]), Err(NbtError::OutOfRange));
        let head = |n: i32| {
            [
                vec![TAG_COMPOUND],
                0u16.to_be_bytes().to_vec(),
                vec![TAG_LIST],
                nstr("servers"),
                vec![TAG_COMPOUND],
                i32b(n),
            ]
            .concat()
        };
        let cut = [head(1), vec![TAG_STRING], 4u16.to_be_bytes().to_vec(), b"na".to_vec()].concat();
        assert_eq!(parse_servers_dat(&cut), Err(NbtError::OutOfRange));
        let partial = [head(1), vec![TAG_STRING], nstr("name"), 4u16.to_be_bytes().to_vec(), b"Te".to_vec()].concat();
        assert_eq!(parse_servers_dat(&partial).unwrap(), vec![se("Te", "", false, false)]);
        let overrun = [head(1), vec![TAG_STRING], nstr("name"), 60000u16.to_be_bytes().to_vec()].concat();
        assert_eq!(parse_servers_dat(&overrun).unwrap(), vec![default_entry()]);
        let negative = [head(-5), vec![TAG_END]].concat();
        assert!(parse_servers_dat(&negative).unwrap().is_empty());
        let end_elems = [
            vec![TAG_COMPOUND],
            0u16.to_be_bytes().to_vec(),
            vec![TAG_LIST],
            nstr("servers"),
            vec![TAG_END],
            i32b(1),
            vec![TAG_END],
        ]
        .concat();
        assert!(msg(&end_elems).starts_with("unsupported NBT tag 0x0 at offset "));
        let not_list = [
            vec![TAG_COMPOUND],
            0u16.to_be_bytes().to_vec(),
            vec![TAG_COMPOUND],
            nstr("servers"),
            vec![TAG_END],
            vec![TAG_END],
        ]
        .concat();
        assert!(parse_servers_dat(&not_list).unwrap().is_empty());
        let ints = [
            vec![TAG_COMPOUND],
            0u16.to_be_bytes().to_vec(),
            vec![TAG_LIST],
            nstr("servers"),
            vec![TAG_INT],
            i32b(2),
            i32b(1),
            i32b(2),
            vec![TAG_END],
        ]
        .concat();
        assert_eq!(parse_servers_dat(&ints).unwrap(), vec![default_entry(), default_entry()]);
    }
}
