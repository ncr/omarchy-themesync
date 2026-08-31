//! The two advertising packets of `protocol/BEACON.md` (v3): the desktop's state beacon and
//! the watch's request. Both are Manufacturer Specific Data under company id 0xFFFF, both
//! carry `mac4` = the first 4 bytes of HMAC-SHA256 under the pairing key.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::protocol::{self, crc16};

pub const COMPANY_ID: u16 = 0xFFFF;
pub const MAGIC: u8 = 0x54; // 'T'
pub const KIND_STATE: u8 = 0x01;
/// v3 request. v1/v2 requests were kind 0x02 and 10 bytes; a mixed pair fails on the kind.
pub const KIND_REQUEST: u8 = 0x03;
/// The watch's "pair with me" advertisement (BEACON.md §2b): `'T' 0x04 [token u32 le]`,
/// unsigned (there is no key yet), on its connectable advertisement so the address it comes
/// from is the one to connect to.
pub const KIND_PAIR: u8 = 0x04;
pub const PAIR_ADV_LEN: usize = 6;
pub const REQUEST_LEN: usize = 11;
pub const MAC_LEN: usize = 4;
pub const KEY_LEN: usize = 16;
/// First byte of the `…0005` write: `[0x01][code][key 16][name 0–12 B]` (protocol/BEACON.md §2b).
pub const PAIR_WRITE_TAG: u8 = 0x01;
pub const PAIR_WRITE_LEN: usize = 2 + KEY_LEN;
/// The desktop's name in the offer, so the watch can list "which desktop" when several answer.
pub const PAIR_NAME_MAX: usize = 12;
/// A pending pairing key is honoured for this long (BEACON.md §2b); the watch's own window
/// is the same.
pub const PENDING_KEY_TTL: std::time::Duration = std::time::Duration::from_secs(120);
/// The largest theme packet the beacon carries. The legitimate maximum is 93 bytes (14 roles,
/// a 31-byte name, flags); extended advertising fits ~250 bytes of manufacturer data. Anything
/// above this is a bug or an attack on the daemon's own socket, not a theme.
pub const MAX_THEME_LEN: usize = 200;

/// Constant-time equality for the 4-byte MACs (no timing oracle, even a theoretical one).
fn mac_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

/// Is `theme` a v2 packet the beacon may carry: decodable, no meta records of its own (they
/// would confuse `v2_theme_end` on the receiver), and within [`MAX_THEME_LEN`].
pub fn check_theme(theme: &[u8]) -> Result<(), String> {
    if theme.len() > MAX_THEME_LEN {
        return Err(format!("theme packet is {} bytes, more than {MAX_THEME_LEN}", theme.len()));
    }
    protocol::decode_v2(theme).map_err(|e| format!("theme packet does not decode: {e}"))?;
    if protocol::v2_theme_end(theme) != theme.len() {
        return Err("theme packet already carries a 0x42/0x43 meta record".into());
    }
    Ok(())
}

/// The pairing offer written to the watch: 18 bytes plus the desktop's name (cut to
/// [`PAIR_NAME_MAX`] bytes on a character boundary).
pub fn encode_pair_write(code: u8, key: &[u8; KEY_LEN], name: &str) -> Vec<u8> {
    let mut b = Vec::with_capacity(PAIR_WRITE_LEN + PAIR_NAME_MAX);
    b.push(PAIR_WRITE_TAG);
    b.push(code);
    b.extend_from_slice(key);
    let mut n = 0;
    for ch in name.chars() {
        if n + ch.len_utf8() > PAIR_NAME_MAX {
            break;
        }
        n += ch.len_utf8();
    }
    b.extend_from_slice(&name.as_bytes()[..n]);
    b
}

/// The token of a `'T' 0x04` pairing advertisement, if `b` is one.
pub fn decode_pair_adv(b: &[u8]) -> Option<u32> {
    if b.len() != PAIR_ADV_LEN || b[0] != MAGIC || b[1] != KIND_PAIR {
        return None;
    }
    Some(u32::from_le_bytes([b[2], b[3], b[4], b[5]]))
}

/// This desktop's name for the pairing offer: the hostname, else "desktop".
pub fn desktop_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "desktop".into())
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

