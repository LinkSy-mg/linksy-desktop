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

Android column from the phone's `docs/android-app.md` (LinkSy-mg/linksy-mobile, 2026-10-21).
"Not possible" rows are hidden by the desktop for Android phones (see §4.10).

| Feature | Desktop (Linux) | Android | Notes |
|---|---|---|---|
| Discovery, presence, pairing (code, address, QR) | Done | Done | §4.1, §4.6, §4.7 |
| Files and folders, accept/decline, resume | Done | Done | §4.5; phone saves to Downloads/Linksy |
| Clipboard text (on demand), ping, rename | Done | Done | |
| Open on PC (`open_url`) | Done | Done, both ways | §4.5 |
| Battery, Ring / Find | Done | Done | §4.5 |
| The computer's music on the phone | Done | Done | `media_watch`, `media_state`, `media_command` |
| The phone's music on the computer | Done | Not possible | needs notification-listener access |
| Pause the computer's media during calls | Done | Done | `call_state`; never a `contact` |
| Lock when away | Done | Done | `heartbeat`; foreground service |
| Photo auto-import | Done | Done | one folder the user picks, `YYYY-MM` folders |
| Phone in the Files sidebar | Done | Done (one folder) | `fs`, §4.9; `info` gives the folder name |
| Touchpad, keyboard, slides | Done | Done | session `input`, §4.8 |
| Phone shares its screen | Done | Done | `screen` |
| Phone as webcam | Done | Done | `camera`; desktop: V4L2 "Linksy Camera" with v4l2loopback |
| Phone as second display | Done | Done | `display`; at the phone's shape since desktop 0.5.5 |
| Phone watches / controls the computer | Done | Done | phone requests `screen` / `control` |
| Controlling the phone from the computer | Done | Not possible | AccessibilityService is blocked for sideloaded apps |
| Texting from the computer | Done | Not possible | SMS permissions are blocked for sideloaded apps |
| Phone notifications on the computer, reply | Done | Not possible | notification-listener access is blocked |
| Images/files on the clipboard | Planned | Planned | §2 of `docs/android-app.md` |
| Automatic clipboard sync, computer → phone | Planned | Planned | `clipboard_watch` |
| Bluetooth LE presence | To do | To do | |

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
3. **Discovery from the platform.** Available: if raw multicast is unreliable on some
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
  {"type":"open_url","url":"https://…"}        // "Open on PC": open in the browser
  {"type":"battery","level":82,"charging":false} // sender's battery, percent 0–100
  {"type":"ring"}                               // ring loudly so the user finds you
  {"type":"ring_stop"}
  // Phone notifications mirrored to the computer (phone → computer):
  {"type":"notification_posted","key":"0|com.whatsapp|1|null|10123","app":"com.whatsapp",
   "app_name":"WhatsApp","title":"Alice","body":"Lunch?","can_reply":true,
   "actions":[{"id":"read","label":"Mark as read"}],"icon":"<base64 PNG, optional>"}
  {"type":"notification_removed","key":"…"}
  // …and what the computer asks back (computer → phone):
  {"type":"notification_reply","key":"…","text":"Sure!"}
  {"type":"notification_action","key":"…","action":"read"}
  {"type":"notification_dismiss","key":"…"}
  // Media controls, both ways:
  {"type":"media_watch","on":true}              // "send me your media_state while connected"
  {"type":"media_state","seq":1760945000123,"players":[{"id":"spotify","app":"Spotify",
   "title":"…","artist":"…","album":"…","playing":true,"position_ms":42000,
   "length_ms":215000,"can_next":true,"can_previous":true,"can_seek":true,"art":"<base64>"}]}
  {"type":"media_command","player":"spotify","command":{"action":"play_pause"}}
  // actions: play, pause, play_pause, next, previous, seek (+ "position_ms")
  {"type":"call_state","state":"ringing","contact":"Alice"}   // ringing | active | idle
  {"type":"heartbeat_watch","on":true,"every_s":20}            // computer → phone
  {"type":"heartbeat"}                                         // phone → computer, every every_s
  // Texting from the computer: requests (computer → phone)…
  {"type":"sms_list_threads","limit":50}
  {"type":"sms_list_messages","thread":"7","before_ms":1760900000000,"limit":50}
  {"type":"sms_send","client_id":"<uuid>","to":["+261340000000"],"text":"Sure!"}
  // …answers and news (phone → computer):
  {"type":"sms_threads","threads":[{"id":"7","addresses":["+261…"],"names":["Alice"],
   "snippet":"Lunch?","date_ms":1760945000000,"unread":true}]}
  {"type":"sms_messages","thread":"7","messages":[{"id":"312","from_me":false,
   "address":"+261…","body":"Lunch?","date_ms":1760945000000}],"more":true}
  {"type":"sms_message","thread":"7","message":{…}}        // new message, either way
  {"type":"sms_sent","client_id":"<uuid>","ok":true}        // or "ok":false,"error":"…"
  ```

  Receiving any of these counts as activity on the connection. Unknown types are
  ignored, so a newer device can send messages an older one doesn't know yet.

  `open_url`: only `http://` and `https://` links, at most 8 KiB, no control characters;
  receivers drop anything else (the core enforces this both ways). The desktop opens it
  in the default browser at once and lists it in the activity timeline.

  `battery`: the phone sends it right after connecting and whenever the level moves by
  5 points or charging starts/stops (not more often: each send may open a connection).
  The desktop shows it on the device while the phone is around, and notifies once at
  15% or below when not charging. `ring`: play the alarm stream at full volume, even on
  silent/DND, until `ring_stop`, a tap on a full-screen "Found it", or 30 s. `ring` and
  `ring_stop` travel on separate streams and may arrive out of order; ignore a
  `ring_stop` that comes before its `ring` within a second. A desktop that receives
  `ring` shows an urgent notification with a sound ("find my laptop").

  **Notifications.** `key` is the phone's stable id for the notification (Android:
  `StatusBarNotification.getKey()`), 1–1024 bytes; posting the same key again updates it
  in place. `icon` is a small PNG of the app's icon (96 px is plenty), base64, at most
  64 KiB raw; send it every time or only the first time per app (the desktop caches it
  per `app`). The desktop shows each one as a native notification with up to three
  buttons: Reply (when `can_reply`), the first of `actions`, and "Mute <app>" (muting is
  stored on the computer; the phone needn't know). What comes back:
  - `notification_reply`: answer through the notification's `RemoteInput` action.
  - `notification_action`: fire the action with that `id` (use the action's index or a
    stable label as the id).
  - `notification_dismiss`: the user clicked it or swiped it away on the computer: cancel
    it on the phone (`NotificationListenerService.cancelNotification(key)`).
  Send `notification_removed` when it goes away on the phone (read, swiped, answered), so
  it goes away on the computer too. Nothing of it is stored on either side.

  **Media.** `media_state` is always the sender's *whole* set of players; a player missing
  from it has stopped. `seq` is the time it was taken in ms since 1970: receivers keep the
  newest and drop late, older ones (streams can arrive out of order). `art` is optional,
  JPEG or PNG, at most 64 KiB raw.
  - **Phone → computer:** send `media_state` whenever playback changes (play/pause,
    track, every few seconds isn't needed: position is extrapolated) and after
    connecting. The computer shows each player in GNOME's media controls as
    "<app> on <phone>", and the keyboard's media keys work on it; buttons come back as
    `media_command`.
  - **Computer → phone:** send `media_watch {on: true}` while the phone shows the
    computer's player (its media notification / page), `{on: false}` when it stops. The
    computer then sends `media_state` on every change while the connection lives, and
    acts on `media_command` for its players.

  **Calls.** Send `call_state` to paired computers that are around when a call starts
  ringing, is answered, and ends (`idle`). The computer pauses whatever plays when it
  rings and resumes exactly those players at `idle`; ringing also shows "Call from
  <contact>" for 20 s. `contact` is optional: only send it if the user allowed contacts.

  **Lock when you walk away.** A computer where the user turned it on for this phone sends
  `heartbeat_watch {on: true, every_s}` (5–20 s) when the phone connects and when it's
  switched on, `{on: false}` when switched off. While on, send `heartbeat` every
  `every_s` seconds; a repeated watch replaces the schedule (don't start a second one).
  The computer locks when no heartbeat came for its grace time (default 60 s), once, and
  waits for a heartbeat before it can lock again. It never unlocks. This only works if
  the heartbeats keep coming while the phone sits idle with its screen off: Android's
  Doze stops background networking, so run them from a foreground service with its own
  quiet notification ("Keeping fedora locked when you leave") while any computer
  watches, or ask for the battery-optimization exemption. Heartbeats that stop while
  the phone is on the desk would lock the computer for nothing.

  **Texting.** The computer asks; the phone answers with the matching message, within
  15 s (60 s for `sms_send`), or the computer says the phone didn't answer. Threads
  newest first; messages newest first, at most `limit` (≤ 100), `more` when older ones
  exist (the computer pages back with `before_ms` = the oldest date it has). `names`
  follow `addresses` (empty when unknown or without contacts permission). After sending,
  answer `sms_sent` and also push the sent message as `sms_message`; push every incoming
  text as `sms_message` to computers that are connected (no need to dial for it: the
  computer asks again when its Messages page opens). Nothing is stored on the computer.

- **Bidirectional stream = one file transfer** (`TransferMsg`), opened by the sender:

  1. Sender → `{"type":"offer","id":"<uuid>","files":[{"path":"Photos/a.jpg","size":123}]}`
     Add `"import":true` for **photo auto-import**: the computer saves the files under
     its Pictures folder, in `Pictures/<phone name>/<path>`, without asking, or answers
     `decline` right away when it doesn't take imports (the user turned it off there).
     Use paths like `2026-10/IMG_0001.jpg` so photos sort by month.
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
{"type":"session_accept"}
{"type":"session_decline"}                          // the user said no
{"type":"session_decline","reason":"unsupported"}  // this device can't do that kind
```

A device that **can't** do a kind (a phone asked to be controlled before that's built)
must answer with `reason: "unsupported"`, through core `node.decline_unsupported(request)`
instead of `node.respond(request, false)`. The desktop then says "realme can't be
controlled yet" instead of "didn't allow it", and a refused control request keeps the
screen view running. Unknown `reason` values read as a plain decline.

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
- **`camera` (phone as webcam):** the desktop sends `session_request` with
  `"kind":"camera"`; the phone asks its user ("Studio wants to use your camera"), then
  streams its camera exactly like a screen: H.264 **Constrained Baseline**, Annex-B,
  **1280×720 at 30 fps**, keyframe every 2 s, SPS/PPS before each keyframe. The desktop
  decodes it and offers it to every app as a PipeWire camera named
  "Linksy Camera (<phone name>)". On Android: CameraX `Preview`/`ImageAnalysis` or
  Camera2 into a `MediaCodec` input surface; keep it running in a foreground service with
  the camera type while the session lasts; offer front/back switch and keep the phone's
  screen dimmed. Nothing comes back on the viewer's half.
- Core API: `node.request_session(id, SessionKind::Screen | Control | Display | Camera) -> Session` (read
  `session.recv`), and `node.incoming_sessions()` → accepted `Session`s to feed
  (`session.send`). Events: `session_request {request, from, kind}`,
  `session_started {session, peer, kind, sharing}`, `session_ended {session}`.


**`input` sessions (desktop 0.4.1): the phone as touchpad, keyboard and presentation
remote.** The *phone* requests `{"type":"session_request","id":…,"kind":"input"}`; the
computer asks its user, then answers `session_accept`. No media flows: the phone writes
framed `ControlMsg`s on its send side, as a `control` viewer does, until it finishes the
stream. Two messages exist for this:

```json
{"type":"pointer_by","dx":12.5,"dy":-3.0}   // move by pixels (touchpad)
{"type":"text","text":"hello"}              // type text (keyboard field, IME)
```

plus the usual `button`, `scroll` (two fingers) and `key` (X11 keysyms: Right 0xff53 /
Left 0xff51 or Page_Down 0xff56 / Page_Up 0xff55 for slides, Return 0xff0d, BackSpace
0xff08, Escape 0xff1b). Absolute `pointer` messages are ignored (there's no screen).
The computer shows "<phone> is your touchpad and keyboard" with Stop.

### 4.9 Browsing files ("phone in the Files sidebar") — `src/fs.rs`

A computer browses the phone's files with one request per **bidirectional stream**. The
first frame is the request, tagged `"type":"fs"` and an `op`:

```json
{"type":"fs","op":"list","path":"/DCIM"}
{"type":"fs","op":"stat","path":"/DCIM/IMG_1.jpg"}
{"type":"fs","op":"read","path":"/DCIM/IMG_1.jpg","offset":0,"length":1048576}
{"type":"fs","op":"write","path":"/Download/a.pdf","offset":0,"length":1048576,"truncate":true}
// …followed by exactly `length` raw bytes
{"type":"fs","op":"mkdir","path":"/Download/New"}
{"type":"fs","op":"delete","path":"/Download/New"}        // folders with their contents
{"type":"fs","op":"rename","path":"/a.txt","to":"/b.txt"}
{"type":"fs","op":"info"}                                  // optional → {"type":"fs_info","name":"Documents"}
```

The phone answers one frame (then, for `fs_data`, exactly that many raw bytes) and
finishes the stream:

```json
{"type":"fs_entries","entries":[{"name":"DCIM","dir":true,"size":0,"modified_ms":0}]}
{"type":"fs_entry","entry":{"name":"IMG_1.jpg","dir":false,"size":2400000,"modified_ms":1760945000000}}
{"type":"fs_data","length":1048576}
{"type":"fs_ok"}
{"type":"fs_error","message":"Permission denied","not_found":false}
```

Paths are absolute within what the phone shares, `/` its root (shared storage,
`/storage/emulated/0`). Reads and writes are at most 4 MiB; the computer uses 1 MiB.
`stat "/"` must answer a folder. A phone that doesn't call `node.incoming_fs()` answers
"this device doesn't share its files" automatically, and the computer tells the user to
turn sharing on in the phone app. In Rust: `node.incoming_fs()` gives `FsCall`s; answer
each with `entries` / `entry` / `data` / `ok` / `error`, and read a write's bytes with
`body()` first.


### 4.10 Features: what each device can do (`DeviceInfo.features`)

`DeviceInfo` has an optional `features` list (in `hello`/`welcome`), so the other side can
hide what a device can't do. Names: `files`, `clipboard`, `open_url`, `battery`, `ring`,
`media_remote` (controls the other's players), `media_state` (shares its own players),
`call_state`, `heartbeat`, `import`, `fs`, `input`, `screen`, `control` (can be
controlled), `display`, `camera`, `watch`, `notifications` (mirrors its notifications),
`sms`. Set `Config.features` in the core. A desktop announces all of them. When a device
sends no list (older versions), the desktop assumes everything except, for
`os == "android"`, `sms`, `notifications`, `media_state` and `control`.

## 5. Core API the app would call (`linksy_core::Node`)

```text
Node::start(Config) -> Node                 Config { data_dir, download_dir, name, kind, port, bind, discovery, stay_connected, idle_after }
node.info() -> DeviceInfo                   node.subscribe() -> stream of Event
node.devices() -> [DeviceView]              DeviceView { device, trusted, connected, nearby }
node.pair_device(id) / node.pair(addr)      node.respond(request_id, accept)
node.send_files(id, paths) -> transfer_id   node.cancel(transfer_id)
node.import_files(id, paths) -> transfer_id // photo auto-import (offer with "import")
node.send_clipboard(id, text)               node.notify(id, title, body)
node.open_url(id, url)                      // Event::OpenUrl { from, url } on the receiver
node.send_battery(id, Battery { level, charging })   node.ring(id, on)   // Event::Battery / Event::Ring
DeviceView.battery: Option<Battery>         // last level each device reported
node.post_notification(id, RemoteNotification{…})   node.remove_notification(id, key)
node.send_media_state(id, &MediaState{ seq, players })   node.media_command(id, player, cmd)
node.watch_media(id, on)                    // Event::MediaState / MediaCommand / MediaWatch
node.send_call_state(id, CallState::Ringing | Active | Idle, contact)
node.heartbeat(id)                          // Event::HeartbeatWatch { from, on, every_s } says when
node.sms_threads(id, threads) / node.sms_messages(id, thread, messages, more)   // answers
node.sms_message(id, thread, message) / node.sms_sent(id, client_id, ok, error)
// to act on: Event::SmsListThreads / SmsListMessages / SmsSend
Event::NotificationReply / NotificationAction / NotificationDismissed { from, key, … }   // to act on
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
- Screen windows fit the shared screen's shape (a phone's screen gets a tall, narrow
  window, refitted on rotation), like iPhone Mirroring. A phone sharing its screen should
  keep sending its real aspect ratio and orientation; don't letterbox on the phone side.
- When the desktop uses the phone's camera, the desktop shows "Camera Is On · Click to
  preview" with Preview / Stop. The phone side should show a matching persistent
  "<desktop> is using your camera" notification with Stop.
- An outgoing transfer says "Waiting for <device> to accept…" until the other side
  answers, then shows progress.
- Received files can be opened by tapping the finished transfer (desktop also allows
  dragging them out).

## 7. Open items affecting mobile

Phone-side fixes found testing with a realme 15 Pro 5G (2026-10-19):

- [ ] **Screen video has black strips.** `ScreenShare.fit` rounds *both* sides down to a
      multiple of 16 independently, so 1272×2800 becomes 864×1920 (0.450) for a 0.454
      screen, and `VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR` letterboxes the difference inside
      the video. Keep the exact aspect: pick the width (multiple of 16), then
      `height = round(width × h / w)` rounded to an **even** number (H.264 needs even
      sizes; encoders pad to 16 internally and signal the crop). Check
      `MediaCodecInfo.VideoCapabilities.isSizeSupported(w, h)`, stepping the width down
      by 16 until supported. The desktop window already fits the stream exactly.
- [ ] **Answer unsupported kinds with `decline_unsupported`** (above), exposed through the
      bridge, instead of `respond(false)` in `RustNode`.
- [ ] **`control`: let the desktop drive the phone.** An `AccessibilityService`
      (`canPerformGestures`) applies `pointer`/`button`/`scroll` messages with
      `dispatchGesture`: `button left pressed` + `pointer` moves + `released` = one
      stroke (tap if short and still, drag otherwise), `scroll` = a two-finger swipe,
      `key` = `performGlobalAction` for Back/Home (keysyms Escape → back, Super/Home →
      home) and text via `AccessibilityNodeInfo.ACTION_SET_TEXT` on the focused field.
      The user enables the service once in Settings → Accessibility; until then answer
      `control` with `decline_unsupported`.
- [ ] **`display`: be the desktop's second screen.** Accept `display` requests (prompt
      "fedora wants to use this screen as a second display"), go immersive full screen
      in landscape, decode the stream with `MediaCodec` into a `SurfaceView`, and send
      taps as `pointer` + `button left`, two-finger drags as smooth `scroll`. The desktop
      currently sends 1920×1080; matching the phone's own size is planned.

- [ ] Core: send from file descriptors / readers (for `content://` URIs).
- [ ] Core: receive through a callback or into a caller-provided sink (MediaStore).
- [x] Core: inject discovered devices from the platform (`device_found` / `device_lost`).
- [x] QR pairing format and desktop side (§4.7). The phone side (scanner) is to build.
- [ ] Phone showing a QR for a desktop to scan: no desktop camera flow planned; use the
      phone as the scanner.
- [x] Brand color: Signal.

Found testing "Use as Second Display" on the realme (2026-10-20): the phone turned
landscape but stayed black while GNOME had the extra monitor. The desktop's stream was
fine (recorded with `fake_phone --display`, it decodes to the desktop's wallpaper), but
x264 cut every frame into ~12 slices, and `DisplayReceiver` feeds MediaCodec one NAL unit
per input buffer, each with its own timestamp, so the decoder never had a whole picture.
Desktop 0.2.3 sends one slice per frame, which fixes it with today's receiver. To be
robust against any encoder (GPU ones may slice too):

- [ ] **Feed MediaCodec whole access units.** The desktop's stream has an access unit
      delimiter (NAL type 9) before every frame: collect NALs until the next type 9 and
      queue them as one buffer, one timestamp per access unit. Send SPS+PPS (types 7, 8)
      together as one `BUFFER_FLAG_CODEC_CONFIG` buffer. Also set
      `KEY_MAX_INPUT_SIZE` (e.g. 2 MiB) when configuring, so a big keyframe always fits.
- [ ] **Use the phone's shape.** The virtual monitor is 1920×1080 today, letterboxed on a
      2800×1272 screen. Proposal (needs a small core change, not done yet): the phone
      reports its landscape size when accepting a `display` session and the desktop
      creates the monitor at that size (scaled to fit 1080 px high).

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
- **2026-10-19** — Phone as webcam, desktop side (§4.8): session kind `camera`; the
  desktop publishes the stream as a PipeWire camera. Media: 720p30 Constrained Baseline.
- **2026-10-19** — Desktop UX: screen windows fit the shared screen's shape; camera preview card. No protocol change.
- **2026-10-19** — `session_decline` may carry `reason: "unsupported"`; core
  `decline_unsupported`. Phone-side to-dos from real-device testing in §7 (screen aspect,
  control via AccessibilityService, display mode).
- **2026-10-20** — Desktop 0.1.1 ships as Flatpak, .deb and .rpm too. No protocol or core
  API change; a phone sees a Flatpak desktop exactly like any other.
- **2026-10-20** — Desktop app ID is now `mg.diary.linksy.Linksy` (site: linksy.diary.mg).
  Nothing on the wire changes. If the phone app has no ID yet, `mg.diary.linksy` fits.
- **2026-10-20** — `ROADMAP.md` lists the next features (0.2–0.5) with the phone-side
  work for each. Nothing to build yet; protocol details land here as each one starts.
- **2026-10-20** — **Open on PC** (desktop 0.2.0): `open_url` message (§4.5), core
  `node.open_url(id, url)` and `Event::OpenUrl`. Phone to-do: an "Open on PC" target in
  the Android share sheet for `text/plain` shares that are links (`ACTION_SEND`), with a
  device picker when several computers are paired; plain text can go as `clipboard`.
  The phone may also open links a computer sends (`Event::OpenUrl`), with a tap to confirm.
- **2026-10-20** — **Battery & Ring** (desktop 0.2.1): `battery`, `ring`, `ring_stop`
  (§4.5); core `send_battery`, `ring`, `DeviceView.battery`, `Event::Battery`,
  `Event::Ring`. Phone to-do: listen to `ACTION_BATTERY_CHANGED` and send `battery` to
  paired computers that are around (on connect, ±5 points, plug/unplug); on `ring`, play
  the alarm at full volume even on silent with a full-screen "Found it" (stops on
  `ring_stop` or after 30 s); a "Find my computer" button that sends `ring`. Test against
  a desktop without the phone: `cargo run -p linksy-core --example fake_phone` acts as a
  phone (see the file).
- **2026-10-20** — **Phone notifications with reply** (desktop 0.2.2): `notification_posted`,
  `notification_removed` (phone → computer), `notification_reply`, `notification_action`,
  `notification_dismiss` (computer → phone), §4.5; core `post_notification`,
  `remove_notification` and the matching events. Phone to-do: a
  `NotificationListenerService` (onboarding explains and opens the special-access
  setting); for each posted notification that isn't ongoing, a group summary, low
  importance or Linksy's own, send `notification_posted` to paired computers that are
  around (send `notification_removed` from `onNotificationRemoved`); answer replies via the
  `RemoteInput` action, actions via their `PendingIntent`, dismissals via
  `cancelNotification(key)`. A per-app on/off list on the phone is a nice extra. Test with
  `fake_phone --notify "Alice" "Lunch?"` against a computer to see the desktop side.
- **2026-10-20** — Desktop 0.2.3: x264 now sends one slice per frame (the black second
  display on the realme). §7 has the receiver change that makes this robust.
- **2026-10-20** — **Media controls** (desktop 0.3.0): `media_state`, `media_command`,
  `media_watch` (§4.5) and their core calls. Phone to-do: with notification access
  (already needed for notifications), `MediaSessionManager.getActiveSessions` +
  `MediaController.Callback` give the phone's players: send `media_state` on changes,
  act on `media_command` via `MediaController.transportControls`. For the computer's
  players, a media-style notification ("Spotify on fedora") with play/pause/next, shown
  while `media_watch` is on. Try it against a computer with
  `fake_phone --media "Song" "Artist"` (appears in GNOME's media controls) and
  `fake_phone --watch-media --press play-pause`.
- **2026-10-20** — **Pause on calls** (desktop 0.3.1): `call_state` (§4.5), core
  `send_call_state`. Phone to-do: `TelephonyCallback.CallStateListener`
  (READ_PHONE_STATE) mapping RINGING → `ringing`, OFFHOOK → `active`, IDLE → `idle`;
  include the contact's name only with contacts permission. Try with
  `fake_phone --call 5` while something plays on the computer.
- **2026-10-20** — **Lock when you walk away** (desktop 0.3.2): `heartbeat_watch`,
  `heartbeat` (§4.5), core `heartbeat`, `watch_heartbeat`. Phone to-do: honor
  `heartbeat_watch` with a foreground service (see §4.5 on Doze), one schedule per
  computer. Test with `fake_phone --leave-after 30` and the switch on the phone's page
  (the computer really locks; set `LINKSY_LOCK_COMMAND` on a test service to fake it).
- **2026-10-20** — **Photo auto-import** (desktop 0.4.0): offers can carry
  `"import": true` (§4.5); core `import_files`. The computer takes imports by default
  (Preferences, "Import Photos From Phones"). Phone to-do: a setting per computer
  ("Back up new photos to fedora"); watch `MediaStore` (images and videos) with a
  `ContentObserver` or a periodic `WorkManager` job; when that computer is around and the
  phone is on Wi-Fi (optionally only while charging), send what's new with
  `import_files`, paths `YYYY-MM/<name>`; remember what went where so nothing is sent
  twice (resume makes retries cheap). A declined import means that computer doesn't
  want them: say so in the setting. Try with `fake_phone --import <folder>`.
- **2026-10-20** — **Touchpad, keyboard and presentation remote** (desktop 0.4.1):
  session kind `input` and `ControlMsg` `pointer_by` / `text` (§4.8). Phone to-do: a
  "Remote" screen per computer: a touchpad area (one finger moves with `pointer_by`,
  tap = left click, two-finger tap = right click, two fingers scroll), a keyboard button
  that opens the IME and sends `text` (and `key` for Backspace/Enter), and a
  presentation mode where volume down/up send Right/Left and the screen stays on.
  Try with `fake_phone --touchpad` against a service started with `LINKSY_SCREEN_TEST=1`
  (it logs the input instead of applying it).
- **2026-10-20** — **Texting from the desktop** (desktop 0.5.0): `sms_*` messages (§4.5)
  and their core calls. Phone to-do: READ_SMS, SEND_SMS, RECEIVE_SMS (and READ_CONTACTS
  for names) behind an onboarding switch, since APKs from GitHub may ask for them;
  answer `sms_list_threads` from `Telephony.Threads`/`Sms.Conversations`,
  `sms_list_messages` from `Telephony.Sms` (MMS text parts are a bonus); `sms_send` via
  `SmsManager.sendMultipartTextMessage` with a sent `PendingIntent` that answers
  `sms_sent`; a `RECEIVE_SMS` receiver (or ContentObserver) pushes `sms_message`. Try it
  against a computer: `fake_phone` answers with a made-up inbox, and the computer's
  `linksy texts <phone>` / `linksy text <phone> <number> <message>` exercise the rest.
- **2026-10-20** — **Phone in the Files sidebar** (desktop 0.5.1): `fs` request streams
  (§4.9), core `node.fs()` / `node.incoming_fs()`. The computer mounts the phone as a
  WebDAV place (GVfs) named after it, so it's in Files and every file dialog. Phone
  to-do: a "Let <computer> browse my files" switch (off by default) that starts serving
  `incoming_fs()`; `MANAGE_EXTERNAL_STORAGE` ("All files access") is the simplest for a
  GitHub APK, or the storage access framework for chosen folders only; map `/` to shared
  storage; answer reads with `RandomAccessFile` (videos seek); run it from the foreground
  service while a computer has it open. Try against a computer with
  `fake_phone --files <folder>` and its "Browse Files" row.
- **2026-10-21** — `Config` has an `os` field (defaults to the build target, so "android"
  on the phone with no change). Only the fake phone sets it.
- **2026-10-21** — Desktop 0.5.3: "Finish Setup" installs what the computer lacks; with
  v4l2loopback the phone's camera becomes a "Linksy Camera" V4L2 device that browsers and
  Zoom list. Nothing changes for the phone.
- **2026-10-21** — Read `docs/android-app.md`. Desktop 0.5.4: `DeviceInfo.features` (§4.10; the
  phone can now send its list), the desktop hides Messages, Take Control, phone
  notification settings and phone media for phones without them, and `fs` has an
  optional `info` request whose `fs_info {name}` labels the Files bookmark
  ("Pixel 9 (Documents)"). §2 now shows the phone's real status.
