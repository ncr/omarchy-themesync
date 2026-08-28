# Code review, 2026-08-28 — before a release to Omarchy users

Three independent Opus reviews, read-only, against commit `230ed07` (protocol v3, beacon at
30 ms). Lenses: (1) the running daemon's correctness and robustness, (2) what breaks on a
stranger's machine and what a release needs, (3) spec conformance and decoder safety. The
three full reports follow the synthesis verbatim. The 2026-08-27 protocol review is in
`review-2026-08-27.md`.

## Verdict

No memory-safety or injection problem anywhere: every decoder is panic-free on hostile
bytes, the C decoder has no out-of-bounds read, all §4 interop vectors recompute
independently, `omarchy-theme-set` gets a directory name chosen by crc match with no shell,
and a BLE neighbour without the key is dropped before any side effect. The protocol and the
codecs are sound.

What is not ready is everything around the daemon: install, the unit, the hook, first-run,
diagnostics, and a handful of daemon lifecycle bugs (a beacon that goes dark, two daemons at
once, a hung GATT push that locks the list forever). Two protocol-level points remain open
and need the watch side.

## Findings that appear in two or three reports (highest confidence)

| # | finding | reports | fix size |
|---|---|---|---|
| 1 | The unit starts outside the graphical session (`WantedBy=default.target`) and without Omarchy's PATH; `omarchy-theme-set` then fails or half-applies. | 1#1, 2 B4 | small: unit + resolve `$OMARCHY_PATH/bin` explicitly |
| 2 | No single-instance guard; `serve_socket` unlinks and rebinds, a failed bind kills IPC silently. | 1#2, 1#3, 2 S4 | small: flock + treat socket death as fatal |
| 3 | `set_beacon` drops the old advertisement before the new one registers; `Request::Push` is unvalidated; a persistent failure = dark beacon, `status` still says "on". | 1#5, 1#12, 3#7, 3#8 | small |
| 4 | No adapter / rfkill / BT 4.x → 3 s restart loop forever, adapter re-powered against the user's wishes. | 1#6, 2 B5 | medium: backoff + `SupportedSecondaryChannels` check + unit conditions |
| 5 | The ARQ retransmit is logged as "rejected" and counted in `status`; the desync signal is noise. | 1#7, 2 S3 | trivial: `==` vs `<` |
| 6 | `pair` outside the daemon saves the key immediately (no confirmation) and leaves the old counter → freshly paired watch locked out. | 1#8, 3#6 | small |
| 7 | Key file world-readable for a window (`write_atomic` chmods after writing); `key`/`key.pending` share a temp name; corrupt `ctr` silently → 0. | 1#13, 2 S7, 3#10, 3#11 | small |
| 8 | IPC socket `/tmp` fallback uses `$UID` (never exported) → squattable path; any local process can `PairPending`/`ResetCounter`/`Push`. | 1#14, 2 S8 | small: require `XDG_RUNTIME_DIR`, document the trust model |
| 9 | Unauthenticated packets produce one journal line each (MAC checked last; dedup remembers one packet). | 1#15, 3#3 | small: MAC first, rate-limit |

## Single-report findings that still matter before release

- **2 B1/B2/B3** — no install path; the repo hook silently no-ops (`~/.cargo/bin` not on the session PATH); BlueZ `Experimental = true` is undocumented and its absence degrades to "swipes work sometimes". The whole first-run story needs `themesync install` / `uninstall` / `doctor`, and a README written for a stranger (2 S1, S2).
- **1#4** — a hung GATT operation holds the list-push semaphore forever; no timeout anywhere in `push_list` / `gatt_push`.
- **1#9** — with no hook installed, a watch SET changes the desktop but the beacon keeps the old theme and the watch bounces back; `current_theme()` at startup is never retried.
- **2 S5** — an unpaired install runs continuous active discovery for nothing.
- **3#4** — a slug longer than 31 bytes can never match its own crc → one GATT list push per gesture, forever. **3#5** — `push-list --direct` skips the crc-collision check.
- **2 S10** — no `LICENSE`, no `repository`/`rust-version`, no tag, no CI.

## Protocol-level points (need the watch session)

