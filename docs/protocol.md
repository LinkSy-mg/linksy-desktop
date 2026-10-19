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
  smaller one) in `brand/logo/`. The brand color is **not chosen yet**; candidates are
  Signal `#2563FF`, Aurora `#7457F5`, Coral `#FF5A52`, Mint `#12B282` (+ ink `#1E2A3B`),
  Mono. The desktop app uses the system accent color. On Android, dynamic color with the
  chosen brand color as the fallback seed is the natural fit.

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
| QR-code pairing | ⬜ | ⬜ | planned; phone scans desktop |
| Automatic clipboard sync | ⬜ | ⬜ | planned |
| Images/files on the clipboard | ⬜ | ⬜ | planned |
| Phone notifications mirrored to desktop | ⬜ | ⬜ | planned; Android NotificationListenerService |
| Reply to messages from desktop | ⬜ | ⬜ | planned; via notification RemoteInput |
| Phone as webcam | ⬜ | ⬜ | planned |
| Bluetooth LE presence | ⬜ | ⬜ | planned |
| Screen mirroring / remote input | ⬜ | ⬜ | desktop-only, optional |

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

- One main screen: a drop/choose area on top, devices below, transfers under that.
  On Android: the system **share sheet** ("Share → Linksy") is the main entry point;
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
  connected (off by default; explain it costs battery).

## 7. Open items affecting mobile

- [ ] Core: send from file descriptors / readers (for `content://` URIs).
- [ ] Core: receive through a callback or into a caller-provided sink (MediaStore).
- [x] Core: inject discovered devices from the platform (`device_found` / `device_lost`).
- [ ] QR pairing: desktop shows a QR with `linksy://pair?id=<id>&addr=<ip:port>&name=…`;
      phone scans and dials with `intent: pair`. Format not final.
- [ ] Brand color choice.

## Changelog

- **2026-10-19** — Document created. Desktop has discovery, presence re-checks, pairing,
  files/folders with resume, clipboard text, notifications, connect-on-demand with idle
  close, stay-connected handshake flag, live settings (name, download folder,
  auto-accept, stay connected), live rename (`renamed` message), pair by address.
- **2026-10-19** — `Node::device_found` / `device_lost` for platform discovery. Fixed:
  connecting on demand to a device with several addresses could fail (parallel `hello`s);
  now only QUIC handshakes race (§4.6). After a rename, the old name's late mDNS goodbye
  no longer hides the device. Desktop app: open/drag/clear received files.
