//! Theme Protocol v1 — the wire format between the desktop and the watch.
//!
//! The normative description is `protocol/THEME_PROTOCOL.md`; this module is the Rust
//! reference implementation (the C one is `watch/common/theme_proto.c`).
//!
//! ```text
//! ThemeState (desktop -> watch, written to the Theme State characteristic)
//!
//!  off  size  field
//!    0   2    magic       0x54 0x48  ("TH")
//!    2   1    version     1
//!    3   1    flags       bit0 = light mode, others reserved (must be 0)
//!    4   1    n_colors    number of RGB888 slots that follow (v1 senders: 14)
//!    5   3n   colors      slot order = `palette::Role` discriminant, append-only
//!  5+3n ...   tlv*        [tag u8][len u8][value]  tag 1 = theme name (UTF-8, <= 32 B)
//!  end-2 2    crc16       CRC-16/CCITT-FALSE over bytes [0, end-2), little-endian
//! ```
//!
//! Forward compatibility inside v1: a receiver applies `min(n_colors, slots it knows)`
//! colours and skips TLV tags it does not know. Extra slots and tags therefore never need
//! a version bump; only a layout change does.

use std::fmt;

use crate::palette::{Mode, Rgb, Role, WatchPalette};

pub const MAGIC_THEME: [u8; 2] = [0x54, 0x48]; // "TH"
pub const MAGIC_CONTROL: [u8; 2] = [0x54, 0x43]; // "TC"
pub const VERSION: u8 = 1;
pub const FLAG_LIGHT: u8 = 0x01;
pub const TLV_NAME: u8 = 0x01;
pub const HEADER_LEN: usize = 5;
pub const CRC_LEN: usize = 2;
/// Hard cap the watch enforces on a single ThemeState write (fits any sane MTU/long write).
pub const MAX_PACKET_LEN: usize = 240;

/// CRC-16/CCITT-FALSE: poly 0x1021, init 0xFFFF, no reflection, no final xor.
/// (`crc16("123456789") == 0x29B1`.) Chosen because it is a five-line loop in any language.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// Serialize a watch palette as a v1 ThemeState packet.
pub fn encode_theme(p: &WatchPalette) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + 3 * Role::COUNT + 2 + 32 + CRC_LEN);
    out.extend_from_slice(&MAGIC_THEME);
    out.push(VERSION);
    out.push(if p.mode == Mode::Light { FLAG_LIGHT } else { 0 });
    out.push(Role::COUNT as u8);
    for role in Role::ALL {
        let c = p.get(role);
        out.extend_from_slice(&[c.r, c.g, c.b]);
    }
    let name = WatchPalette::clamp_name(&p.name);
    if !name.is_empty() {
        out.push(TLV_NAME);
        out.push(name.len() as u8);
        out.extend_from_slice(name.as_bytes());
    }
    let crc = crc16(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    debug_assert!(out.len() <= MAX_PACKET_LEN);
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    BadMagic,
    UnsupportedVersion(u8),
    BadCrc { expected: u16, actual: u16 },
    BadTlv,
    TooLong(usize),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "packet truncated"),
            DecodeError::BadMagic => write!(f, "bad magic (not a ThemeState packet)"),
            DecodeError::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            DecodeError::BadCrc { expected, actual } => {
                write!(f, "crc mismatch: packet says {expected:#06x}, computed {actual:#06x}")
            }
            DecodeError::BadTlv => write!(f, "malformed TLV extension"),
            DecodeError::TooLong(n) => write!(f, "packet too long ({n} bytes)"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// What a receiver gets out of a packet, before it decides what to do with unknown slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedTheme {
    pub version: u8,
    pub mode: Mode,
    /// Every colour slot the sender put on the wire, known or not.
    pub colors: Vec<Rgb>,
    pub name: Option<String>,
    pub unknown_tlvs: Vec<(u8, Vec<u8>)>,
    pub crc: u16,
}

impl DecodedTheme {
    /// The v1 receiver's view: known slots filled, unknown slots (if the sender is older
    /// than us) taken from `defaults`.
    pub fn into_palette(self, defaults: &WatchPalette) -> WatchPalette {
        let mut p = defaults.clone();
        p.mode = self.mode;
        p.name = self.name.unwrap_or_default();
        for (i, c) in self.colors.iter().enumerate().take(Role::COUNT) {
            p.colors[i] = *c;
        }
        p
    }
}