- **3#1** — the list COMMIT MAC is a pure function of the list bytes, so a recorded push replays byte-for-byte; the v3 note claiming it closes review item #9 is wrong. Fix: a nonce in the READ status covered by the MAC (firmware change), or correct the claim in §3a.
- **3#2** — beacon replay: any recorded beacon stays valid forever under the same key; alternating two recorded beacons drives NVS writes on the watch, which contradicts §1's flash-budget claim. Cheapest: rate-limit NVS writes on the watch (≥ 1/minute) and say so in §1.
- **3#16/#17** — `protocol/THEME_PROTOCOL.md` and everything under `watch/` are Theme Protocol v1; the v2 TLV format is documented only in a comment in `protocol.rs`, and no C twin of the v3 decoders exists in this repo. Either port or mark historical.
- **3#18** — the desktop's pending key never expires (spec says 120 s).

## Suggested order

1. **Daemon correctness** (one sitting, all small): rows 2, 3, 5, 6, 7, 8, 9 above, plus 1#4 timeouts, 3#4 slug clamp, 3#5 collision bail, 1#9 re-read after SET.
2. **Deployment**: unit (row 1, row 4), `install`/`uninstall`/`doctor`, no discovery without a key, `status --json` + human summary, README rewrite, `LICENSE` + Cargo metadata, PKGBUILD + `.install`, CI, tag `v0.1.0`.
3. **Protocol with the watch session**: list nonce, NVS rate limit, pending-key expiry, `watch/` cleanup.
4. Tests from report 3 ("five tests to add"), the counter-rule unit test first.

---

## Review 1: `themesync daemon` — correctness and robustness (Opus, 2026-08-28)

# Should fix before release

### 1. `omarchy-theme-set` is resolved through `$PATH`, which the user unit does not have at login
`host/src/omarchy.rs:136` (`Command::new("omarchy-theme-set")`), `systemd/themesync.service:29-41`.

`/usr/share/omarchy/bin` is **not** in any `environment.d` or `profile.d` drop-in on this machine — it reaches the user manager only via the session's `import-environment`. The unit is `WantedBy=default.target`, so it starts when the user manager starts (PAM session), *before* the compositor imports that environment. `After=graphical-session.target` does not delay it: that target is not part of the same transaction, so the ordering is a no-op.

Failure scenario: fresh install, user enables the service, logs in. The daemon runs with the systemd default `PATH`. Beacon and scan work, but every watch SET fails for the whole session with `SET <name> failed: running omarchy-theme-set (is Omarchy on PATH?)`, and `omarchy-theme-color` is silently absent too. The author never sees this because the dev loop restarts the service *after* login.

Fix: resolve the binaries explicitly — `$OMARCHY_PATH/bin/omarchy-theme-set`, falling back to `/usr/share/omarchy/bin` then `PATH` — and/or ship the unit as `WantedBy=graphical-session.target` with `PartOf=graphical-session.target`.

### 2. No single-instance guard; a second daemon is invisible and the first one loses its socket
`host/src/daemon.rs:74` (`let _ = std::fs::remove_file(&path)`), `:78` (bind), `:289-293` (the failure is only logged).

`serve_socket` unlinks the existing socket unconditionally and binds a new one. Two daemons (service + `themesync daemon` in a terminal) advertise two beacons, both accept the same requests, both write `ctr`, both run `omarchy-theme-set`. Only the newest receives IPC. If the bind *fails*, the spawn logs `socket server died: …` once and the daemon keeps running as a beacon no theme change can refresh, while `themesync status` reports "daemon: not running".

Fix: exclusive `flock` on `~/.config/themesync/lock` (or ping the existing socket before unlinking) and exit with a clear message; treat `serve_socket` returning at all as fatal (`Restart=always` then does the right thing).

### 3. `listener.accept()?` and unbounded/untimed `read_line` — one bad client kills IPC for good
`host/src/daemon.rs:81`, `:86`.

`accept()` errors propagate out of `serve_socket`, ending the socket server permanently. `ECONNABORTED`/`EMFILE` are transient. The per-connection task does an unbounded, untimed `read_line`: a client that never writes leaks a task and an fd forever; an endless line without `\n` allocates without bound.

Fix: `match` on the accept error and continue; wrap the read in `tokio::time::timeout` and cap it with `AsyncBufReadExt::take(4096)`.

### 4. A hung GATT operation closes the list-push gate permanently
`host/src/daemon.rs:185-192` (`gate.try_acquire_owned`), `:193-252`; `host/src/transport/ble.rs:387-433`.