/// The desktop's local civil time for the `0x44` beacon record:
/// `[year-2000][month 1-12][day][hour][min][sec][weekday 0-6 = Sunday..Saturday]`.
/// Civil local time on purpose: the watch face shows what the desktop's clock shows, and
/// neither side needs a time zone database. Out-of-range years saturate; a leap second
/// reads as :59.
#[cfg(unix)]
pub fn local_time_record() -> [u8; 7] {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let t = now.as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    [
        (tm.tm_year - 100).clamp(0, 255) as u8, // tm_year counts from 1900
        (tm.tm_mon + 1) as u8,
        tm.tm_mday as u8,
        tm.tm_hour as u8,
        tm.tm_min as u8,
        tm.tm_sec.min(59) as u8,
        tm.tm_wday as u8,
    ]
}

/// `[magic][kind=state][theme v2 TLV][0x43 2 echo]([0x44 7 time])[0x42 4 mac]` — `echo` is the last request
/// counter accepted (the watch's ack number), the mac covers everything before its record
/// (BEACON.md §1). `theme` must be a v2 packet without meta records.
pub fn encode_state(key: &[u8], theme: &[u8], echo: u16, time: Option<&[u8; 7]>) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 + theme.len() + 4 + 9 + 2 + MAC_LEN);
    v.extend_from_slice(&[MAGIC, KIND_STATE]);
    v.extend_from_slice(theme);
    protocol::v2_append_echo(&mut v, echo);
    if let Some(t) = time {
        protocol::v2_append_time(&mut v, t);
    }
    let mac = mac4(key, &v);
    protocol::v2_append_mac(&mut v, &mac);
    v
}

/// The receiver's view of a state beacon: the theme bytes (what the apply rule hashes),
/// the echo, and the time record when present — if the packet is ours and the mac verifies.
#[cfg_attr(not(test), allow(dead_code))]
pub fn decode_state<'a>(key: &[u8], b: &'a [u8]) -> Result<(&'a [u8], u16, Option<[u8; 7]>), RequestError> {
    if b.len() < 2 || b[0] != MAGIC || b[1] != KIND_STATE {
        return Err(RequestError::NotARequest);
    }
    let theme = &b[2..];
    let end = protocol::v2_theme_end(theme);
    let meta = &theme[end..];
    // tail: [0x43 2 echo], an optional [0x44 7 time], then [0x42 4 mac] last.
    let with_time = meta.len() == 4 + 9 + 2 + MAC_LEN;
    if !(with_time || meta.len() == 4 + 2 + MAC_LEN) || meta[0] != protocol::V2_TAG_ECHO || meta[1] != 2 {
        return Err(RequestError::BadLength(b.len()));
    }
    let (time, mac_rec) = if with_time {
        if meta[4] != protocol::V2_TAG_TIME || meta[5] != 7 {
            return Err(RequestError::BadLength(b.len()));
        }
        (Some(<[u8; 7]>::try_from(&meta[6..13]).unwrap()), &meta[13..])
    } else {
        (None, &meta[4..])
    };
    if mac_rec[0] != protocol::V2_TAG_MAC || mac_rec[1] as usize != MAC_LEN {
        return Err(RequestError::BadLength(b.len()));
    }
    if !mac_eq(&mac4(key, &b[..b.len() - MAC_LEN - 2]), &mac_rec[2..]) {
        return Err(RequestError::BadMac);
    }
    Ok((&theme[..end], u16::from_le_bytes([meta[2], meta[3]]), time))
}

/// The watch names the theme it wants (`Set`, by slug crc); there is no relative stepping —
/// the watch holds the whole list (protocol/BEACON.md §3) and picks from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `arg` = crc16 of the theme slug.
    Set,
    /// "Burst the state beacon again."
    Resend,
    /// "Push me the theme list over GATT" (protocol/BEACON.md §3); arg 0.
    List,
}

