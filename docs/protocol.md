# Linksy mobile handoff

This document is for whoever builds the Linksy phone app (Flutter, Android first). It
describes the product, what the desktop side already does, the exact wire protocol the
phone must speak, and what is still missing. It is updated every time the desktop side
changes; see the [changelog](#changelog) at the end.

Repository: `LinkSy-mg/linksy-desktop` (this repo). Desktop code is Rust; the Linux app
is GTK4/libadwaita.

---

## 1. Product in one page

**Linksy — Your devices, connected.** Local-first sharing between your own devices: no
accounts, no cloud, no relay server. Devices on the same network find each other, pair
once with a 6-digit code, then share files, clipboard text and notifications directly.

Decisions already made with the owner:

- **Platforms:** Linux desktop first, **Android** is the phone platform. iPhone, macOS and
  Windows come later.
- **Native feel on each platform.** On Android that means Material You (dynamic color,
  Material 3 components, predictive back, edge-to-edge), not a desktop look.
- **Lightweight above all.** Small install, tiny idle footprint, no background work
  unless there is something to do.
- **Battery:** devices *always know who is nearby* but only *connect when there is
  something to share* (see §4.6). No always-on connection by default.
- **Brand:** the logo is the "Sender" mark (a larger dot with an arc handing off to a
  smaller one) in `brand/logo/`. The palette is **Signal**: blue `#2563FF` (sender,
  accent), cyan `#22C3E6` (receiver), deep `#1438B8`. Use `#2563FF` as the Material You
  seed color; `brand/logo/logo.png` is a 1024×1024 export on white.

## 2. Feature status

| Feature | Desktop (Linux) | Android | Notes |
|---|---|---|---|
| Discovery on the local network (mDNS) | ✅ | ⬜ | §4.1 |
| Presence (who's nearby, re-checked) | ✅ | ⬜ | §4.6 |
| Pairing with a 6-digit code | ✅ | ⬜ | §4.3 |
| Pair by IP address (when discovery is blocked) | ✅ | ⬜ | default port 47200 |
| Send / receive files and folders | ✅ | ⬜ | resume + BLAKE3 check, §4.5 |
| Accept / decline prompt for incoming files | ✅ | ⬜ | 120 s timeout |
| Auto-accept from paired devices (setting) | ✅ | ⬜ | |
| Clipboard text sharing (on demand) | ✅ | ⬜ | up to 256 KiB |
| Device-to-device notifications ("ping") | ✅ | ⬜ | |
| Rename device (live to peers) | ✅ | ⬜ | `Renamed` message |
| Connect on demand / idle close / stay connected | ✅ | ⬜ | §4.6 |
| Desktop notifications with Accept/Decline | ✅ | ⬜ | Android: notification actions |
| QR-code pairing | ✅ shows QR | ⬜ | phone scans desktop, §4.7 |
| Automatic clipboard sync | ⬜ | ⬜ | planned |
| Images/files on the clipboard | ⬜ | ⬜ | planned |
| Phone notifications mirrored to desktop | ⬜ | ⬜ | planned; Android NotificationListenerService |
| Reply to messages from desktop | ⬜ | ⬜ | planned; via notification RemoteInput |
| Phone as webcam | ⬜ | ⬜ | planned |
| Bluetooth LE presence | ⬜ | ⬜ | planned |
| Screen mirroring (watch / share) | ✅ | ⬜ | §4.8; phone can share via MediaProjection |
| Remote control of a mirrored screen | ✅ | ⬜ | §4.8, kind `control` |
| Sidecar (second display) | ✅ | ⬜ | §4.8, kind `display`; a tablet/phone can be the display |
| Phone as webcam | ⬜ | ⬜ | planned; same session stream, kind `camera` |

## 3. How to build the phone app (recommendation)

**Use the Rust core through `flutter_rust_bridge`; do not reimplement the protocol in
Dart.** The protocol needs QUIC with TLS 1.3 *mutual* authentication using self-signed
certificates, a custom certificate verifier, and a TLS keying-material exporter for the
pairing code. Dart's `dart:io` has no QUIC and no exporter API, and a second
implementation would drift. Reusing `crates/linksy-core` gives byte-identical behavior
with the desktop.

- Build `linksy-core` for `aarch64-linux-android`, `armv7-linux-androideabi`,
  `x86_64-linux-android` (e.g. with `cargo-ndk`). It uses `ring`, `quinn`, `rustls`,
  `mdns-sd`, `tokio`: all build for Android.
- Wrap the `Node` API (§5) in a thin bridge crate inside the Flutter project, not in
  `linksy-core` itself.
- Run the node inside an Android **foreground service** while a transfer or prompt is in
  progress; otherwise let the system sleep. Presence (mDNS) needs a
  `WifiManager.MulticastLock` while the app is in use.
- Store `cert.der`, `key.der`, `trusted.json` and `settings.json` in the app's private
  files dir (`Config::data_dir`). The key file must stay private.

Things the core still needs for Android (desktop side will add them; tracked in §7):

1. **Sending from `content://` URIs.** `Node::send_files` takes file paths. Android share
   intents give content URIs. Planned: an API that takes open file descriptors / readers
   with a name and size.
2. **Receiving into MediaStore.** Received files go to `Settings::download_dir`. On
   Android point it at an app-specific dir and move finished files into
   `MediaStore.Downloads`, or wait for a "save via callback" API.
3. **Discovery from the platform.** ✅ Available: if raw multicast is unreliable on some
   phones, browse with `NsdManager` (service `_linksy._udp.`), read the TXT record
   (§4.1) and call `node.device_found(device, addrs)` / `node.device_lost(id)`. Start
   the node with `Config::discovery = false` in that case. (Advertising still has to be
   done too: register the same service with `NsdManager`.)

## 4. Wire protocol (version 1)

Everything here is implemented in `crates/linksy-core`; source references are given so
details can be checked.

### 4.1 Discovery (mDNS / DNS-SD) — `src/discovery.rs`

- Service type: `_linksy._udp.local.`
- Instance name: `"<device name> <SHORT-ID>"`, e.g. `Studio Desktop EBB0-9CE2`.
- Host name: `linksy-<short id lowercase>.local.`, e.g. `linksy-ebb0-9ce2.local.`
- Port: the device's QUIC UDP port. Desktop default **47200**, random if taken.
- TXT records:

  | key | value |
  |---|---|
  | `id` | full device id: 64 lowercase hex chars (§4.2) |
  | `name` | device name (≤ 48 chars) |
  | `kind` | `desktop`, `laptop`, `phone` or `tablet` |
  | `os` | `linux`, `android`, `macos`, `windows`, … |
  | `v` | protocol version, `1` |

- TXT data is only a hint. Identity is always proven by TLS (§4.2); never trust a TXT
  `id` for anything but display and choosing whom to dial.
- A device leaving cleanly sends an mDNS goodbye. Presence is re-checked every 45 s by
  re-querying each visible instance; no answer → the device is gone.
- On rename, the old instance is unregistered (goodbye) and a new one registered.

### 4.2 Identity — `src/identity.rs`

- Each install generates a **self-signed X.509 certificate** with an **ECDSA P-256 /
  SHA-256** key (rcgen defaults), SAN `linksy`.
- **Device id = SHA-256 of the certificate DER**, shown as 64 lowercase hex chars.
- **Short id** = first 4 bytes of the id, uppercase hex, split `XXXX-XXXX`.
- The id never changes for an install. Paired devices pin it.

### 4.3 Transport and TLS — `src/tls.rs`

- **QUIC** (RFC 9000) over UDP, **TLS 1.3 only**, **ALPN `linksy/1`**, SNI `linksy`.
- **Mutual TLS:** both sides present their device certificate. Each side accepts any
  certificate whose TLS 1.3 handshake signature verifies, then computes the peer id from
  the presented certificate and checks it (§4.4).
- Transport settings used by desktop: stream receive window 16 MiB, connection receive
  window 32 MiB, send window 32 MiB, keep-alive every 10 s, idle timeout 30 s.
- **Pairing code:** TLS exporter (RFC 8446 §7.5) with label `linksy pairing code v1`,
  empty context, 8 bytes output. Interpret as a big-endian `u64`, take `% 1_000_000`,
  format as 6 digits with leading zeros. Both ends of the same TLS session get the same
  code; a man in the middle cannot make both screens match. Display as `123 456`.

### 4.4 Framing and handshake — `src/protocol.rs`, `src/node.rs`

**Framing:** every control message is a big-endian `u32` length followed by that many
bytes of UTF-8 JSON. Maximum frame 1 MiB. Enums are tagged with `"type"` in snake_case.

**Handshake:** the dialer opens the **first bidirectional stream** and sends `hello`:

```json
{"type":"hello","version":1,"intent":"connect","stay_connected":false,
 "device":{"id":"<64 hex>","name":"Pixel 9","kind":"phone","os":"android"}}
```

- `intent`: `connect` (already paired) or `pair` (start pairing).
- `stay_connected` (optional, default `false`): the sender keeps links open, so the
  receiver must not close this connection for being idle.
- `invite` (optional, omitted when unused): the token from a scanned QR code (§4.7).

The listener checks `device.id` equals the fingerprint of the TLS certificate (else
closes with "identity mismatch") and answers:

```json
{"type":"welcome","stay_connected":false,
 "device":{"id":"<64 hex>","name":"Studio Desktop","kind":"desktop","os":"linux"}}
```

or rejects:

```json
{"type":"reject","reason":"not_paired"}      // also "incompatible_version", "busy"
```

The dialer checks `welcome.device.id` equals the listener's certificate fingerprint.
For `intent: connect`, a dialer also refuses (closes) if the peer isn't in its trust
list.

**Pairing** (after `welcome`, on the same stream): both sides compute the code, show it,
and ask their user. Each side sends its decision once:

```json
{"type":"pair_decision","accept":true}
```

Pairing succeeds only if **both** sent `accept: true`; each side then stores the other in
its trust list. A decline on either side ends pairing on both (the other prompt should
close immediately). Timeout 60 s. After pairing, the connection becomes a normal
connection.

**Duplicate connections:** if both sides dial each other at once, keep the connection
**opened by the device with the smaller id** (byte-wise comparison of the 32-byte ids)
and close the other with reason `duplicate`.

### 4.5 Streams after the handshake

- **Unidirectional stream = one small message**, then finish. Types (`Message`):

  ```json
  {"type":"notify","title":"Ping","body":""}
  {"type":"clipboard","text":"…"}             // ≤ 256 KiB of UTF-8, else ignored
  {"type":"renamed","name":"Living Room PC"}    // sender's new name
  {"type":"ping"}
  ```

  Receiving any of these counts as activity on the connection.

- **Bidirectional stream = one file transfer** (`TransferMsg`), opened by the sender:

  1. Sender → `{"type":"offer","id":"<uuid>","files":[{"path":"Photos/a.jpg","size":123}]}`
     Paths are relative, `/`-separated. Folders are sent as their files, with the folder
     name as the first component.
  2. Receiver asks the user (or auto-accepts), then answers
     `{"type":"accept","offsets":[0, 5242880, …]}` — one entry per file: how many bytes
     it already has (resume) — or `{"type":"decline"}`. Unanswered after 120 s = decline.
  3. For each file in order, the sender writes the **raw bytes from the offset to the
     end** (not framed), then `{"type":"file_done","blake3":"<hex>"}` where the hash
     covers the **whole file** (including the part the receiver already had).
  4. After the last file the sender finishes its side of the stream. The receiver
     answers `{"type":"complete"}` or `{"type":"failed","reason":"…"}`.

  Receiver rules: write into `<name>.linksy-part`, rename to the final name only after
  the hash matches; delete the part file on mismatch. Resume from an existing
  `.linksy-part`. Never overwrite: pick `name (1).ext`, `name (2).ext`, …
  **Reject unsafe paths:** empty, absolute, containing `..`, `.`, `\`, `:` or NUL.

### 4.6 Presence and connection lifetime

- Presence = discovery (§4.1). The UI shows a paired device as **Nearby** when it is
  announced on the network, **Connected** while a link is open, **Not nearby** otherwise.
- **Connect on demand:** sending anything dials the device at every known address in
  parallel (announced addresses + last address that worked), 3 s per attempt. **Only the
  QUIC/TLS handshakes race**: the first connection whose certificate matches the wanted
  device gets the Linksy `hello`; the others are closed before any stream is opened.
  (Running `hello` on several connections at once makes the receiver register
  duplicates that close each other.)
- **Idle close:** a connection with no transfer or prompt in progress and no message for
  **60 s** is closed (QUIC close code 0, reason `idle`) — unless either side said
  `stay_connected: true`. The next send dials again.
- Listening costs nothing; incoming connections are always accepted, so a phone can be
  reached while the app's service is running.

### 4.7 QR-code pairing (invites) — `src/invite.rs`

The desktop shows a QR code (menu → "Pair with QR Code…", or `linksy invite`). It
encodes a link:

```
linksy://pair?v=1&id=<64 hex>&n=<name>&a=<ip:port>,<ip:port>&t=<token>
```

- Values are percent-encoded (RFC 3986 unreserved characters stay as is); `+` decodes
  to a space. Unknown keys must be ignored. `v` must be `1`.
- `a`: up to four addresses, best first (private IPv4 LAN, then other IPv4 such as VPNs,
  then IPv6). IPv6 is in brackets: `[fd00::1]:47200`.
- `t`: one-time token (32 hex chars). Valid **five minutes**, used up by the first
  successful pairing, and cancelled when the dialog closes (the desktop shows a fresh
  one every 4.5 minutes while open).

The phone, after scanning:

1. Races QUIC connections to the addresses (§4.6) and keeps the first one whose
   **certificate fingerprint equals `id`**. Any other certificate: close, try the rest,
   fail with "That's not the device in the QR code" if none matches.
2. Sends `hello` with `"intent":"pair"` and **`"invite":"<token>"`**.
3. After `welcome`, sends `{"type":"pair_decision","accept":true}` straight away (no code
   shown; scanning is the user's consent) and reads the desktop's decision.

The desktop, on a `hello` whose `invite` matches its current token, also skips the
prompt and answers `accept: true`. If the token is wrong, reused or expired it falls back
to the normal flow: it shows the 6-digit code and asks its user. The phone should then
show the code too (§4.3) — so a phone must still support the code flow.

Security: the phone authenticates the desktop by the pinned `id`; the desktop
authenticates the phone by the token, which only someone who saw the screen has.

### 4.8 Media sessions (screen mirroring) — `src/protocol.rs`, `src/node.rs`

A media session is a **bidirectional stream** whose first frame is a session request
(file offers are the other kind of bidirectional stream; dispatch on the first frame's
`type`). The device that wants to **watch** opens it:

```json
{"type":"session_request","id":"<uuid>","kind":"screen"}
```

The receiving device **always asks its user** ("Pixel 9 wants to see your screen",
Allow / Don't Allow; auto-accept never applies) and answers within 120 s:

```json
{"type":"session_accept"}      // or {"type":"session_decline"}
```

After `session_accept`:

- **Sharer → viewer:** raw **H.264 Annex-B byte stream** (no framing): start codes,
  SPS/PPS repeated before every keyframe (`config-interval=-1`), access-unit aligned.
  **Profile: Constrained Baseline** (every decoder handles it, including OpenH264 and
  phone hardware). Desktop sends ~8 Mbit/s, keyframe at least every 2 s (60 frames),
  up to 30 fps, at the screen's resolution.
- **Viewer → sharer:** framed JSON control messages (u32 BE length + JSON). A sharer must
  keep reading this half and ignore unknown types. In `screen` sessions the viewer sends
  nothing; in **`control`** sessions (same media, plus input; the prompt says "wants to
  control your screen") it sends input:

  ```json
  {"type":"pointer","x":0.42,"y":0.13}                  // fraction of the shared screen, 0–1
  {"type":"button","button":"left","pressed":true}     // left|middle|right|back|forward
  {"type":"scroll","dx":0,"dy":2,"discrete":true}     // wheel steps; false = pixels
  {"type":"key","keysym":65307,"pressed":true}         // X11 keysym (Escape here)
  ```

  On Android, a phone viewer controlling a desktop maps taps to `pointer` + `button`
  left, two-finger drags to smooth `scroll`, and the soft keyboard's characters to
  keysyms (`U+0020–U+007E` are the keysym; others are `0x01000000 + codepoint`).
  A phone sharing its screen can't accept desktop input without an accessibility
  service; leave `control` unsupported there (decline the request).

- **`display` (Sidecar): roles reversed.** The device that *sends* `session_request` with
  `"kind":"display"` is the one that shares: it creates a new virtual monitor (desktop:
  1920×1080) and streams it; the receiver, after its user allows ("X wants to use this
  screen as a second display"), shows it **full screen** and sends `pointer` / `button` /
  `scroll` / `key` input back, exactly as in `control`. This is how an Android tablet or
  phone becomes a desktop's second screen: accept the request, go immersive full screen,
  decode, and map touches to `pointer` + `button` left (two-finger drag → `scroll`).
  Core reports `sharing: true` on the requester and `false` on the display.
- Either side ends the session by finishing/resetting its half or closing the stream.
  A connection with a session open is never closed as idle.

On Android:
- **Watching** a desktop: decode with `MediaCodec` (`video/avc`) into a `SurfaceView`;
  split on start codes or feed via an Annex-B parser.
- **Sharing** the phone's screen with a desktop: `MediaProjection` (the system asks the
  user) → `MediaCodec` encoder, `KEY_PROFILE = AVCProfileConstrainedBaseline`,
  `KEY_I_FRAME_INTERVAL = 2`, prepend SPS/PPS (`csd-0`/`csd-1`) to every keyframe, write
  Annex-B to the stream. The desktop app then shows it in its viewer window.
- Core API: `node.request_session(id, SessionKind::Screen | Control | Display) -> Session` (read
  `session.recv`), and `node.incoming_sessions()` → accepted `Session`s to feed
  (`session.send`). Events: `session_request {request, from, kind}`,
  `session_started {session, peer, kind, sharing}`, `session_ended {session}`.

## 5. Core API the app would call (`linksy_core::Node`)

```text
Node::start(Config) -> Node                 Config { data_dir, download_dir, name, kind, port, bind, discovery, stay_connected, idle_after }
node.info() -> DeviceInfo                   node.subscribe() -> stream of Event
node.devices() -> [DeviceView]              DeviceView { device, trusted, connected, nearby }
node.pair_device(id) / node.pair(addr)      node.respond(request_id, accept)
node.send_files(id, paths) -> transfer_id   node.cancel(transfer_id)
node.send_clipboard(id, text)               node.notify(id, title, body)
node.settings() / node.update_settings(s)   Settings { name, download_dir, stay_connected, auto_accept }
node.forget(id)                             node.shutdown()
node.device_found(device, addrs)            node.device_lost(id)        // platform discovery
node.create_invite() -> Invite (.to_uri())  node.cancel_invite()        // show a QR code
node.pair_with_invite(uri)                                              // after scanning one
node.request_session(id, kind) -> Session   node.incoming_sessions()    // screen mirroring
```

Events (JSON-serializable, same shapes as the desktop IPC):
`device_found`, `device_lost`, `device_updated`, `connected`, `disconnected`,
`pairing_request {request, device, code, incoming}`, `paired`, `pairing_failed`,
`transfer_request {transfer, from, files, total}`, `transfer_started`,
`transfer_progress {transfer, bytes, total}`, `transfer_complete {transfer, saved}`,
`transfer_failed {transfer, reason}`, `settings_changed`, `notification`, `clipboard`.

Every consent question (pairing, incoming files) is an event with an id; the UI answers
with `respond(id, accept)`. Whichever front end answers first wins (desktop: the window
or the notification).

## 6. UX notes from the desktop app

The owner found the first single-column design "too simplistic and non intuitive" and
chose **device pages**: a list of devices, and per device a page with big actions and a
history of everything shared with it. Mirror that on the phone:

- **Device list** (home): paired devices with colored initials avatars and a presence
  dot (Nearby / Connected / Not nearby), then "Nearby, not paired" with a Pair button,
  and a prominent **Add Device** (QR scanner first on the phone).
- **Device page**: big avatar, name, "Nearby · Linux laptop"; three big action buttons
  **Send Files**, **Send Clipboard**, **Ping**; then **Activity** grouped by day (Today,
  Yesterday, date), newest first. Rows: icon in a tinted circle (green incoming, accent
  outgoing, red failed/declined), title (file name / "N items" / quoted clipboard text /
  ping title), detail ("Received · 2.1 MB · 14:03", "Declined · …", or live
  "81 MB of 400 MB · 38 MB/s · 8 seconds left" with a progress bar and cancel). Tap a
  received file to open it; clipboard rows have "Copy again".
- History is kept by the desktop service (`linksyd`), not by the core: the phone keeps
  its own. Desktop keeps 500 entries and never writes clipboard text to disk (it may be a
  password); do the same.

Earlier notes, still valid:

- On Android: the system **share sheet** ("Share → Linksy") is the main entry point;
  the app shows nearby devices to pick from. A Quick Settings tile could send the
  clipboard.
- Pairing dialog shows the code large (`123 456`), with "Pair" / "Decline".
- Incoming files: "<Device> Wants to Share", "N items · size", list of names, Accept /
  Decline. Also as a notification with actions when the app isn't open.
- Transfers show progress, speed and time left; finished incoming transfers open the
  file or its folder.
- Errors name the device in plain words: "Studio Desktop isn't reachable. Check that
  it's on and on this network."
- Settings: device name, save location, accept from paired devices automatically, stay
  connected (off by default; explain it costs battery). The desktop also shows its own
  address there, for "Pair by Address".
- When no device is around, the empty state offers "Pair with QR Code" and "Pair by
  Address" right there. On the phone: a big "Scan QR Code" button belongs in that state.
- An outgoing transfer says "Waiting for <device> to accept…" until the other side
  answers, then shows progress.
- Received files can be opened by tapping the finished transfer (desktop also allows
  dragging them out).

## 7. Open items affecting mobile

- [ ] Core: send from file descriptors / readers (for `content://` URIs).
- [ ] Core: receive through a callback or into a caller-provided sink (MediaStore).
- [x] Core: inject discovered devices from the platform (`device_found` / `device_lost`).
- [x] QR pairing format and desktop side (§4.7). The phone side (scanner) is to build.
- [ ] Phone showing a QR for a desktop to scan: no desktop camera flow planned; use the
      phone as the scanner.
- [x] Brand color: Signal.

## Changelog

- **2026-10-19** — Document created. Desktop has discovery, presence re-checks, pairing,
  files/folders with resume, clipboard text, notifications, connect-on-demand with idle
  close, stay-connected handshake flag, live settings (name, download folder,
  auto-accept, stay connected), live rename (`renamed` message), pair by address.
- **2026-10-19** — `Node::device_found` / `device_lost` for platform discovery. Fixed:
  connecting on demand to a device with several addresses could fail (parallel `hello`s);
  now only QUIC handshakes race (§4.6). After a rename, the old name's late mDNS goodbye
  no longer hides the device. Desktop app: open/drag/clear received files.
- **2026-10-19** — QR-code pairing (§4.7): `linksy://pair` invite links with a one-time
  token, `invite` field in `hello`, desktop dialog and `linksy invite`.
- **2026-10-19** — Desktop redesign to device pages with an activity timeline (§6), and
  a history kept by the service. No protocol change.
- **2026-10-19** — Brand palette chosen: Signal (§1).
- **2026-10-19** — Screen mirroring (§4.8): `session_request`/`accept`/`decline` streams,
  H.264 Constrained Baseline Annex-B, core `request_session` / `incoming_sessions`.
- **2026-10-19** — Remote control (§4.8): session kind `control` and the input messages
  (`pointer`, `button`, `scroll`, `key`); core `ControlMsg`, `control_frame`,
  `read_control_frames`.
- **2026-10-19** — Sidecar (§4.8): session kind `display`, where the requester shares a
  virtual monitor and the receiver shows it full screen and sends input back.