`push_list` has no timeout: `connect()`, `discover_services()`, `read()`, `write()` can block indefinitely. The permit is owned by the spawned task, so one hang means every later trigger is answered `another push is in progress; skipped` for the daemon's life, and the `done` oneshot never fires.

Fix: wrap the whole `step` future in `tokio::time::timeout(60 s)`; same in `gatt_push` (`daemon.rs:139-166`), which otherwise leaks one hung task per theme change when `--no-gatt` is not passed.

### 5. The beacon can go dark and `status` will still say it is on
`host/src/transport/adv.rs:41-54`, `host/src/daemon.rs:263-275`, `:369-381`.

`set_beacon` does `self.handle.take()` — dropping the current advertisement — *before* it knows the new registration succeeds. If both attempts fail, nothing is on the air until the next 60 s tick, which re-runs the same call: a payload BlueZ rejects permanently → dark forever; adapter powered off → `set_powered(true)` is only called in `Radio::open()` at startup, never repeated for the advertising radio. `Request::Status` reports `beacon on` purely from `theme_wire.is_empty()`.

Fix: keep the old handle until the new registration returns `Ok`; on repeated failure re-run `Radio::open()`; track the last successful registration and report it in `status`.

### 6. No adapter at start → a 3 s systemd restart loop, forever
`host/src/daemon.rs:277-278`, `:298-311`; `systemd/themesync.service:36-38`.

`Radio::open().await?` aborts `run()` when BlueZ is not up. With `Restart=always`/`RestartSec=3` the unit restarts every ~3 s indefinitely. The scan supervisor logs `scan: …; retrying in 3 s` every three seconds, 28 800 journal lines a day, no backoff.

Fix: wait-and-retry inside the daemon with exponential backoff, log once then rate-limit, instead of exiting.

### 7. `ctr_rejected` counts normal ARQ retransmissions, so the "desync is loud" signal is pure noise
`host/src/daemon.rs:468-479`.

BlueZ delivers each advertising event while a request is on the air, so the *same* counter arrives again after acceptance (verified live: `Set #1109` then `Set #1109 rejected: already seen`). One "rejected" line and one `ctr_rejected += 1` per button press in healthy operation.

Fix: `req.ctr == ctr_last` is the expected duplicate (log at most once, do not count); only `req.ctr < ctr_last` is desync and deserves the counter and the `themesync pair` hint.

### 8. Pairing outside the daemon writes a new key but leaves the old counter → the freshly paired watch is locked out
`host/src/main.rs:434-437` (`pair --no-watch`) and `:447-450` (no daemon running), vs. `daemon.rs:451-453` which does reset it.

Scenario: ctr = 4000, user re-pairs before enabling the service. New key both sides, watch counts from 1, daemon loads `ctr = 4000`, rejects every request.

Fix: `beacon::save_ctr(0)` next to every `save_key` (a helper that writes both), and/or store the counter alongside the key.

### 9. Nothing recovers the beacon if the theme-set hook is missing, and the startup theme is read only once
`host/src/daemon.rs:314-320`, `:120-136`, `:352-358`.

The beacon is refreshed *only* by an IPC `Sync`/`Push`, i.e. only if the hook is installed. Hook missing: a watch SET runs `omarchy-theme-set`, desktop changes, beacon keeps the *old* theme; the watch gives up after 10 s and repaints back to the stale beacon — the theme visibly bounces back. If `current_theme()` fails at startup, `theme_v2` stays empty and is never retried.

Fix: the actor signals the main loop after a successful `set_theme` so it re-reads the theme; retry `current_theme()` on the republish tick while empty; warn at startup when the hook file is absent.

# Nice to have

### 10. Blocking `std::process::Command` inside the async select loop
`daemon.rs:50-55` (`current_theme` → `omarchy.rs:165` runs `omarchy-theme-color` synchronously), called at `:314`, `:405`, `:459`. Blocks the main loop; `rrx` (capacity 32, `try_send` at `:303`) silently drops requests if it fills. Use `spawn_blocking` like `list_push` does; consider a timeout/kill for `omarchy-theme-set` in the actor thread.

### 11. `watched` bookkeeping in the scanner can duplicate or orphan per-device tasks
`host/src/transport/adv.rs:94-127`. `DeviceRemoved` removes the address but does not stop the task; interleaving `DeviceRemoved(A)` → `DeviceAdded(A)` → old stream ends (removes the *new* entry) leaves a live task with no entry → duplicates accumulate. Store an `AbortHandle` per address and abort on `DeviceRemoved`.