impl Op {
    pub fn code(self) -> u8 {
        match self {
            Op::Set => 1,
            Op::Resend => 2,
            Op::List => 3,
        }
    }
    pub fn from_code(c: u8) -> Option<Op> {
        Some(match c {
            1 => Op::Set,
            2 => Op::Resend,
            3 => Op::List,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// Monotonic per pairing key, never 0; the desktop accepts only `ctr > last accepted`.
    pub ctr: u16,
    pub op: Op,
    /// `Set`: crc16 of the theme slug; otherwise 0.
    pub arg: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    NotARequest,
    BadLength(usize),
    BadOp(u8),
    /// RESEND and LIST carry `arg = 0` (BEACON.md §2).
    BadArg(u16),
    ZeroCounter,
    BadMac,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestError::NotARequest => write!(f, "not a theme packet"),
            RequestError::BadLength(n) => write!(f, "packet is {n} bytes, wrong length for its kind"),
            RequestError::BadOp(o) => write!(f, "unknown op {o:#04x}"),
            RequestError::BadArg(a) => write!(f, "arg {a:#06x} on an op that takes none"),
            RequestError::ZeroCounter => write!(f, "counter 0 is never valid"),
            RequestError::BadMac => write!(f, "MAC does not verify (wrong key?)"),
        }
    }
}

/// First 4 bytes of HMAC-SHA256(key, data): the request MAC, the beacon MAC, and the theme
/// list's COMMIT MAC.
pub fn mac4(key: &[u8], data: &[u8]) -> [u8; MAC_LEN] {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(data);
    let out = m.finalize().into_bytes();
    [out[0], out[1], out[2], out[3]]
}

/// `[magic][kind=request][ctr u16 le][op][arg u16 le][mac 4]` — 11 bytes, no time.
#[cfg_attr(not(test), allow(dead_code))]
pub fn encode_request(key: &[u8], req: Request) -> [u8; REQUEST_LEN] {
    let mut b = [0u8; REQUEST_LEN];
    b[0] = MAGIC;
    b[1] = KIND_REQUEST;
    b[2..4].copy_from_slice(&req.ctr.to_le_bytes());
    b[4] = req.op.code();
    b[5..7].copy_from_slice(&req.arg.to_le_bytes());
    let m = mac4(key, &b[..7]);
    b[7..].copy_from_slice(&m);
    b
}

/// Parse and authenticate. The MAC is checked before anything inside the packet is looked
/// at, so a stranger gets one answer (`BadMac`) and no parse-error oracle. The counter check
/// against the last accepted value is the caller's job (it needs the persisted state).
pub fn decode_request(key: &[u8], b: &[u8]) -> Result<Request, RequestError> {
    if b.len() < 2 || b[0] != MAGIC || b[1] != KIND_REQUEST {
        return Err(RequestError::NotARequest);
    }
    if b.len() != REQUEST_LEN {
        return Err(RequestError::BadLength(b.len()));
    }
    if !mac_eq(&mac4(key, &b[..7]), &b[7..]) {
        return Err(RequestError::BadMac);
    }
    let op = Op::from_code(b[4]).ok_or(RequestError::BadOp(b[4]))?;
    let ctr = u16::from_le_bytes([b[2], b[3]]);
    if ctr == 0 {
        return Err(RequestError::ZeroCounter);
    }
    let arg = u16::from_le_bytes([b[5], b[6]]);
    if op != Op::Set && arg != 0 {
        return Err(RequestError::BadArg(arg));
    }
    Ok(Request { ctr, op, arg })
}

/// What the counter rule says about an authenticated request (BEACON.md §2), as a pure
/// function so it can be tested without a daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtrVerdict {
    /// `ctr > last`: accept and persist `ctr`.
    Accept,
    /// `ctr == last`: the watch's own retransmission of the request just accepted; drop silently.
    Duplicate,
    /// `ctr < last`: stale (reflash, replay, cache); drop and say so.
    Stale,
    /// The counter file could not be read: nothing is accepted until it is reset.
    Locked,
    /// `last == 65535`: the counter is used up; re-pair.
    Exhausted,
}

pub fn judge_ctr(ctr: u16, last: u16, locked: bool) -> CtrVerdict {
    if locked {
        CtrVerdict::Locked
    } else if ctr > last {
        CtrVerdict::Accept
    } else if ctr == last {
        CtrVerdict::Duplicate
    } else if last == u16::MAX {
        CtrVerdict::Exhausted
    } else {
        CtrVerdict::Stale
    }
}

/// True for any packet of ours (any kind), so the scanner can skip other 0xFFFF users.
pub fn is_ours(b: &[u8]) -> bool {
    b.len() >= 2 && b[0] == MAGIC && (b[1] == KIND_STATE || b[1] == KIND_REQUEST || b[1] == KIND_PAIR)
}

/// The `arg` of a `Set` request.
pub fn slug_id(slug: &str) -> u16 {
    crc16(slug.as_bytes())
}

// ---- files under ~/.config/themesync/ ----

fn config_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("themesync")
}

