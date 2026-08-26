//! The Omarchy theme list, pushed to the watch over a GATT connection
//! (`protocol/BEACON.md` §3, characteristic `7a0e0006`): every installed theme as the same v2
//! packet the state beacon carries, so the watch can show a tappable picker and paint a
//! theme before the desktop confirms it.
//!
//! ```text
//! LIST BYTES   [0x01 ver][count u8] then count × [len u8][v2 packet: [2] roles… 0x40 slug 0x41 flags]
//!              omarchy-theme-list order (= NEXT/PREV order); no 0x42/0x43 neighbour records.
//! READ status  [0x01 ver][count][crc16 le][flags]   5 bytes; flags bit0 stored on SD, bit1 a list is loaded
//! WRITE frames BEGIN  [0x01][count][total u16 le][crc16 u16 le]
//!              DATA   [0x02][offset u16 le][bytes…]     strictly sequential
//!              COMMIT [0x03][mac 4 B]                   mac = HMAC-SHA256(pairing key, list bytes)[0..4]
//! ```
//!
//! crc16 is CRC-16/CCITT-FALSE ([`crc16`]), the one the SET request already uses.

use std::fmt;

use crate::beacon::mac4;
use crate::omarchy::Omarchy;
use crate::palette::map_source;
use crate::protocol::{self, crc16};

pub const LIST_VERSION: u8 = 1;
/// `THEMELIST_MAX` / `THEMELIST_MAX_BYTES` in the watch firmware (`main/themelist.h`).
pub const MAX_ENTRIES: usize = 64;
pub const MAX_BYTES: usize = 8192;

pub const FRAME_BEGIN: u8 = 0x01;
pub const FRAME_DATA: u8 = 0x02;
pub const FRAME_COMMIT: u8 = 0x03;
pub const BEGIN_LEN: usize = 6;
pub const DATA_HEADER_LEN: usize = 3;
pub const MAC_LEN: usize = 4;
pub const COMMIT_LEN: usize = 1 + MAC_LEN;

/// The largest frame the watch accepts (its preferred MTU 512 − 3) and the smallest one
/// that is still worth sending (default ATT MTU 23 − 3).
pub const MAX_FRAME: usize = 509;
pub const MIN_FRAME: usize = 20;

pub const STATUS_LEN: usize = 5;
pub const STATUS_FLAG_SD: u8 = 0x01;
pub const STATUS_FLAG_LOADED: u8 = 0x02;

/// ATT application error codes the watch answers a bad frame with. After any rejected frame
/// the transfer is dead on the watch (further DATA fails with 0x81) until a new BEGIN, which
/// is why a failed push is retried from scratch, never resumed.
pub const ATT_ERROR_FIRST: u8 = 0x80;
pub const ATT_ERROR_LAST: u8 = 0x85;
pub fn att_error_meaning(code: u8) -> Option<&'static str> {
    Some(match code {
        0x80 => "DATA out of order (offset != bytes received so far)",
        0x81 => "DATA/COMMIT without a BEGIN (or after a rejected frame)",
        0x82 => "list too big for the watch",
        0x83 => "COMMIT rejected: bad MAC (pairing key differs)",
        0x84 => "COMMIT rejected: bad CRC",
        0x85 => "COMMIT rejected: the list bytes do not parse",
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListError {
    TooManyEntries(usize),
    EntryTooLong { index: usize, len: usize },
    TooBig(usize),
    Truncated,
    BadVersion(u8),
}

impl fmt::Display for ListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListError::TooManyEntries(n) => write!(f, "{n} themes, the watch stores at most {MAX_ENTRIES}"),
            ListError::EntryTooLong { index, len } => write!(f, "entry {index} is {len} bytes, at most 255 fit"),
            ListError::TooBig(n) => write!(f, "list is {n} bytes, the watch accepts at most {MAX_BYTES}"),
            ListError::Truncated => write!(f, "list truncated"),
            ListError::BadVersion(v) => write!(f, "unknown list version {v}"),
        }
    }
}

impl std::error::Error for ListError {}

/// `[ver][count]` + `[len][packet]`* — the bytes the watch stores verbatim on its card.
pub fn encode_list(packets: &[Vec<u8>]) -> Result<Vec<u8>, ListError> {
    if packets.len() > MAX_ENTRIES {
        return Err(ListError::TooManyEntries(packets.len()));
    }
    let mut out = Vec::with_capacity(2 + packets.iter().map(|p| p.len() + 1).sum::<usize>());
    out.push(LIST_VERSION);
    out.push(packets.len() as u8);
    for (i, p) in packets.iter().enumerate() {
        if p.len() > 255 {
            return Err(ListError::EntryTooLong { index: i, len: p.len() });
        }
        out.push(p.len() as u8);
        out.extend_from_slice(p);
    }
    if out.len() > MAX_BYTES {
        return Err(ListError::TooBig(out.len()));
    }
    Ok(out)
}