### 12. `Request::Push` puts unvalidated bytes straight into the advertisement
`daemon.rs:417-428`. `from_hex` strips non-hex silently; no parse/length check. Validate with `protocol::decode_v2` + `v2_theme_end(v2) == v2.len()` and cap the length.

### 13. State-file handling: a permissions window and silent corruption recovery
`host/src/beacon.rs:216-228`, `:282-284`. `write_atomic` writes with the umask then chmods 0600 — key world-readable for a window; use `OpenOptions::mode(0o600).create_new(true)`. No `fsync`. `load_ctr` maps unparsable content to `0` silently — log loudly, prefer the in-memory value.

### 14. IPC socket surface
`host/src/transport/ipc.rs:207-220`, `daemon.rs:78`. Socket mode `0777 & ~umask`; fine under `XDG_RUNTIME_DIR` (0700) but chmod 0600 is one line. `$UID` is not exported, so the fallback is `/tmp/themesync-<user>.sock` in a sticky world-writable dir — squattable. Any client reaching the socket can `PairPending`, `ResetCounter`, `Push` — one README sentence.

### 15. Unbounded log on hostile/foreign packets
`daemon.rs:436-442`. Dedup remembers only the last packet; rate-limit (count per minute, one summary line).

### 16. `Reply::ok("sent")` when there is no pairing key
`daemon.rs:405-413`. `sync` claims success with no key. Return an error.

# Checked and found sound
- Packet parsing from the air is panic-free (`decode_request`, `decode_state`, `v2_theme_end`, `decode_v2`); the existing `expect`s are unreachable.
- No shell injection: `Command::new(...).arg(name)`, name is a directory name from `list_themes()` selected by crc match.
- Counter/replay logic matches the spec; echo before `omarchy-theme-set`; no read of BlueZ's cached `ManufacturerData`.
- `spawn_actor` newest-wins coalescing is correct; a failed `set_theme` cannot kill the thread.
- The select loop cannot panic on "all branches disabled"; all branches cancel-safe.
- Two `bluer` sessions + `btleplug` coexist; scan supervisor drops the whole `Radio` before re-opening.
- The hook cannot re-enter destructively (`sync --async` re-execs detached).
- Malformed JSON on the socket → `bad request`; `PairPending` rejects non-16-byte keys.
- The theme-list transfer matches §3a (MTU framing, COMMIT MAC, retry from BEGIN, read-back verification, crc-collision refusal).

---

## Review 2: release readiness — someone else's machine (Opus, 2026-08-28)

Verified on this box: `bluez 5.87-2`, `rustc 1.98`, Omarchy 4.0.1 at `/usr/share/omarchy` (`~/.local/share/omarchy` is a symlink to it), `cargo test` = 40 passed.

## Blockers

**B1 — There is no install path; the documented one fails on the first command.** README says `systemctl --user enable --now themesync`, but nothing installs `themesync.service` into `~/.config/systemd/user/` (that step is a comment in `systemd/themesync.service:5-6`). A stranger gets `Unit themesync.service does not exist`. *Fix:* `themesync install` (hook + unit + preflight + "now run pair") and `themesync uninstall`; package the unit at `/usr/lib/systemd/user/` with `ExecStart=/usr/bin/themesync daemon --no-gatt` (`%h/.cargo/bin/themesync` at `:22` only exists for a cargo-install).

**B2 — The hook checked into the repo silently does nothing on a normal install.** `hooks/theme-set.d/themesync:12-14`: `command -v themesync || exit 0`, and `~/.cargo/bin` is not on the Omarchy session PATH (verified with `systemctl --user show-environment`). Total silence. The author is unaffected only because the installed hook is the `install-hook`-generated one with the absolute path baked in. *Fix:* delete the repo copy and make `themesync install-hook` the only route, or search `/usr/bin/themesync:$HOME/.cargo/bin/themesync` explicitly and report via `notify-send`/`systemd-cat` when nothing is found.