/// Parse and validate a ThemeState packet. Mirrors `theme_proto_decode()` in C exactly.
pub fn decode_theme(bytes: &[u8]) -> Result<DecodedTheme, DecodeError> {
    if bytes.len() > MAX_PACKET_LEN {
        return Err(DecodeError::TooLong(bytes.len()));
    }
    if bytes.len() < HEADER_LEN + CRC_LEN {
        return Err(DecodeError::Truncated);
    }
    if bytes[0..2] != MAGIC_THEME {
        return Err(DecodeError::BadMagic);
    }
    let version = bytes[2];
    if version != VERSION {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    let body_end = bytes.len() - CRC_LEN;
    let expected = u16::from_le_bytes([bytes[body_end], bytes[body_end + 1]]);
    let actual = crc16(&bytes[..body_end]);
    if expected != actual {
        return Err(DecodeError::BadCrc { expected, actual });
    }
    let flags = bytes[3];
    let n = bytes[4] as usize;
    let colors_end = HEADER_LEN + 3 * n;
    if colors_end > body_end {
        return Err(DecodeError::Truncated);
    }
    let colors = bytes[HEADER_LEN..colors_end]
        .chunks_exact(3)
        .map(|c| Rgb::new(c[0], c[1], c[2]))
        .collect();

    let mut name = None;
    let mut unknown_tlvs = Vec::new();
    let mut i = colors_end;
    while i < body_end {
        if i + 2 > body_end {
            return Err(DecodeError::BadTlv);
        }
        let tag = bytes[i];
        let len = bytes[i + 1] as usize;
        let start = i + 2;
        let end = start + len;
        if end > body_end {
            return Err(DecodeError::BadTlv);
        }
        let value = &bytes[start..end];
        match tag {
            TLV_NAME => name = Some(String::from_utf8_lossy(value).into_owned()),
            _ => unknown_tlvs.push((tag, value.to_vec())),
        }
        i = end;
    }

    Ok(DecodedTheme {
        version,
        mode: if flags & FLAG_LIGHT != 0 { Mode::Light } else { Mode::Dark },
        colors,
        name,
        unknown_tlvs,
        crc: expected,
    })
}

/// Watch -> desktop requests (notified on the Control characteristic). Tiny and CRC-less:
/// the link layer already protects notifications and they always fit in one ATT PDU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    NextTheme,
    PrevTheme,
    ToggleMode,
    /// "Send me the current theme again" (e.g. after the watch rebooted with nothing stored).
    Resend,
}

impl Control {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn opcode(self) -> u8 {
        match self {
            Control::NextTheme => 0x01,
            Control::PrevTheme => 0x02,
            Control::ToggleMode => 0x03,
            Control::Resend => 0x04,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn encode(self) -> [u8; 4] {
        [MAGIC_CONTROL[0], MAGIC_CONTROL[1], VERSION, self.opcode()]
    }

    pub fn decode(bytes: &[u8]) -> Result<Control, DecodeError> {
        if bytes.len() < 4 {
            return Err(DecodeError::Truncated);
        }
        if bytes[0..2] != MAGIC_CONTROL {
            return Err(DecodeError::BadMagic);
        }
        if bytes[2] != VERSION {
            return Err(DecodeError::UnsupportedVersion(bytes[2]));
        }
        match bytes[3] {
            0x01 => Ok(Control::NextTheme),
            0x02 => Ok(Control::PrevTheme),
            0x03 => Ok(Control::ToggleMode),
            0x04 => Ok(Control::Resend),
            _ => Err(DecodeError::BadTlv),
        }
    }
}

/// Result code the watch reports after a ThemeState write. Same numbering as C.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StatusCode {
    Ok = 0,
    BadMagic = 1,
    BadVersion = 2,
    BadCrc = 3,
    Truncated = 4,
    BadTlv = 5,
    /// Nothing has been applied since boot and nothing was persisted.
    NoTheme = 6,
    Unknown = 0xFF,
}

impl From<u8> for StatusCode {
    fn from(v: u8) -> Self {
        match v {
            0 => StatusCode::Ok,
            1 => StatusCode::BadMagic,
            2 => StatusCode::BadVersion,
            3 => StatusCode::BadCrc,
            4 => StatusCode::Truncated,
            5 => StatusCode::BadTlv,
            6 => StatusCode::NoTheme,
            _ => StatusCode::Unknown,
        }
    }
}

/// The 6-byte Status characteristic: `[ver][result][crc lo][crc hi][n_applied][mode]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub version: u8,
    pub result: StatusCode,
    /// CRC of the last packet the watch *applied* — the acknowledgement token.
    pub applied_crc: u16,
    pub n_applied: u8,
    pub mode: Mode,
}

impl Status {
    pub const LEN: usize = 6;

