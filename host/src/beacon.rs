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
pub const REQUEST_LEN: usize = 11;
pub const MAC_LEN: usize = 4;
pub const KEY_LEN: usize = 16;
/// First byte of the `…0005` write: `[0x01][code][key 16]` (protocol/BEACON.md §2b).
pub const PAIR_WRITE_TAG: u8 = 0x01;
pub const PAIR_WRITE_LEN: usize = 2 + KEY_LEN;
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

/// `[magic][kind=state][theme v2 TLV][0x43 2 echo][0x42 4 mac]` — `echo` is the last request
/// counter accepted (the watch's ack number), the mac covers everything before its record
/// (BEACON.md §1). `theme` must be a v2 packet without meta records.
pub fn encode_state(key: &[u8], theme: &[u8], echo: u16) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 + theme.len() + 4 + 2 + MAC_LEN);
    v.extend_from_slice(&[MAGIC, KIND_STATE]);
    v.extend_from_slice(theme);
    protocol::v2_append_echo(&mut v, echo);
    let mac = mac4(key, &v);
    protocol::v2_append_mac(&mut v, &mac);
    v
}

/// The receiver's view of a state beacon: the theme bytes (what the apply rule hashes) and
/// the echo, if the packet is ours and the mac verifies.
#[cfg_attr(not(test), allow(dead_code))]
pub fn decode_state<'a>(key: &[u8], b: &'a [u8]) -> Result<(&'a [u8], u16), RequestError> {
    if b.len() < 2 || b[0] != MAGIC || b[1] != KIND_STATE {
        return Err(RequestError::NotARequest);
    }
    let theme = &b[2..];
    let end = protocol::v2_theme_end(theme);
    let meta = &theme[end..];
    // fixed tail: [0x43 2 echo][0x42 4 mac]
    if meta.len() != 4 + 2 + MAC_LEN || meta[0] != protocol::V2_TAG_ECHO || meta[1] != 2 || meta[4] != protocol::V2_TAG_MAC || meta[5] as usize != MAC_LEN {
        return Err(RequestError::BadLength(b.len()));
    }
    if !mac_eq(&mac4(key, &b[..b.len() - MAC_LEN - 2]), &meta[6..]) {
        return Err(RequestError::BadMac);
    }
    Ok((&theme[..end], u16::from_le_bytes([meta[2], meta[3]])))
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

/// True for any packet of ours (either kind), so the scanner can skip other 0xFFFF users.
pub fn is_ours(b: &[u8]) -> bool {
    b.len() >= 2 && b[0] == MAGIC && (b[1] == KIND_STATE || b[1] == KIND_REQUEST)
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
        let w = encode_pair_write(0x7C, &new);
        assert_eq!(w.len(), PAIR_WRITE_LEN);
        assert_eq!(&w[..2], &[PAIR_WRITE_TAG, 0x7C]);
        assert_eq!(&w[2..], &new);
    }

    #[test]
    fn state_beacon_is_theme_plus_mac() {
        let theme = [2u8, 1, 10, 20, 30, 2, 40, 50, 60];
        let s = encode_state(&KEY, &theme, 0x0102);
        assert_eq!(&s[..2], &[MAGIC, KIND_STATE]);
        assert_eq!(&s[2..2 + theme.len()], &theme);
        assert_eq!(s.len(), 2 + theme.len() + 4 + 2 + MAC_LEN);
        assert!(is_ours(&s));
        assert!(!is_ours(&[0x00, 0x01]));
        assert_eq!(decode_state(&KEY, &s), Ok((&theme[..], 0x0102)));
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
        let s2 = encode_state(&KEY, &theme, 0x0103);
        assert_eq!(protocol::v2_theme_end(&s2[2..]), theme.len());
        assert_ne!(s2, s);
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
        let beacon = encode_state(&key, &theme, 0);
        assert_eq!(protocol::to_hex(&beacon), "540102011020300240506040046e6f72644101004302000042044cdcbd41");
        let beacon1 = encode_state(&key, &theme, 1);
        assert_eq!(protocol::to_hex(&beacon1), "540102011020300240506040046e6f7264410100430201004204caa53127");
        assert_eq!(beacon.len(), 30);
        assert_eq!(decode_state(&key, &beacon1).unwrap().1, 1);
        assert_eq!(crc16(&theme), 0xD5E2);
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