**B3 — BlueZ `Experimental = true` is an undocumented hard requirement for watch→desktop.** Nothing in the repo mentions it. Stock Arch `bluez` ships `#Experimental = false`; `org.bluez.AdvertisementMonitorManager1` is `[experimental]`. Degradation on a stock box: one journal line (`adv.rs:86`), beacon fine (secondary channel / intervals are not experimental), but requests invisible until the kernel's periodic scan restart — 0–2 s latency, short requests lost. Swipes work sometimes; only the journal hints why. *Fix:* preflight in the daemon and in `doctor`/`status` with a loud, actionable line; the installer offers to patch (`/etc/bluetooth/main.conf` only, no `conf.d`; sudo + backup + `systemctl restart bluetooth`) or prints the lines to paste; README Requirements.

**B4 — The systemd unit ignores Omarchy's conventions and starts outside the graphical session.** `systemd/themesync.service:19-27`: `WantedBy=default.target`, `Restart=always`. Omarchy's own units (`/usr/share/omarchy/default/systemd/user/bt-agent.service`, `omarchy-fcitx5.service`) use `WantedBy=graphical-session.target` + `PartOf=graphical-session.target` + `ConditionEnvironment=WAYLAND_DISPLAY` and their comments spell out this trap. The daemon starts on SSH logins and at boot before uwsm imports the environment; `omarchy-theme-set` then runs without `WAYLAND_DISPLAY` and half-applies. *Fix:*
```ini
[Unit]
After=bluetooth.target graphical-session.target
PartOf=graphical-session.target
Wants=bluetooth.target
ConditionEnvironment=WAYLAND_DISPLAY
ConditionPathIsDirectory=/sys/class/bluetooth
[Service]
ExecCondition=/usr/bin/systemctl is-active --quiet bluetooth.service
Restart=on-failure
RestartSec=5
[Install]
WantedBy=graphical-session.target
```

**B5 — No adapter / rfkill'd / BT 4.x adapter = infinite 3-second restart loop, no explanation.** `Radio::open()` (`adv.rs:24-29`) fails → `run` returns `Err` → `Restart=always` forever, re-powering the adapter against the user's wishes. On BT 4.x, `set_beacon` with `SecondaryChannel::OneM` is rejected twice a minute; the 78–85-byte payload cannot fit legacy advertising at all — the feature is impossible, not degraded. *Fix:* the conditions of B4 for "no hardware"; do not force `set_powered(true)` (or once, logged); read `LEAdvertisingManager1.SupportedSecondaryChannels` at startup and exit with a one-line explanation naming the BLE-5 requirement.

## Should fix before release

**S1 — README is a lab notebook, not a user document, and parts are stale.** A 60-line "Status (2026-08-26)" still describes `seq`, nonces/acks, 120 ms/2.56 s scanning. No Requirements, troubleshooting, uninstall; firmware referenced as `~/dev/onewheel/watch`. *Fix:* What it does → Requirements (Omarchy 4.0.x, BlueZ with `Experimental = true`, BLE 5 controller, OW-Watch firmware ≥ commit X) → Install → Pair → Verify → Troubleshoot (the failure modes above with their exact log lines) → Uninstall → dev notes. Move the status log to `docs/` as a changelog.

**S2 — Protocol version mismatch fails as gibberish.** Older firmware yields `not a theme packet` / `MAC does not verify (wrong key?)`, which reads as a pairing problem. *Fix:* print the protocol version in `--version` and `status`; a compat table in the README.

**S3 — Every accepted request also logs a "rejected" line** (`Set #1109` then `Set #1109 rejected: already seen`) and `status` shows `rejected 9`. The retransmit is by design. *Fix:* `ctr == ctr_last` not logged/counted; `monitor: … carries our data` line dropped or gated.

**S4 — Two daemons silently coexist** (`daemon.rs:74-78` unlinks and rebinds). *Fix:* bind first and fail with "another themesync daemon is already running", or flock `$XDG_RUNTIME_DIR/themesync.lock`.

**S5 — An unpaired install pays the whole radio cost and produces nothing.** No key → no beacon, but discovery runs unconditionally (`daemon.rs:298-311`), active scan with SCAN_REQs to everything in range, forever. *Fix:* no discovery without a key; a periodic "run `themesync pair`" line; surface in `status`.

**S6 — Privacy/RF cost of a 24/7 beacon is undocumented.** 30 ms constant, static address, theme name in cleartext; nothing backs off on lock/idle. *Fix:* README paragraph; better, stop beacon and scan on `loginctl` lock / idle.