/// The entries of a list, as the watch would split them (the packets are not parsed).
pub fn decode_list(b: &[u8]) -> Result<Vec<Vec<u8>>, ListError> {
    if b.len() < 2 {
        return Err(ListError::Truncated);
    }
    if b[0] != LIST_VERSION {
        return Err(ListError::BadVersion(b[0]));
    }
    let count = b[1] as usize;
    let mut out = Vec::with_capacity(count);
    let mut i = 2;
    for _ in 0..count {
        let len = *b.get(i).ok_or(ListError::Truncated)? as usize;
        let end = i + 1 + len;
        if end > b.len() {
            return Err(ListError::Truncated);
        }
        out.push(b[i + 1..end].to_vec());
        i = end;
    }
    Ok(out)
}

/// The COMMIT MAC: first 4 bytes of HMAC-SHA256(pairing key, list bytes).
pub fn list_mac(key: &[u8], list: &[u8]) -> [u8; MAC_LEN] {
    mac4(key, list)
}

/// Clamp a DATA payload size to what the protocol allows: `mtu - 3` capped at [`MAX_FRAME`].
pub fn frame_len_for_mtu(mtu: u16) -> usize {
    (mtu as usize).saturating_sub(3).clamp(MIN_FRAME, MAX_FRAME)
}

/// Every write for one transfer, in order: BEGIN, the DATA frames (each at most `frame`
/// bytes in total), COMMIT.
pub fn frames(list: &[u8], key: &[u8], frame: usize) -> Vec<Vec<u8>> {
    let frame = frame.clamp(MIN_FRAME, MAX_FRAME);
    let chunk = frame - DATA_HEADER_LEN;
    let count = list.get(1).copied().unwrap_or(0);
    let mut out = Vec::with_capacity(2 + list.len().div_ceil(chunk));
    let mut begin = vec![FRAME_BEGIN, count];
    begin.extend_from_slice(&(list.len() as u16).to_le_bytes());
    begin.extend_from_slice(&crc16(list).to_le_bytes());
    out.push(begin);
    for (i, part) in list.chunks(chunk).enumerate() {
        let mut f = Vec::with_capacity(DATA_HEADER_LEN + part.len());
        f.push(FRAME_DATA);
        f.extend_from_slice(&((i * chunk) as u16).to_le_bytes());
        f.extend_from_slice(part);
        out.push(f);
    }
    let mut commit = vec![FRAME_COMMIT];
    commit.extend_from_slice(&list_mac(key, list));
    out.push(commit);
    out
}

/// One line per frame for `push-list --dry-run`.
pub fn describe_frame(f: &[u8]) -> String {
    match f.first() {
        Some(&FRAME_BEGIN) if f.len() == BEGIN_LEN => format!("BEGIN  count {} total {} crc {:#06x}", f[1], u16::from_le_bytes([f[2], f[3]]), u16::from_le_bytes([f[4], f[5]])),
        Some(&FRAME_DATA) if f.len() >= DATA_HEADER_LEN => format!("DATA   offset {:>5} len {:>3}", u16::from_le_bytes([f[1], f[2]]), f.len() - DATA_HEADER_LEN),
        Some(&FRAME_COMMIT) if f.len() == COMMIT_LEN => format!("COMMIT mac {}", protocol::to_hex(&f[1..])),
        _ => format!("({} bytes)", f.len()),
    }
}

/// What the watch reports on a READ of the list characteristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListStatus {
    pub version: u8,
    pub count: u8,
    pub crc: u16,
    pub on_sd: bool,
    pub loaded: bool,
}

impl ListStatus {
    pub fn decode(b: &[u8]) -> Result<ListStatus, ListError> {
        if b.len() < STATUS_LEN {
            return Err(ListError::Truncated);
        }
        if b[0] != LIST_VERSION {
            return Err(ListError::BadVersion(b[0]));
        }
        Ok(ListStatus { version: b[0], count: b[1], crc: u16::from_le_bytes([b[2], b[3]]), on_sd: b[4] & STATUS_FLAG_SD != 0, loaded: b[4] & STATUS_FLAG_LOADED != 0 })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn encode(&self) -> [u8; STATUS_LEN] {
        let crc = self.crc.to_le_bytes();
        [self.version, self.count, crc[0], crc[1], (if self.on_sd { STATUS_FLAG_SD } else { 0 }) | (if self.loaded { STATUS_FLAG_LOADED } else { 0 })]
    }

    /// True when the watch already holds exactly this list.
    pub fn holds(&self, list: &[u8]) -> bool {
        self.loaded && self.crc == crc16(list) && self.count as usize == list.get(1).copied().unwrap_or(0) as usize
    }
}

impl fmt::Display for ListStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} themes, crc {:#06x}, {}{}", self.count, self.crc, if self.loaded { "loaded" } else { "no list" }, if self.on_sd { ", on SD" } else { "" })
    }
}