    pub fn decode(bytes: &[u8]) -> Result<Status, DecodeError> {
        if bytes.len() < Status::LEN {
            return Err(DecodeError::Truncated);
        }
        Ok(Status {
            version: bytes[0],
            result: StatusCode::from(bytes[1]),
            applied_crc: u16::from_le_bytes([bytes[2], bytes[3]]),
            n_applied: bytes[4],
            mode: if bytes[5] & FLAG_LIGHT != 0 { Mode::Light } else { Mode::Dark },
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn encode(&self) -> [u8; Status::LEN] {
        let crc = self.applied_crc.to_le_bytes();
        [
            self.version,
            self.result as u8,
            crc[0],
            crc[1],
            self.n_applied,
            if self.mode == Mode::Light { FLAG_LIGHT } else { 0 },
        ]
    }
}

/// The 4-byte Info characteristic: `[proto_min][proto_max][max_colors][features]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub proto_min: u8,
    pub proto_max: u8,
    pub max_colors: u8,
    pub features: u8,
}

impl Info {
    pub const LEN: usize = 4;
    #[allow(dead_code)]
    pub const FEATURE_CONTROL: u8 = 0x01;
    #[allow(dead_code)]
    pub const FEATURE_PERSIST: u8 = 0x02;

    pub fn decode(bytes: &[u8]) -> Result<Info, DecodeError> {
        if bytes.len() < Info::LEN {
            return Err(DecodeError::Truncated);
        }
        Ok(Info { proto_min: bytes[0], proto_max: bytes[1], max_colors: bytes[2], features: bytes[3] })
    }

    pub fn supports(&self, version: u8) -> bool {
        (self.proto_min..=self.proto_max).contains(&version)
    }
}

/// Render a packet as the annotated hex dump `omawatch encode` prints.
pub fn hexdump_annotated(bytes: &[u8]) -> String {
    let mut s = String::new();
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ");
    if bytes.len() < HEADER_LEN + CRC_LEN {
        return hex(bytes);
    }
    s.push_str(&format!("{:<14} {}\n", "magic", hex(&bytes[0..2])));
    s.push_str(&format!("{:<14} {}\n", "version", hex(&bytes[2..3])));
    s.push_str(&format!("{:<14} {}\n", "flags", hex(&bytes[3..4])));
    s.push_str(&format!("{:<14} {}\n", "n_colors", hex(&bytes[4..5])));
    let n = bytes[4] as usize;
    let mut i = HEADER_LEN;
    for slot in 0..n {
        if i + 3 > bytes.len() - CRC_LEN {
            break;
        }
        let label = Role::from_slot(slot as u8).map(|r| r.name()).unwrap_or("(unknown slot)");
        s.push_str(&format!("{:<14} {}\n", label, hex(&bytes[i..i + 3])));
        i += 3;
    }
    let body_end = bytes.len() - CRC_LEN;
    if i < body_end {
        s.push_str(&format!("{:<14} {}\n", "tlv", hex(&bytes[i..body_end])));
    }
    s.push_str(&format!("{:<14} {}\n", "crc16", hex(&bytes[body_end..])));
    s.push_str(&format!("total {} bytes", bytes.len()));
    s
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    let clean: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() % 2 != 0 {
        return None;
    }
    (0..clean.len()).step_by(2).map(|i| u8::from_str_radix(&clean[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WatchPalette {
        let mut p = WatchPalette {
            mode: Mode::Dark,
            name: "tokyo-night".into(),
            colors: [Rgb::default(); Role::COUNT],
        };
        for (i, role) in Role::ALL.iter().enumerate() {
            p.set(*role, Rgb::new(i as u8, 0x80 + i as u8, 0xff - i as u8));
        }
        p
    }

    #[test]
    fn crc16_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
        assert_eq!(crc16(b""), 0xFFFF);
    }

    #[test]
    fn round_trip() {
        let p = sample();
        let bytes = encode_theme(&p);
        // 5 header + 14*3 colours + (2 + 11) name TLV + 2 crc
        assert_eq!(bytes.len(), 5 + 42 + 13 + 2);
        let d = decode_theme(&bytes).unwrap();
        assert_eq!(d.version, 1);
        assert_eq!(d.mode, Mode::Dark);
        assert_eq!(d.name.as_deref(), Some("tokyo-night"));
        assert_eq!(d.colors.len(), Role::COUNT);
        assert!(d.unknown_tlvs.is_empty());
        let back = d.into_palette(&sample());
        assert_eq!(back, p);
    }

    #[test]
    fn light_flag_survives() {
        let mut p = sample();
        p.mode = Mode::Light;
        p.name.clear();
        let bytes = encode_theme(&p);
        assert_eq!(bytes.len(), 5 + 42 + 2); // no name TLV
        assert_eq!(bytes[3], FLAG_LIGHT);
        assert_eq!(decode_theme(&bytes).unwrap().mode, Mode::Light);
    }

    #[test]
    fn rejects_corruption() {
        let mut bytes = encode_theme(&sample());
        bytes[10] ^= 0x01;
        assert!(matches!(decode_theme(&bytes), Err(DecodeError::BadCrc { .. })));

        let mut bytes = encode_theme(&sample());
        bytes[0] = 0;
        assert_eq!(decode_theme(&bytes), Err(DecodeError::BadMagic));

        let bytes = encode_theme(&sample());
        assert_eq!(decode_theme(&bytes[..20]), Err(DecodeError::BadCrc { expected: u16::from_le_bytes([bytes[18], bytes[19]]), actual: crc16(&bytes[..18]) }));
        assert_eq!(decode_theme(&bytes[..4]), Err(DecodeError::Truncated));

        let mut bytes = encode_theme(&sample());
        bytes[2] = 2;
        // fix the crc so the version check is what fails
        let n = bytes.len();
        let crc = crc16(&bytes[..n - 2]).to_le_bytes();
        bytes[n - 2] = crc[0];
        bytes[n - 1] = crc[1];
        assert_eq!(decode_theme(&bytes), Err(DecodeError::UnsupportedVersion(2)));
    }

    #[test]
    fn older_receiver_ignores_extra_slots_and_tlvs() {
        // A "future" sender: 16 colour slots and an unknown TLV tag 0x7e.
        let p = sample();
        let mut body = vec![MAGIC_THEME[0], MAGIC_THEME[1], VERSION, 0, 16];
        for role in Role::ALL {
            let c = p.get(role);
            body.extend_from_slice(&[c.r, c.g, c.b]);
        }
        body.extend_from_slice(&[1, 2, 3, 4, 5, 6]); // two extra slots
        body.extend_from_slice(&[0x7e, 3, 0xaa, 0xbb, 0xcc]); // unknown tlv
        body.extend_from_slice(&[TLV_NAME, 3, b'n', b'e', b'w']);
        let crc = crc16(&body).to_le_bytes();
        body.extend_from_slice(&crc);

        let d = decode_theme(&body).unwrap();
        assert_eq!(d.colors.len(), 16);
        assert_eq!(d.unknown_tlvs, vec![(0x7e, vec![0xaa, 0xbb, 0xcc])]);
        assert_eq!(d.name.as_deref(), Some("new"));
        let back = d.into_palette(&sample());
        assert_eq!(back.colors, p.colors); // only the 14 known slots were applied
    }

    #[test]
    fn newer_receiver_keeps_defaults_for_missing_slots() {
        // An "old" sender that only knew 12 slots.
        let p = sample();
        let mut body = vec![MAGIC_THEME[0], MAGIC_THEME[1], VERSION, 0, 12];
        for role in &Role::ALL[..12] {
            let c = p.get(*role);
            body.extend_from_slice(&[c.r, c.g, c.b]);
        }
        let crc = crc16(&body).to_le_bytes();
        body.extend_from_slice(&crc);
        let mut defaults = sample();
        defaults.set(Role::Success, Rgb::new(9, 9, 9));
        defaults.set(Role::Info, Rgb::new(8, 8, 8));
        let back = decode_theme(&body).unwrap().into_palette(&defaults);
        assert_eq!(back.get(Role::Success), Rgb::new(9, 9, 9));
        assert_eq!(back.get(Role::Info), Rgb::new(8, 8, 8));
        assert_eq!(back.get(Role::Background), p.get(Role::Background));
    }

    #[test]
    fn malformed_tlv_is_rejected() {
        let mut body = vec![MAGIC_THEME[0], MAGIC_THEME[1], VERSION, 0, 0];
        body.extend_from_slice(&[TLV_NAME, 10, b'x']); // claims 10 bytes, has 1
        let crc = crc16(&body).to_le_bytes();
        body.extend_from_slice(&crc);
        assert_eq!(decode_theme(&body), Err(DecodeError::BadTlv));
    }

    #[test]
    fn control_and_status_round_trip() {
        for c in [Control::NextTheme, Control::PrevTheme, Control::ToggleMode, Control::Resend] {
            assert_eq!(Control::decode(&c.encode()), Ok(c));
        }
        assert_eq!(Control::decode(&[0x54, 0x43, 1, 0x77]), Err(DecodeError::BadTlv));
        let st = Status { version: 1, result: StatusCode::Ok, applied_crc: 0xBEEF, n_applied: 14, mode: Mode::Light };
        assert_eq!(Status::decode(&st.encode()), Ok(st));
        let info = Info::decode(&[1, 1, 14, 3]).unwrap();
        assert!(info.supports(1));
        assert!(!info.supports(2));
        assert_eq!(info.features & Info::FEATURE_CONTROL, Info::FEATURE_CONTROL);
    }

    #[test]
    fn hex_helpers() {
        let bytes = encode_theme(&sample());
        assert_eq!(from_hex(&to_hex(&bytes)).unwrap(), bytes);
        assert_eq!(from_hex("54 48\n01"), Some(vec![0x54, 0x48, 1]));
        assert_eq!(from_hex("abc"), None);
    }
}