/// Write `text` to `path` atomically: a temp file created with mode 0600 from the start (the
/// key is never world-readable, not even for a moment), fsync'd, then renamed over the target,
/// and the directory fsync'd — so a power cut leaves either the old file or the new one. The
/// directory is created 0700.
fn write_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from("."));
    if !dir.is_dir() {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(&dir)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp{}", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let _ = std::fs::remove_file(&tmp); // a leftover from a crash between create and rename
    let mut f = opts.open(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?;
    if let Ok(d) = std::fs::File::open(&dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// The pairing key: `~/.config/themesync/key` (32 hex chars).
pub fn key_path() -> std::path::PathBuf {
    config_dir().join("key")
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
/// daemon restart (deploys happen often) does not lose it. It expires [`PENDING_KEY_TTL`]
/// after it was written (BEACON.md §2b) — the file's mtime is the clock.
pub fn pending_key_path() -> std::path::PathBuf {
    config_dir().join("key.pending")
}

/// The pending key and how long it has been pending, unless it has expired (then the file is
/// removed and `None` returned).
pub fn load_pending_key() -> Option<([u8; KEY_LEN], std::time::Duration)> {
    let path = pending_key_path();
    let age = std::fs::metadata(&path).ok()?.modified().ok()?.elapsed().unwrap_or_default();
    if age > PENDING_KEY_TTL {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    load_key_from(&path).map(|k| (k, age))
}

pub fn save_pending_key(k: &[u8; KEY_LEN]) -> std::io::Result<()> {
    write_atomic(&pending_key_path(), &format!("{}\n", crate::protocol::to_hex(k)))
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

/// Install `k` as the active key. The request counter is reset with it: a counter belongs to
/// one key (BEACON.md §2, "pairing resets both sides"), and a stale one would lock out the
/// freshly paired watch, which counts from 1 again.
pub fn save_key(k: &[u8; KEY_LEN]) -> std::io::Result<std::path::PathBuf> {
    let path = key_path();
    write_atomic(&path, &format!("{}\n", crate::protocol::to_hex(k)))?;
    save_ctr(0)?;
    Ok(path)
}

/// The last request counter accepted under the active key: `~/.config/themesync/ctr`.
/// Missing = 0 (nothing accepted yet, e.g. right after pairing).
pub fn ctr_path() -> std::path::PathBuf {
    config_dir().join("ctr")
}

/// `Ok(0)` when the file does not exist; `Err` when it exists but does not hold a counter
/// (a truncated or edited file) — the caller must not treat that as 0, which would reopen
/// every replay the counter has closed.
pub fn load_ctr() -> Result<u16, String> {
    match std::fs::read_to_string(ctr_path()) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(format!("{}: {e}", ctr_path().display())),
        Ok(s) => s.trim().parse().map_err(|_| format!("{} holds {:?}, not a counter", ctr_path().display(), s.trim())),
    }
}

pub fn save_ctr(ctr: u16) -> std::io::Result<()> {
    write_atomic(&ctr_path(), &format!("{ctr}\n"))
}

/// The paired watch's Bluetooth address: `~/.config/themesync/watch`. Where list pushes
/// connect (the watch's advertisement is non-scannable, so it cannot be found by UUID).
pub fn watch_path() -> std::path::PathBuf {
    config_dir().join("watch")
}

pub fn load_watch_addr() -> Option<String> {
    std::fs::read_to_string(watch_path()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn save_watch_addr(addr: &str) -> std::io::Result<()> {
    write_atomic(&watch_path(), &format!("{addr}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 16] = [7; 16];

    #[test]
    fn request_round_trip_and_mac() {
        let r = Request { ctr: 0x1234, op: Op::Set, arg: slug_id("tokyo-night") };
        let b = encode_request(&KEY, r);
        assert_eq!(b.len(), REQUEST_LEN);
        assert_eq!(&b[..2], &[MAGIC, KIND_REQUEST]);
        assert_eq!(decode_request(&KEY, &b), Ok(r));
        // wrong key, flipped op, flipped counter: all rejected
        assert_eq!(decode_request(&[8; 16], &b), Err(RequestError::BadMac));
        let mut x = b; x[4] = Op::Resend.code();
        assert_eq!(decode_request(&KEY, &x), Err(RequestError::BadMac));
        let mut y = b; y[2] ^= 1;
        assert_eq!(decode_request(&KEY, &y), Err(RequestError::BadMac));
        assert_eq!(decode_request(&KEY, &b[..10]), Err(RequestError::BadLength(10)));
        // a stranger's packet fails on the MAC whatever else is wrong with it (no parse oracle)
        let mut junk = [0u8; REQUEST_LEN]; junk[0] = MAGIC; junk[1] = KIND_REQUEST; junk[4] = 0x07;
        assert_eq!(decode_request(&KEY, &junk), Err(RequestError::BadMac));
        // a correctly signed packet with an unknown op is still refused
        let mut w = [0u8; REQUEST_LEN]; w[..2].copy_from_slice(&[MAGIC, KIND_REQUEST]); w[2] = 1; w[4] = 0x07;
        let m = mac4(&KEY, &w[..7]); w[7..].copy_from_slice(&m);
        assert_eq!(decode_request(&KEY, &w), Err(RequestError::BadOp(7)));
        // RESEND/LIST carry arg 0
        let bad_arg = encode_request(&KEY, Request { ctr: 5, op: Op::List, arg: 1 });
        assert_eq!(decode_request(&KEY, &bad_arg), Err(RequestError::BadArg(1)));
        assert_eq!(decode_request(&KEY, &[MAGIC, KIND_STATE, 0]), Err(RequestError::NotARequest));
        // a v2 (kind 0x02) request is not a request at all, whatever its length
        let mut old = [0u8; 10]; old[0] = MAGIC; old[1] = 0x02;
        assert_eq!(decode_request(&KEY, &old), Err(RequestError::NotARequest));
        // counter 0 is refused (after the MAC, which passes here)
        let z = encode_request(&KEY, Request { ctr: 0, op: Op::List, arg: 0 });
        assert_eq!(decode_request(&KEY, &z), Err(RequestError::ZeroCounter));
    }

    #[test]
    fn pending_key_confirms_pairing() {
        let old = [1u8; 16];
        let new = [2u8; 16];
        let with_new = encode_request(&new, Request { ctr: 9, op: Op::Resend, arg: 0 });
        let with_old = encode_request(&old, Request { ctr: 9, op: Op::List, arg: 0 });
        assert_eq!(decode_request_with(Some(&old), Some(&new), &with_new).unwrap().1, Verified::Pending);
        assert_eq!(decode_request_with(Some(&old), Some(&new), &with_old).unwrap().1, Verified::Active);
        assert_eq!(decode_request_with(Some(&old), None, &with_new), Err(RequestError::BadMac));
        assert_eq!(decode_request_with(None, Some(&new), &with_new).unwrap().1, Verified::Pending);
        assert_eq!(decode_request_with(None, None, &with_new), Err(RequestError::BadMac));
        let w = encode_pair_write(0x7C, &new, "");
        assert_eq!(w.len(), PAIR_WRITE_LEN);
        assert_eq!(&w[..2], &[PAIR_WRITE_TAG, 0x7C]);
        assert_eq!(&w[2..], &new);
        let named = encode_pair_write(0x7C, &new, "spawner");
        assert_eq!(&named[PAIR_WRITE_LEN..], b"spawner");
        let long = encode_pair_write(0x7C, &new, "żółw-na-biurku-xyz");
        assert!(long.len() <= PAIR_WRITE_LEN + PAIR_NAME_MAX);
        assert!(std::str::from_utf8(&long[PAIR_WRITE_LEN..]).is_ok(), "cut on a character boundary");
        assert_eq!(decode_pair_adv(&[MAGIC, KIND_PAIR, 0x78, 0x56, 0x34, 0x12]), Some(0x1234_5678));
        assert_eq!(decode_pair_adv(&[MAGIC, KIND_PAIR, 0x78, 0x56, 0x34]), None);
        assert_eq!(decode_pair_adv(&[MAGIC, KIND_REQUEST, 0, 0, 0, 0]), None);
        assert!(is_ours(&[MAGIC, KIND_PAIR, 0, 0, 0, 0]));
    }

    #[test]
    fn state_beacon_is_theme_plus_mac() {
        let theme = [2u8, 1, 10, 20, 30, 2, 40, 50, 60];
        let s = encode_state(&KEY, &theme, 0x0102, None);
        assert_eq!(&s[..2], &[MAGIC, KIND_STATE]);
        assert_eq!(&s[2..2 + theme.len()], &theme);
        assert_eq!(s.len(), 2 + theme.len() + 4 + 2 + MAC_LEN);
        assert!(is_ours(&s));
        assert!(!is_ours(&[0x00, 0x01]));
        assert_eq!(decode_state(&KEY, &s), Ok((&theme[..], 0x0102, None)));
        assert_eq!(decode_state(&[8; 16], &s), Err(RequestError::BadMac));
        let mut t = s.clone(); t[4] ^= 1; // a colour byte
        assert_eq!(decode_state(&KEY, &t), Err(RequestError::BadMac));
        let mut u = s.clone(); u[2 + theme.len() + 2] ^= 1; // the echo is under the mac too
        assert_eq!(decode_state(&KEY, &u), Err(RequestError::BadMac));
        assert_eq!(decode_state(&KEY, &s[..s.len() - 1]), Err(RequestError::BadLength(s.len() - 1)));
        // the parser still reads the theme and reports the meta records
        let d = protocol::decode_v2(&s[2..]).unwrap();
        assert_eq!(d.colors.len(), 2);
        assert_eq!(d.echo, Some(0x0102));
        assert_eq!(d.mac, Some(mac4(&KEY, &s[..s.len() - 6])));
        // a different echo changes the mac but not the theme bytes the watch hashes
        let s2 = encode_state(&KEY, &theme, 0x0103, None);
        assert_eq!(protocol::v2_theme_end(&s2[2..]), theme.len());
        assert_ne!(s2, s);
        // with the time record: same theme bytes, time reported, mac covers the time
        let t = [26u8, 8, 31, 13, 5, 9, 1];
        let timed = encode_state(&KEY, &theme, 0x0102, Some(&t));
        assert_eq!(timed.len(), s.len() + 9);
        assert_eq!(protocol::v2_theme_end(&timed[2..]), theme.len());
        assert_eq!(decode_state(&KEY, &timed), Ok((&theme[..], 0x0102, Some(t))));
        let mut flip = timed.clone();
        flip[2 + theme.len() + 4 + 3] ^= 1; // a byte inside the time record
        assert_eq!(decode_state(&KEY, &flip), Err(RequestError::BadMac));
        // a mangled time length is a framing error, not a mac check
        let mut short = vec![MAGIC, KIND_STATE];
        short.extend_from_slice(&theme);
        protocol::v2_append_echo(&mut short, 0x0102);
        short.extend_from_slice(&[protocol::V2_TAG_TIME, 6, 26, 8, 31, 13, 5, 9]);
        let m = mac4(&KEY, &short);
        protocol::v2_append_mac(&mut short, &m);
        assert_eq!(decode_state(&KEY, &short), Err(RequestError::BadLength(short.len())));
        // local_time_record is sane whatever the wall clock says
        let now = local_time_record();
        assert!((1..=12).contains(&now[1]) && (1..=31).contains(&now[2]));
        assert!(now[3] < 24 && now[4] < 60 && now[5] < 60 && now[6] < 7);
    }

    /// Interop anchors shared with the watch firmware (BEACON.md §4): key 00 01 .. 0f.
    #[test]
    fn interop_vectors() {
        let key: Vec<u8> = (0u8..16).collect();
        let set = encode_request(&key, Request { ctr: 1, op: Op::Set, arg: slug_id("tokyo-night") });
        assert_eq!(slug_id("tokyo-night"), 0xAAE5);
        assert_eq!(set.to_vec(), vec![0x54, 0x03, 0x01, 0x00, 0x01, 0xe5, 0xaa, 0x3f, 0x0e, 0xc9, 0x9b]);
        let resend = encode_request(&key, Request { ctr: 2, op: Op::Resend, arg: 0 });
        assert_eq!(resend.to_vec(), vec![0x54, 0x03, 0x02, 0x00, 0x02, 0x00, 0x00, 0x0c, 0xc2, 0xe4, 0xfc]);
        let list = encode_request(&key, Request { ctr: 3, op: Op::List, arg: 0 });
        assert_eq!(list.to_vec(), vec![0x54, 0x03, 0x03, 0x00, 0x03, 0x00, 0x00, 0xea, 0x01, 0xce, 0xc7]);
        assert_eq!(decode_request(&key, &list).unwrap().op, Op::List);
        assert_eq!(Op::from_code(4), None);
        assert_eq!(Op::from_code(0), None);

        let theme = protocol::from_hex("02011020300240506040046e6f7264410100").unwrap();
        let beacon = encode_state(&key, &theme, 0, None);
        assert_eq!(protocol::to_hex(&beacon), "540102011020300240506040046e6f72644101004302000042044cdcbd41");
        let beacon1 = encode_state(&key, &theme, 1, None);
        assert_eq!(protocol::to_hex(&beacon1), "540102011020300240506040046e6f7264410100430201004204caa53127");
        assert_eq!(beacon.len(), 30);
        assert_eq!(decode_state(&key, &beacon1).unwrap().1, 1);
        // 2026-08-31 13:05:09, a Monday (BEACON.md §1's time vector)
        let timed = encode_state(&key, &theme, 1, Some(&[26, 8, 31, 13, 5, 9, 1]));
        assert_eq!(protocol::to_hex(&timed), "540102011020300240506040046e6f72644101004302010044071a081f0d0509014204e28aefc0");
        assert_eq!(decode_state(&key, &timed).unwrap().2, Some([26, 8, 31, 13, 5, 9, 1]));
        assert_eq!(crc16(&theme), 0xD5E2);
    }

    #[test]
    fn counter_rule() {
        use CtrVerdict::*;
        assert_eq!(judge_ctr(1, 0, false), Accept);
        assert_eq!(judge_ctr(500, 3, false), Accept); // gaps are fine (a crash on the watch skips values)
        assert_eq!(judge_ctr(7, 7, false), Duplicate);
        assert_eq!(judge_ctr(6, 7, false), Stale);
        assert_eq!(judge_ctr(1, 4000, false), Stale); // a reflashed watch
        assert_eq!(judge_ctr(9, 9, true), Locked);
        assert_eq!(judge_ctr(65535, 0, true), Locked);
        assert_eq!(judge_ctr(1, u16::MAX, false), Exhausted);
        assert_eq!(judge_ctr(u16::MAX, u16::MAX, false), Duplicate);
        // reset-counter reopens acceptance; pairing sets last = the confirming request's ctr
        assert_eq!(judge_ctr(1, 0, false), Accept);
    }

    /// Every decoder, fed structured junk: truncations and extensions of every valid
    /// encoder output plus a deterministic pseudo-random corpus. Nothing may panic, and
    /// `v2_theme_end` may never point past the end.
    #[test]
    fn hostile_input_never_panics() {
        let key: Vec<u8> = (0u8..16).collect();
        let theme = protocol::from_hex("02011020300240506040046e6f7264410100").unwrap();
        let mut corpus: Vec<Vec<u8>> = Vec::new();
        let valid = [
            encode_request(&key, Request { ctr: 1, op: Op::Set, arg: 0xAAE5 }).to_vec(),
            encode_state(&key, &theme, 1, None),
            encode_state(&key, &theme, 1, Some(&[26, 8, 31, 13, 5, 9, 1])),
            theme.clone(),
            crate::themelist::encode_list(&[theme.clone()]).unwrap(),
            crate::themelist::ListStatus { version: crate::themelist::STATUS_VERSION, count: 1, crc: 1, on_sd: false, loaded: true, nonce: 5 }.encode().to_vec(),
        ];
        for v in &valid {
            for n in 0..=v.len() {
                corpus.push(v[..n].to_vec());
                let mut ext = v.clone();
                ext.extend(std::iter::repeat(0x42).take(n));
                corpus.push(ext);
            }
            for i in 0..v.len() {
                for bit in [0x01, 0x80, 0xFF] {
                    let mut x = v.clone();
                    x[i] ^= bit;
                    corpus.push(x);
                }
            }
        }
        let mut seed: u32 = 0x1234_5678;
        for len in 0..48 {
            for _ in 0..64 {
                let mut v = Vec::with_capacity(len);
                for _ in 0..len {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    v.push((seed >> 24) as u8);
                }
                // bias towards our magic and record tags so the parsers get past the first byte
                if len > 1 && seed & 1 == 0 { v[0] = MAGIC; v[1] = if seed & 2 == 0 { KIND_STATE } else { KIND_REQUEST }; }
                if len > 0 && seed & 4 == 0 { v[0] = 2; }
                corpus.push(v);
            }
        }
        for b in &corpus {
            let _ = decode_request(&key, b);
            let _ = decode_request_with(Some(&[1; 16]), Some(&[2; 16]), b);
            let _ = decode_state(&key, b);
            let _ = protocol::decode_v2(b);
            assert!(protocol::v2_theme_end(b) <= b.len());
            let _ = protocol::decode_theme(b);
            let _ = crate::themelist::decode_list(b);
            let _ = crate::themelist::ListStatus::decode(b);
            let _ = check_theme(b);
            let _ = is_ours(b);
        }
        assert!(corpus.len() > 3000);
    }

    /// The apply-rule contract of a beacon (BEACON.md §1): the fixed tail, the mac last,
    /// the theme bytes stable across echoes.
    #[test]
    fn state_beacon_conformance() {
        let key: Vec<u8> = (0u8..16).collect();
        let theme = protocol::from_hex("02011020300240506040046e6f7264410100").unwrap();
        let good = encode_state(&key, &theme, 7, None);
        // no mac record at all
        let mut no_mac = vec![MAGIC, KIND_STATE];
        no_mac.extend_from_slice(&theme);
        protocol::v2_append_echo(&mut no_mac, 7);
        assert_eq!(decode_state(&key, &no_mac), Err(RequestError::BadLength(no_mac.len())));
        // a mac record in the middle of the colour records: the theme ends there, the tail is wrong
        let mut mid = vec![MAGIC, KIND_STATE, 2, 1, 0x10, 0x20, 0x30];
        protocol::v2_append_mac(&mut mid, &[0; 4]);
        mid.extend_from_slice(&[2, 0x40, 0x50, 0x60]);
        protocol::v2_append_echo(&mut mid, 7);
        let m = mac4(&key, &mid);
        protocol::v2_append_mac(&mut mid, &m);
        assert!(decode_state(&key, &mid).is_err());
        // echo after the mac
        let mut swapped = vec![MAGIC, KIND_STATE];
        swapped.extend_from_slice(&theme);
        let m = mac4(&key, &swapped);
        protocol::v2_append_mac(&mut swapped, &m);
        protocol::v2_append_echo(&mut swapped, 7);
        assert!(decode_state(&key, &swapped).is_err());
        // a duplicate echo
        let mut dup = vec![MAGIC, KIND_STATE];
        dup.extend_from_slice(&theme);
        protocol::v2_append_echo(&mut dup, 7);
        protocol::v2_append_echo(&mut dup, 8);
        let m = mac4(&key, &dup);
        protocol::v2_append_mac(&mut dup, &m);
        assert!(decode_state(&key, &dup).is_err());
        // time in the wrong place: before the echo
        let mut early = vec![MAGIC, KIND_STATE];
        early.extend_from_slice(&theme);
        protocol::v2_append_time(&mut early, &[26, 8, 31, 13, 5, 9, 1]);
        protocol::v2_append_echo(&mut early, 7);
        let m = mac4(&key, &early);
        protocol::v2_append_mac(&mut early, &m);
        assert!(decode_state(&key, &early).is_err());
        // a duplicate time record
        let mut twice = vec![MAGIC, KIND_STATE];
        twice.extend_from_slice(&theme);
        protocol::v2_append_echo(&mut twice, 7);
        protocol::v2_append_time(&mut twice, &[26, 8, 31, 13, 5, 9, 1]);
        protocol::v2_append_time(&mut twice, &[26, 8, 31, 13, 5, 9, 1]);
        let m = mac4(&key, &twice);
        protocol::v2_append_mac(&mut twice, &m);
        assert!(decode_state(&key, &twice).is_err());
        // the theme bytes the watch hashes move with neither the echo nor the time
        let (t7, e7, _) = decode_state(&key, &good).unwrap();
        let other = encode_state(&key, &theme, 8, Some(&[26, 8, 31, 13, 5, 9, 2]));
        let (t8, e8, _) = decode_state(&key, &other).unwrap();
        assert_eq!((t7, e7), (&theme[..], 7));
        assert_eq!((t8, e8), (&theme[..], 8));
        assert_eq!(crc16(t7), crc16(t8));
        // the largest legal theme (14 roles, a 31-byte name, flags) fits the beacon budget
        let mut big = vec![2u8];
        for id in 1..=14u8 { big.extend_from_slice(&[id, 1, 2, 3]); }
        big.push(protocol::V2_TAG_NAME); big.push(31); big.extend_from_slice(&[b'x'; 31]);
        big.extend_from_slice(&[protocol::V2_TAG_FLAGS, 1, 0]);
        assert_eq!(check_theme(&big), Ok(()));
        let wire = encode_state(&key, &big, u16::MAX, Some(&local_time_record()));
        assert!(wire.len() <= MAX_THEME_LEN + 2 + 4 + 9 + 6);
        assert!(wire.len() + 4 <= 254, "manufacturer data + AD header must fit one extended advertising PDU");
        assert_eq!(decode_state(&key, &wire).unwrap().0, &big[..]);
    }

    #[test]
    fn theme_check_guards_the_beacon() {
        let theme = protocol::from_hex("02011020300240506040046e6f7264410100").unwrap();
        assert_eq!(check_theme(&theme), Ok(()));
        let mut with_echo = theme.clone();
        protocol::v2_append_echo(&mut with_echo, 1);
        assert!(check_theme(&with_echo).unwrap_err().contains("meta record"));
        assert!(check_theme(&[0u8; MAX_THEME_LEN + 1]).unwrap_err().contains("bytes"));
        assert!(check_theme(&[0xff, 0xff, 0xff]).unwrap_err().contains("decode"));
    }

    #[test]
    fn atomic_write_creates_0600_and_replaces() {
        let dir = std::env::temp_dir().join(format!("themesync-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join("ctr");
        write_atomic(&path, "1\n").unwrap();
        write_atomic(&path, "2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "2\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert!(std::fs::read_dir(path.parent().unwrap()).unwrap().count() == 1, "no temp file left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