/// The list built from the installed Omarchy themes.
#[derive(Debug, Clone, Default)]
pub struct Built {
    pub bytes: Vec<u8>,
    /// Slugs in list order.
    pub slugs: Vec<String>,
    /// Themes left out (unresolvable, or beyond the watch's limits) with the reason.
    pub skipped: Vec<(String, String)>,
}

impl Built {
    pub fn crc(&self) -> u16 {
        crc16(&self.bytes)
    }
}

/// Resolve every installed theme to its v2 packet (all roles, slug as the name, no
/// neighbours), in `omarchy-theme-list` order. Themes that do not resolve are skipped, and
/// the list is cut at the watch's limits rather than refused.
///
/// The order must stay the one `Omarchy::neighbour_of` steps through (both call
/// `list_themes`): the watch applies a list entry locally before its NEXT/PREV lands here,
/// and the beacon only confirms.
pub fn build(om: &Omarchy) -> Built {
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut built = Built::default();
    let mut total = 2usize;
    for slug in om.list_themes() {
        if packets.len() == MAX_ENTRIES {
            built.skipped.push((slug, format!("beyond the watch's {MAX_ENTRIES}-entry limit")));
            continue;
        }
        let packet = match om.load_theme(&slug).and_then(|s| map_source(&s).map_err(Into::into)) {
            Ok(mut p) => {
                p.name = slug.clone();
                protocol::encode_v2(&p, false)
            }
            Err(e) => {
                built.skipped.push((slug, format!("{e:#}")));
                continue;
            }
        };
        if total + 1 + packet.len() > MAX_BYTES {
            built.skipped.push((slug, format!("beyond the watch's {MAX_BYTES}-byte limit")));
            continue;
        }
        total += 1 + packet.len();
        built.slugs.push(slug);
        packets.push(packet);
    }
    built.bytes = encode_list(&packets).expect("limits enforced above");
    built
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{Mode, Rgb, Role, WatchPalette};

    fn theme(slug: &str, seed: u8) -> Vec<u8> {
        let mut p = WatchPalette { mode: if seed % 2 == 0 { Mode::Dark } else { Mode::Light }, name: slug.into(), colors: [Rgb::default(); Role::COUNT] };
        for (i, r) in Role::ALL.iter().enumerate() {
            p.set(*r, Rgb::new(seed.wrapping_mul(3).wrapping_add(i as u8), i as u8, seed));
        }
        protocol::encode_v2(&p, false)
    }

    #[test]
    fn list_round_trip_keeps_order_and_packets() {
        let packets = vec![theme("catppuccin", 1), theme("gruvbox", 2), theme("tokyo-night", 3)];
        let list = encode_list(&packets).unwrap();
        assert_eq!(&list[..2], &[LIST_VERSION, 3]);
        assert_eq!(list.len(), 2 + packets.iter().map(|p| p.len() + 1).sum::<usize>());
        assert_eq!(decode_list(&list).unwrap(), packets);
        // every entry is a plain v2 packet the theme parser accepts, slug as the name
        for (p, slug) in decode_list(&list).unwrap().iter().zip(["catppuccin", "gruvbox", "tokyo-night"]) {
            let d = protocol::decode_v2(p).unwrap();
            assert_eq!(d.name.as_deref(), Some(slug));
            assert_eq!(d.colors.len(), Role::COUNT);
            assert!(d.prev.is_none() && d.next.is_none());
        }
        assert_eq!(decode_list(&list[..list.len() - 1]), Err(ListError::Truncated));
        assert_eq!(decode_list(&[9, 0]), Err(ListError::BadVersion(9)));
    }

    #[test]
    fn limits_match_the_firmware() {
        let many: Vec<Vec<u8>> = (0..65).map(|i| theme("x", i)).collect();
        assert_eq!(encode_list(&many), Err(ListError::TooManyEntries(65)));
        assert!(encode_list(&many[..64]).is_ok());
        assert_eq!(encode_list(&[vec![2; 256]]), Err(ListError::EntryTooLong { index: 0, len: 256 }));
        let fat: Vec<Vec<u8>> = (0..40).map(|_| vec![2; 250]).collect(); // 40 * 251 + 2 > 8192
        assert_eq!(encode_list(&fat), Err(ListError::TooBig(2 + 40 * 251)));
    }

    #[test]
    fn frames_reassemble_and_stay_within_the_frame_size() {
        let packets: Vec<Vec<u8>> = (0..19).map(|i| theme(&format!("theme-{i}"), i)).collect();
        let list = encode_list(&packets).unwrap();
        let key = [7u8; 16];
        for frame in [MIN_FRAME, 100, 244, MAX_FRAME] {
            let fs = frames(&list, &key, frame);
            assert_eq!(fs[0], {
                let mut b = vec![FRAME_BEGIN, 19];
                b.extend_from_slice(&(list.len() as u16).to_le_bytes());
                b.extend_from_slice(&crc16(&list).to_le_bytes());
                b
            });
            let mut joined = Vec::new();
            for f in &fs[1..fs.len() - 1] {
                assert_eq!(f[0], FRAME_DATA);
                assert!(f.len() <= frame, "frame {} > {frame}", f.len());
                assert_eq!(u16::from_le_bytes([f[1], f[2]]) as usize, joined.len(), "offsets are sequential");
                joined.extend_from_slice(&f[3..]);
            }
            assert_eq!(joined, list);
            let last = fs.last().unwrap();
            assert_eq!(last.len(), COMMIT_LEN);
            assert_eq!(last[0], FRAME_COMMIT);
            assert_eq!(&last[1..], &list_mac(&key, &list));
            assert_eq!(fs.len(), 2 + list.len().div_ceil(frame - DATA_HEADER_LEN));
        }
        assert_eq!(frame_len_for_mtu(23), MIN_FRAME);
        assert_eq!(frame_len_for_mtu(256), 253);
        assert_eq!(frame_len_for_mtu(512), MAX_FRAME);
        assert_eq!(frame_len_for_mtu(517), MAX_FRAME);
        assert_eq!(frames(&list, &key, 1)[1].len(), MIN_FRAME); // silly sizes are clamped
    }

    #[test]
    fn status_round_trip_and_holds() {
        let list = encode_list(&[theme("nord", 4), theme("rose-pine", 5)]).unwrap();
        let st = ListStatus { version: 1, count: 2, crc: crc16(&list), on_sd: true, loaded: true };
        assert_eq!(ListStatus::decode(&st.encode()), Ok(st));
        assert!(st.holds(&list));
        assert!(!ListStatus { loaded: false, ..st }.holds(&list));
        assert!(!ListStatus { crc: st.crc ^ 1, ..st }.holds(&list));
        assert!(!ListStatus { count: 3, ..st }.holds(&list));
        assert_eq!(ListStatus::decode(&[1, 0, 0, 0]), Err(ListError::Truncated));
        assert_eq!(ListStatus::decode(&[2, 0, 0, 0, 0]), Err(ListError::BadVersion(2)));
        assert_eq!(st.to_string(), format!("2 themes, crc {:#06x}, loaded, on SD", st.crc));
    }

    /// Interop anchor shared with the watch firmware: key 00 01 .. 0f, one two-colour
    /// entry named "nord". The watch side can check its crc16 and HMAC against these.
    #[test]
    fn list_test_vector() {
        let key: Vec<u8> = (0u8..16).collect();
        let entry = vec![0x02, 0x01, 0x10, 0x20, 0x30, 0x02, 0x40, 0x50, 0x60, 0x40, 0x04, b'n', b'o', b'r', b'd', 0x41, 0x01, 0x00];
        let list = encode_list(&[entry.clone()]).unwrap();
        let mut expected = vec![0x01, 0x01, 0x12];
        expected.extend_from_slice(&entry);
        assert_eq!(list, expected);
        assert_eq!(crc16(&list), 0xDB97);
        assert_eq!(list_mac(&key, &list), [0x35, 0xe8, 0x31, 0xb9]);
        let fs = frames(&list, &key, MAX_FRAME);
        assert_eq!(fs[0], vec![0x01, 0x01, 0x15, 0x00, 0x97, 0xdb]);
        assert_eq!(fs[1][..3], [0x02, 0x00, 0x00]);
        assert_eq!(fs[2], vec![0x03, 0x35, 0xe8, 0x31, 0xb9]);
        assert_eq!(att_error_meaning(0x83).unwrap(), "COMMIT rejected: bad MAC (pairing key differs)");
        assert!((ATT_ERROR_FIRST..=ATT_ERROR_LAST).all(|c| att_error_meaning(c).is_some()));
        assert!(att_error_meaning(0x01).is_none() && att_error_meaning(0x86).is_none());
    }
}