**S7 — Key file world-readable window; config dir 0755.** `write_atomic` writes 0644 then chmods. *Fix:* `OpenOptions::mode(0o600).create_new(true)`; directory 0700.

**S8 — IPC socket authority and the `/tmp` fallback.** `socket_path()` (`ipc.rs:60-72`) falls back to `temp_dir()/themesync-<uid>.sock` where `<uid>` comes from `$UID` (never exported) → `$USER` — squattable. Anything running as the user can `Push`, `ResetCounter`, `PairPending`. *Fix:* refuse to run without `XDG_RUNTIME_DIR` (or verify owner+mode); state the trust model.

**S9 — Multiple adapters: the two stacks can pick different ones** (`bluer` `default_adapter()` vs btleplug `adapters().next()`). *Fix:* one `--adapter` option honoured by both.

**S10 — Packaging metadata incomplete, no LICENSE file.** `license = "MIT"` but no `LICENSE`; no `repository`/`readme`/`rust-version`; no tags, CHANGELOG, CI. *Fix:* add them; declare and test an MSRV; tag `v0.1.0`; GitHub Action for `cargo test` + `clippy` + `fmt`.

## Nice to have

**N1 — `themesync doctor`**: Omarchy present, `omarchy-theme-color` on PATH, `AdvertisementMonitorManager1`, `SupportedSecondaryChannels`, adapter powered/unblocked, unit enabled, hook points at an existing binary, key present, watch address known.
**N2 — `status` reads like internals**; `list push never` even after a pairing push (per-process state). Two modes: human summary and `--json`.
**N3 — btleplug is still needed** (all GATT paths + macOS); document the split rather than change it. Runtime deps per `ldd`: `libdbus-1.so.3`, `libsystemd.so.0`.
**N4 — Release profile**: 5.5 MB stripped; `codegen-units = 1`, `panic = "abort"` are cheap cuts.
**N5 — `--help` foregrounds dead v1 tooling** (`encode`/`decode`/`demo`/`scan`/`sync --proto v1`); hide behind `themesync dev …`.

## Verified fine
- `omarchy-theme-set` invocation safe: no shell, name is an on-disk directory chosen by crc match.
- A BLE neighbour without the key can do nothing: kind → no-key → HMAC → counter, all before any side effect; counter persisted atomically.
- No privileges needed; Arch's D-Bus policy allows `org.bluez` for the default context; no `bluetooth` group exists.
- `omarchy update` does not wipe the hook; a leftover hook after uninstall is harmless.
- `~/.local/share/omarchy` vs `/usr/share/omarchy` handled; `OMARCHY_PATH` is in the user-manager environment.

## PKGBUILD sketch
```bash
pkgname=omarchy-themesync
pkgver=0.1.0
pkgrel=1
pkgdesc="Push the active Omarchy desktop theme to a smartwatch over BLE"
arch=(x86_64 aarch64)
url="https://github.com/<you>/omarchy-themesync"
license=(MIT)
depends=(bluez dbus systemd-libs)
optdepends=('bluez-utils: bluetoothctl/btmon for debugging'
            'omarchy>=4.0.0: desktop theme source')
makedepends=(cargo git)
source=("$pkgname-$pkgver.tar.gz::$url/archive/v$pkgver.tar.gz")
build()   { cd "$srcdir/$pkgname-$pkgver/host"; cargo build --release --frozen; }
check()   { cd "$srcdir/$pkgname-$pkgver/host"; cargo test --frozen; }
package() {
  cd "$srcdir/$pkgname-$pkgver"
  install -Dm755 host/target/release/themesync "$pkgdir/usr/bin/themesync"
  install -Dm644 systemd/themesync.service     "$pkgdir/usr/lib/systemd/user/themesync.service"
  install -Dm644 LICENSE                       "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md                     "$pkgdir/usr/share/doc/$pkgname/README.md"
}
```
Plus a `.install` whose `post_install` prints: `Experimental = true` + `systemctl restart bluetooth`; `themesync install`; `themesync pair`.

