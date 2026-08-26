//! The two advertising packets of `protocol/BEACON.md`: the desktop's state beacon and the
//! watch's request. Both are Manufacturer Specific Data under company id 0xFFFF.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::protocol::crc16;

pub const COMPANY_ID: u16 = 0xFFFF;
pub const MAGIC: u8 = 0x54; // 'T'
pub const KIND_STATE: u8 = 0x01;
pub const KIND_REQUEST: u8 = 0x02;
pub const REQUEST_LEN: usize = 10;
pub const KEY_LEN: usize = 16;
/// First byte of the `…0005` write: `[0x01][code][key 16]` (protocol/BEACON.md §2b).
pub const PAIR_WRITE_TAG: u8 = 0x01;
pub const PAIR_WRITE_LEN: usize = 2 + KEY_LEN;

/// The 18-byte pairing write.
pub fn encode_pair_write(code: u8, key: &[u8; KEY_LEN]) -> [u8; PAIR_WRITE_LEN] {
    let mut b = [0u8; PAIR_WRITE_LEN];
    b[0] = PAIR_WRITE_TAG;
    b[1] = code;
    b[2..].copy_from_slice(key);
    b
}

/// Which key a request verified with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verified {
    Active,
    /// Verified with the pending key: the watch confirmed the pairing code.
    Pending,
}