## Minimum to ship v0.1
1. `LICENSE` + `repository`/`rust-version`/`readme` in `Cargo.toml`; tag `v0.1.0`.
2. Fix the unit (B4, B5).
3. Delete or fix the repo hook (B2).
4. `themesync install` / `uninstall` / `doctor`, with the checks wired into daemon startup and `status` (B1, B3, B5, N1).
5. No discovery without a key; stop logging the retransmit as "rejected" (S5, S3).
6. Rewrite README (S1, S2).
7. Key file 0600 from the start; refuse to run without `XDG_RUNTIME_DIR`; single-instance guard (S7, S8, S4).
8. PKGBUILD + `.install` notice; CI.

---

## Review 3: spec conformance and decoder safety (Opus, 2026-08-28)

Verdict: **no blocker.** No panic path in any Rust decoder, no out-of-bounds read in the C decoder; every §4 test vector recomputed independently (Python) — all seven match the code and the spec exactly.

## Should fix before release

### 1. The list COMMIT MAC has no freshness — the v3 change does not close the replay it claims to close
`host/src/themelist.rs:132-139`, spec `protocol/BEACON.md:296-298`. `total` and `crc16` are both computed *from `list`*, so `list_mac` is a pure function of the list bytes — exactly as before v3. The spec says the MAC is "bound to this transfer's header"; the change log says it closes review item #9. It does not: record one push, replay BEGIN/DATA…/COMMIT byte-for-byte later, the watch accepts a stale list (the `…0006` characteristic is writable by any central; no pairing/encryption).
**Fix:** real freshness in the transcript — the watch exposes a nonce in the READ status and the MAC covers `0x03 ‖ nonce ‖ total ‖ crc ‖ list`, or bind to the current request counter. Failing that, correct the claim in §3a.

### 2. Beacon replay: an old signed beacon forces a theme and drives NVS writes
`host/src/beacon.rs:61-69`, spec `protocol/BEACON.md:92-98`. The beacon carries no counter; the echo is not freshness. Any recorded beacon stays valid forever under the same key. Alternate two recorded beacons every ~10 s: each applies, stays stable 10 s, gets written to flash — contradicting §1's "only the MAC stands between a stranger and the watch's flash budget".
**Fix:** accept it explicitly in §1 (replace the flash-budget claim), or add a monotonic beacon counter under the MAC; cheapest watch-side mitigation is a rate limit on NVS writes (≥ 1/minute) independent of the MAC.

### 3. Unauthenticated remote log flood: the request MAC is checked *last*
`host/src/beacon.rs:176-192` (order: kind → length → op → ctr → MAC), `host/src/daemon.rs:435-442`. Input `54 03 01 00 07 00 00 <4 random bytes>` varying each event at 100 ms → one `request ignored: unknown op 0x07` journal line per event, forever, from a stranger with no key.
**Fix:** verify the MAC first, return `BadMac` for everything unauthenticated (removes the parse-error oracle too); rate-limit the "request ignored" line.

### 4. A theme slug longer than 31 bytes livelocks the SET loop
`host/src/protocol.rs:633-640` (name clamped to `V2_MAX_NAME = 31`), `host/src/themelist.rs:257` (unclamped), `host/src/daemon.rs:511` (resolves against the *full* slug). For any slug > 31 UTF-8 bytes the crcs never match → "slug unknown → push the list" → the watch commits the same list → repeat: one GATT push per gesture. Trigger: a 32-char theme directory. `WatchPalette::MAX_NAME_BYTES = 32` (`palette.rs:240`) disagrees with `V2_MAX_NAME = 31`.
**Fix:** in `themelist::build` skip slugs longer than `V2_MAX_NAME` with a `skipped` reason, or resolve SET args against the clamped names actually shipped.

### 5. `push-list --direct` pushes a list with a crc16 slug collision
`host/src/main.rs:528` vs. `main.rs:486` (dry-run) and `daemon.rs:200`. The direct/one-shot path (also taken automatically when no daemon listens, `main.rs:521`) skips the collision bail.
**Fix:** hoist the collision bail to right after `themelist::build` in the direct path.

### 6. `themesync pair` without a running daemon overwrites the active key before the watch confirms
`host/src/main.rs:448-449`. §2b: "the old key stays active until then". Without a daemon the new key is saved immediately; a wrong code or a walk-away cuts off the previously paired watch.
**Fix:** write `key.pending` and refuse to promote without confirmation, or refuse to pair without a daemon; at minimum `--force`.

### 7. A failed advertisement registration takes the beacon dark and leaves no fallback
`host/src/transport/adv.rs:41-53`. `handle.take()` before the new registration is known to succeed; retries re-submit the same failing bytes. Reachable via `Request::Push` with an unvalidated > 250-byte packet. No size guard on the beacon path (`MAX_PACKET_LEN = 240` only in the dead v1 GATT path, `ble.rs:199`); the legitimate worst case is 105 bytes.
**Fix:** register the new advertisement first, then drop the old handle; keep last-known-good `theme_wire`; validate `Push` at the IPC boundary.

### 8. `encode_state`'s documented precondition is unchecked
`host/src/beacon.rs:58-69` ("`theme` must be a v2 packet without meta records"), reachable via `Request::Push`. A packet with its own `0x42`/`0x43` record makes the beacon un-decodable by its own receiver.
**Fix:** return an error when `protocol::v2_theme_end(theme) != theme.len()`.

## Nice to have

9. An empty theme name silently breaks the request loop (`protocol.rs:641`, `daemon.rs:50-55`): a beacon with no slug can never satisfy §1 rules 3/4. Log a warning.
10. `write_atomic` temp paths collide for `key` and `key.pending` (`beacon.rs:220`, `with_extension` replaces the extension). Use `with_file_name`. No `fsync` — the "power cut" comment overstates.
11. A corrupt/empty `ctr` file silently resets replay protection to 0 (`beacon.rs:283`). Distinguish missing from unparseable.
12. Counter exhaustion at 65 535 has no re-pair hint on the desktop (`daemon.rs:468-477`).
13. `arg` is not required to be 0 for RESEND/LIST (`beacon.rs:191`), laxer than §2.
14. MAC comparisons are not constant-time (`beacon.rs:85`, `:188`); `subtle::ConstantTimeEq` is free.
15. `themelist::frames` truncates offsets silently for lists > 64 KB (`themelist.rs:154`, `:160`); unreachable through `build` but the function is public.

## Spec / code drift

16. **`protocol/THEME_PROTOCOL.md` documents v1 only** (`"TH"` magic, fixed colour block, GATT `7e45000x`). The only normative description of v2 TLV is a comment block at `host/src/protocol.rs:527-537`. Promote it into a spec file; mark v1 historical.
17. **The C in `watch/` implements only v1** and is unrelated to BEACON.md v3: no C implementation of the beacon, request, v2 TLV parser, `v2_theme_end`, `mac4` or the list transfer anywhere in the repo, so the §4 vectors have no C twin. Port the v3 decoders into `watch/common/` (and `theme_sim --selftest`) or state in the README that `watch/` is a historical v1 reference.
18. The desktop's pending key never expires, contrary to §2b's 120 s window (`daemon.rs:333-337`, deliberate). Document or expire.
19. Doc nit: `ble.rs:275-276` says the list READ returns a "6-byte status"; it is 5.

## Five tests to add

1. Hostile-input sweep over every decoder (lengths 0..40 of structured-random bytes plus truncations/extensions of every encoder output) — no panic, `v2_theme_end(b) <= b.len()`.
2. Slug round-trip invariant: for every list entry, `crc16(<0x40 name on the wire>) == beacon::slug_id(<slug the daemon resolves>)`, with a 40-byte and a non-ASCII slug in the fixtures.
3. Beacon bytes == list-entry bytes for the same theme (`current_theme()` and `themelist::build` take different paths to `WatchPalette.name`).
4. The counter rule as a unit: factor `daemon.rs:467-481` into `accept(ctr, last) -> Decision`; test reject/accept/gaps/promotion/reset.
5. Apply-rule conformance for `decode_state`: `0x42` missing, in the middle, echo after mac, duplicate `0x43`, oversize theme; theme crc stable across echo changes; largest legal beacon fits the manufacturer-data limit.

## Checked and found correct
- All §4 vectors recomputed independently; `crc16` Rust and C identical, `0x29B1` on "123456789".
- MAC coverage complete in both messages; domain separation sound (`54 03`, `54 01`, `03`).
- Byte layouts in §1/§2/§2b/§3a match the encoders, including endianness and record order.
- No panic path in any Rust decoder; no OOB read in `theme_proto.c` for 31- or 255-byte PDUs.
- Duplicate/unknown TLVs behave as documented; crc16 collision guard correct where it runs; `frame_len_for_mtu` clamps correctly; `watch_addr` written only inside a MAC-verified branch.
- `cargo test`: 40 passed.