/// Try the active key, then the pending one.
pub fn decode_request_with(active: Option<&[u8; KEY_LEN]>, pending: Option<&[u8; KEY_LEN]>, b: &[u8]) -> Result<(Request, Verified), RequestError> {
    let mut last = RequestError::BadMac;
    if let Some(k) = active {
        match decode_request(k, b) {
            Ok(r) => return Ok((r, Verified::Active)),
            Err(RequestError::BadMac) => {}
            Err(e) => return Err(e),
        }
    }
    if let Some(k) = pending {
        match decode_request(k, b) {
            Ok(r) => return Ok((r, Verified::Pending)),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// `[magic][kind=state][seq][host][theme packet...]`
pub fn encode_state(seq: u8, host: u8, theme: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + theme.len());
    v.extend_from_slice(&[MAGIC, KIND_STATE, seq, host]);
    v.extend_from_slice(theme);
    v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Next,
    Prev,
    /// 0x03 was TOGGLE (dark/light); dropped 2026-08-26 — Omarchy themes have no paired
    /// variants, so it only jumped to an unrelated theme. The opcode stays reserved.
    Set,
    Resend,
    /// "Push me the theme list over GATT" (protocol/BEACON.md §3); arg 0.
    List,
}

impl Op {
    pub fn code(self) -> u8 {
        match self {
            Op::Next => 1,
            Op::Prev => 2,
            Op::Set => 4,
            Op::Resend => 5,
            Op::List => 6,
        }
    }
    pub fn from_code(c: u8) -> Option<Op> {
        Some(match c {
            1 => Op::Next,
            2 => Op::Prev,
            4 => Op::Set,
            5 => Op::Resend,
            6 => Op::List,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// Random per button press; repeated while that press is advertised.
    pub nonce: u8,
    pub op: Op,
    /// `Set`: crc16 of the theme slug; otherwise 0.
    pub arg: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    NotARequest,
    BadLength(usize),
    BadOp(u8),
    BadMac,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestError::NotARequest => write!(f, "not a theme request"),
            RequestError::BadLength(n) => write!(f, "request is {n} bytes, expected {REQUEST_LEN}"),
            RequestError::BadOp(o) => write!(f, "unknown op {o:#04x}"),
            RequestError::BadMac => write!(f, "MAC does not verify (wrong key?)"),
        }
    }
}

/// First 4 bytes of HMAC-SHA256(key, data): the request MAC, and the theme list's COMMIT MAC.
pub fn mac4(key: &[u8], data: &[u8]) -> [u8; 4] {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(data);
    let out = m.finalize().into_bytes();
    [out[0], out[1], out[2], out[3]]
}

/// `[magic][kind=request][nonce][op][arg u16 le][mac 4]` — 10 bytes, no time, no counter.
#[cfg_attr(not(test), allow(dead_code))]
pub fn encode_request(key: &[u8], req: Request) -> [u8; REQUEST_LEN] {
    let mut b = [0u8; REQUEST_LEN];
    b[0] = MAGIC;
    b[1] = KIND_REQUEST;
    b[2] = req.nonce;
    b[3] = req.op.code();
    b[4..6].copy_from_slice(&req.arg.to_le_bytes());
    let m = mac4(key, &b[..6]);
    b[6..10].copy_from_slice(&m);
    b
}

/// Parse and authenticate. Telling a repeated advertisement of one press from a new press
/// (same address, nonce, op, arg within a short window) is the caller's job.
pub fn decode_request(key: &[u8], b: &[u8]) -> Result<Request, RequestError> {
    if b.len() < 2 || b[0] != MAGIC || b[1] != KIND_REQUEST {
        return Err(RequestError::NotARequest);
    }
    if b.len() != REQUEST_LEN {
        return Err(RequestError::BadLength(b.len()));
    }
    let op = Op::from_code(b[3]).ok_or(RequestError::BadOp(b[3]))?;
    if mac4(key, &b[..6]) != b[6..10] {
        return Err(RequestError::BadMac);
    }
    Ok(Request { nonce: b[2], op, arg: u16::from_le_bytes([b[4], b[5]]) })
}

/// True for any packet of ours (either kind), so the scanner can skip other 0xFFFF users.
pub fn is_ours(b: &[u8]) -> bool {
    b.len() >= 2 && b[0] == MAGIC && (b[1] == KIND_STATE || b[1] == KIND_REQUEST)
}

/// The `arg` of a `Set` request.
pub fn slug_id(slug: &str) -> u16 {
    crc16(slug.as_bytes())
}

/// CRC-8 (poly 0x07, init 0) of the hostname: the `host` byte of the state beacon.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

pub fn host_id() -> u8 {
    let name = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default();
    let id = crc8(name.as_bytes());
    if id == 0 { 1 } else { id } // 0 means "any" on the wire
}

/// The pairing key: `~/.config/themesync/key` (32 hex chars).
pub fn key_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("themesync").join("key")
}

pub fn load_key() -> Option<[u8; KEY_LEN]> {
    load_key_from(&key_path())
}

fn load_key_from(path: &std::path::Path) -> Option<[u8; KEY_LEN]> {
    let text = std::fs::read_to_string(path).ok()?;
    let bytes = crate::protocol::from_hex(text.trim())?;
    bytes.try_into().ok()
}

/// A key handed over by `themesync pair` but not yet confirmed by the watch. Persisted so a
/// daemon restart (deploys happen often) does not lose it; a late confirmation still counts.
pub fn pending_key_path() -> std::path::PathBuf {
    key_path().with_file_name("key.pending")
}

pub fn load_pending_key() -> Option<[u8; KEY_LEN]> {
    load_key_from(&pending_key_path())
}

pub fn save_pending_key(k: &[u8; KEY_LEN]) -> std::io::Result<()> {
    let path = pending_key_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, format!("{}\n", crate::protocol::to_hex(k)))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn clear_pending_key() {
    let _ = std::fs::remove_file(pending_key_path());
}

pub fn new_key() -> std::io::Result<[u8; KEY_LEN]> {
    use std::io::Read;
    let mut k = [0u8; KEY_LEN];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut k)?;
    Ok(k)
}

pub fn save_key(k: &[u8; KEY_LEN]) -> std::io::Result<std::path::PathBuf> {
    let path = key_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, format!("{}\n", crate::protocol::to_hex(k)))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 16] = [7; 16];

    #[test]
    fn request_round_trip_and_mac() {
        let r = Request { nonce: 0xA5, op: Op::Set, arg: slug_id("tokyo-night") };
        let b = encode_request(&KEY, r);
        assert_eq!(b.len(), REQUEST_LEN);
        assert_eq!(&b[..2], &[MAGIC, KIND_REQUEST]);
        assert_eq!(decode_request(&KEY, &b), Ok(r));
        // wrong key, flipped op, flipped counter: all rejected
        assert_eq!(decode_request(&[8; 16], &b), Err(RequestError::BadMac));
        let mut x = b; x[3] = Op::Next.code();
        assert_eq!(decode_request(&KEY, &x), Err(RequestError::BadMac));
        let mut y = b; y[2] ^= 1;
        assert_eq!(decode_request(&KEY, &y), Err(RequestError::BadMac));
        assert_eq!(decode_request(&KEY, &b[..9]), Err(RequestError::BadLength(9)));
        assert_eq!(decode_request(&KEY, &[MAGIC, KIND_STATE, 0]), Err(RequestError::NotARequest));
    }

    #[test]
    fn pending_key_confirms_pairing() {
        let old = [1u8; 16];
        let new = [2u8; 16];
        let with_new = encode_request(&new, Request { nonce: 9, op: Op::Resend, arg: 0 });
        let with_old = encode_request(&old, Request { nonce: 9, op: Op::Next, arg: 0 });
        assert_eq!(decode_request_with(Some(&old), Some(&new), &with_new).unwrap().1, Verified::Pending);
        assert_eq!(decode_request_with(Some(&old), Some(&new), &with_old).unwrap().1, Verified::Active);
        assert_eq!(decode_request_with(Some(&old), None, &with_new), Err(RequestError::BadMac));
        assert_eq!(decode_request_with(None, Some(&new), &with_new).unwrap().1, Verified::Pending);
        assert_eq!(decode_request_with(None, None, &with_new), Err(RequestError::BadMac));
        let w = encode_pair_write(0x7C, &new);
        assert_eq!(w.len(), PAIR_WRITE_LEN);
        assert_eq!(&w[..2], &[PAIR_WRITE_TAG, 0x7C]);
        assert_eq!(&w[2..], &new);
    }

    #[test]
    fn state_layout() {
        let s = encode_state(9, 0x5a, &[2, 1, 10, 20, 30]);
        assert_eq!(s, vec![MAGIC, KIND_STATE, 9, 0x5a, 2, 1, 10, 20, 30]);
        assert!(is_ours(&s));
        assert!(!is_ours(&[0x00, 0x01]));
    }

    #[test]
    fn crc8_known_value() {
        // CRC-8 (poly 0x07, init 0, no reflection): check value for "123456789" is 0xF4
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_ne!(host_id(), 0);
    }
    /// Interop anchor shared with the watch firmware: key 00 01 .. 0f.
    #[test]
    fn request_test_vectors() {
        let key: Vec<u8> = (0u8..16).collect();
        let next = encode_request(&key, Request { nonce: 0xA5, op: Op::Next, arg: 0 });
        assert_eq!(next.to_vec(), vec![0x54, 0x02, 0xa5, 0x01, 0x00, 0x00, 0x18, 0x11, 0x17, 0x68]);
        let set = encode_request(&key, Request { nonce: 0x3C, op: Op::Set, arg: slug_id("tokyo-night") });
        assert_eq!(slug_id("tokyo-night"), 0xAAE5);
        assert_eq!(set.to_vec(), vec![0x54, 0x02, 0x3c, 0x04, 0xe5, 0xaa, 0x68, 0x05, 0xb1, 0xb4]);
        let list = encode_request(&key, Request { nonce: 0x11, op: Op::List, arg: 0 });
        assert_eq!(&list[..6], &[0x54, 0x02, 0x11, 0x06, 0x00, 0x00]);
        assert_eq!(decode_request(&key, &list).unwrap().op, Op::List);
        assert_eq!(Op::from_code(6), Some(Op::List));
        assert_eq!(Op::from_code(3), None);
    }
}
