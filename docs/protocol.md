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

Android column from the phone's `docs/android-app.md` (LinkSy-mg/linksy-mobile, phone 0.1.16, 2026-10-21).
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
| Images/files on the clipboard | Done | Done | offer with `"clipboard":true` (§4.5) |
| Automatic clipboard sync, computer → phone | Done (not on GNOME) | Done (phone asks) | `clipboard_watch` (§4.5) |
| Phone as microphone | Done | Done | session `microphone`, raw PCM (§4.8) |
| USB cable (no Wi-Fi, lower latency) | Done | Built, untested on hardware | Android Open Accessory + UDP bridge (§4.11) |
| Camera controls from the computer | Done | Done | §4.8; docs/android-app.md §2.8 |
| Device pictures | Done | Done | §4.5; docs/android-app.md §2.7 |
| Away from home (over the internet) | Done | Done | iroh, Linksy's relay, 7-day lease (§4.13) |
| Locate the phone | Done | Done (while the app is open) | `locate` / `location` (§4.14) |
| 1080p webcam | Done | Done | `size` in `camera_control`, `size_options` (§4.8) |
| Terminal on the computer, from the phone | Done (0.7.0) | To do | session `terminal`, code set on the computer (§4.15) |
| Bluetooth LE presence | Not planned | Not planned | |

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
  {"type":"clipboard_watch","on":true}          // send me your clipboard text as it changes
  {"type":"picture","image":"<base64 JPEG or PNG, ≤ 128 KiB>"}  // the sender's own picture
  {"type":"camera_state","session":"<uuid>","state":{…}}        // §4.8, phone → computer
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
     Add `"clipboard":true` to put **one file (≤ 10 MiB)** on the receiver's clipboard:
     an image goes on as an image (pastes into chats and editors), anything else as a
     file (pastes into the file manager). Taken without asking; more than one file or
     a bigger one is declined. The receiver shows no download for it.
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


**Keep the screen on while it's a display (desktop 0.5.8; seen on a realme 15 Pro).** The
phone's screen timeout turned its screen off a minute into a second display, which ended
the session and took the monitor away. While showing a `display` (and while watching a
`screen`/`control`), keep the screen on: `window.addFlags(FLAG_KEEP_SCREEN_ON)` on that
activity (Flutter: a wakelock package, or the flag through a platform channel), cleared
when the session ends. Also don't end the session in `onPause`/`onStop` just because the
user switched apps for a moment; end it on Stop or when the computer ends it. If the
session does drop, the computer now offers "Reconnect" (a new `display` request: answer
it like the first). For `camera` and `microphone` the phone's screen may go off, but
their foreground services must keep running.

**Second display in the phone's shape (desktop 0.5.5).** Accepting a `display`, the phone
may add its landscape screen size: `{"type":"session_accept","width":2400,"height":1080}`
(core: `node.respond_session(request, true, Some((w, h)))`). The computer then makes the
virtual monitor in that shape, at most 1080 px high (2800×1272 → 2376×1080). Without it,
1920×1080 as before.

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

**Camera controls from the computer (desktop 0.5.7, `docs/android-app.md` §2.8).** While a
`camera` session runs, the computer writes `camera_control` frames on its send side
(u32 BE length + JSON), exactly the keys the phone already reads. The phone sends what it
can do and how it's set as a **connection message** (§4.5), at the start and after every
change, whoever made it:

```json
{"type":"camera_state","session":"<session uuid>","state":{…the object in docs/android-app.md §2.8…}}
```

Core: `node.send_camera_state(computer_id, session.id, state_json)`;
`Event::CameraState { from, session, state }` on the computer. The computer shows the
controls under its camera preview: front/back (when both `lenses` exist), the
`zoom_presets` as chips, torch (`has_torch`), click on the preview → `focus_point`
(+ `"focus":"point"`, and an Auto Focus button when the focus isn't `auto`), and in a
menu `exposure`, `white_balance`, `quality`, `orientation`, `stabilization`
(`has_stabilization`), and **60 fps** (when `fps_options` has 60). It redraws from every
`camera_state`, so the phone's own controls and the computer's stay in step.

**60 fps (desktop 0.5.9, `docs/android-app.md` §2.8).** The computer's webcam device advertises
the rate the phone sends. When `fps` changes, whether the computer sent
`{"type":"camera_control","fps":60}` or the phone's own control did it (seen in
`camera_state.fps`), the computer restarts its webcam at the new rate within the same
session, and decoding resumes at the phone's next keyframe with SPS/PPS (which the phone
already sends after the change). Apps using the camera see it blink once. So the phone
must keep reporting `fps` in every `camera_state`.

**1080p (desktop 0.6.0, `docs/android-app.md` §2.8 "later").** Same mechanism for the size: the
computer sends `{"type":"camera_control","size":[1920,1080]}` (or `[1280,720]` back),
and follows `width`/`height` from `camera_state`, restarting its webcam device at that
size within the session. Only 1280×720 and 1920×1080. **Phone to-do:** list what the
camera can stream in `camera_state` as `"size_options":[[1280,720],[1920,1080]]` (the
computer shows its "1080p" switch only then), restart the encoder at the new size on
`size` (SPS/PPS and a keyframe after, as for `fps`), report the new `width`/`height`,
and raise the bitrate (about 1.5× at the same `quality`).

**Webcam latency (desktop 0.5.7).** The computer now decodes with slice threading
instead of libav's frame threading, which held one frame per CPU thread: about 400 ms
less on a 12-thread laptop (measured: 404 ms → 37 ms from receiving a frame to
showing it). On the phone, to keep its part small:
- `MediaFormat.KEY_LATENCY = 1` (API 30+) and `KEY_LOW_LATENCY = 1` where the codec has
  it (`FEATURE_LowLatency`); `KEY_PRIORITY = 0` (realtime); no B-frames
  (Constrained Baseline already has none); `KEY_MAX_B_FRAMES = 0` to be sure.
- Write each encoded buffer to the stream as soon as `dequeueOutputBuffer` returns it;
  don't batch. An `AUD` (`00 00 00 01 09 f0`) before each access unit lets the computer
  end a frame without waiting for the next one's start code (about one frame, 33 ms).
- Keep `KEY_REPEAT_PREVIOUS_FRAME_AFTER` short (e.g. 100 ms) so a still picture keeps
  flowing.
- Over USB (§4.11) Wi-Fi's jitter goes away too.

**`microphone` sessions (desktop 0.5.7): the phone as the computer's microphone.** The
computer sends `{"type":"session_request","id":…,"kind":"microphone"}`; the phone asks
its user ("Studio Laptop wants to use your microphone"), then writes **raw PCM**: 16-bit
signed little-endian, **48 000 Hz, mono**, no header, no framing, in small chunks (10 ms
= 960 bytes; don't buffer more than 20 ms). About 100 KB/s. Nothing comes back. The
computer offers it to every app as a microphone, "Linksy Microphone (<phone name>)", and
drops audio that arrives more than 100 ms late rather than letting delay pile up. On
Android: `AudioRecord` with `MediaRecorder.AudioSource.VOICE_COMMUNICATION` (echo
cancellation and noise suppression on most phones) or `MIC`, `ENCODING_PCM_16BIT`,
`CHANNEL_IN_MONO`, 48 kHz (resample if the device refuses), in a foreground service of
type `microphone` (`FOREGROUND_SERVICE_MICROPHONE`, plus `RECORD_AUDIO` at run time; both
fine for a sideloaded app). Announce `microphone` in `features` (§4.10); until then the
computer doesn't offer it for Android phones. Core: `SessionKind::Microphone`; the phone
writes to `session.send`. Desktop to desktop works too (`linksy mic <device>`).

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
`sms`, `clipboard_file` (takes `"clipboard":true` offers), `clipboard_watch` (answers
`clipboard_watch`), `microphone` (shares its microphone), `picture` (sends and shows
`picture`). Set `Config.features` in the core. A desktop announces all of them,
except `clipboard_watch` where its desktop doesn't let apps watch the clipboard (GNOME).
When a device sends no list (older versions), the desktop assumes everything except, for
`os == "android"`, `sms`, `notifications`, `media_state`, `control`, `clipboard_file`,
`clipboard_watch` and `microphone`.

### 4.11 USB cable (desktop 0.5.7)

Lower latency and no Wi-Fi needed, for the webcam and microphone above all, with **no
developer options**: the computer uses **Android Open Accessory (AOA 1.0)**.

1. When an Android phone is plugged in (and an Android phone is paired), the computer
   asks once per phone ("Pixel 9 is plugged in. Use it with Linksy over the cable?").
   On yes it sends AOA's control requests (51 get protocol, 52 strings, 53 start) with
   **manufacturer `Linksy`, model `Linksy`**, description "Linksy over USB", version
   `1` (the framing below), URI `https://linksy.diary.mg`. The phone re-enumerates as an
   accessory (Google's 18d1:2d00–2d05).
2. **Phone app:** declare the accessory in the manifest
   (`<uses-feature android:name="android.hardware.usb.accessory"/>`, an activity with
   the `USB_ACCESSORY_ATTACHED` intent filter and `res/xml/accessory_filter.xml`:
   `<usb-accessory manufacturer="Linksy" model="Linksy"/>`). Android then offers to
   open Linksy; "always" makes it automatic. Open it with
   `UsbManager.openAccessory(accessory)` → a `ParcelFileDescriptor` with an input and an
   output stream.
3. **What crosses the cable: Linksy's own QUIC packets**, unchanged and still encrypted
   end to end, each as a **2-byte big-endian length + the UDP payload**. On the phone, a
   tiny bridge: bind a UDP socket on `127.0.0.1:0` (port P); for each datagram the
   phone's node sends to it, write `len + bytes` to the accessory; for each `len + bytes`
   read from the accessory, send the bytes back to the node's address (learned from the
   first datagram). Batch several frames in one write when they're waiting.
4. **Then dial the computer through it:** `node.connect(127.0.0.1:P)` (the core's normal
   reconnect, to a known paired device). The handshake checks the computer's pinned key
   as always. On the computer the packets arrive from `127.0.0.1`, which is how both
   sides know this connection is the cable: **a USB connection replaces a Wi-Fi one** to
   the same device (both cores agree without talking: the one over loopback wins), and
   its address is never saved as the device's address. `DeviceView.usb` is true for it;
   the computer shows "Connected by USB".
5. Unplugging ends the accessory streams → the connection drops → the normal Wi-Fi
   reconnect takes over. Start sessions after plugging in: a running session doesn't
   move to the cable.

Linux needs permission to switch the phone and to use it as an accessory: the .deb and
.rpm install a udev rule (`70-linksy-usb.rules`); Finish Setup adds it otherwise (not
possible from the Flatpak). Try the bridge without a phone: `cargo test -p linksyd usb`
runs QUIC through an in-memory cable between two nodes.

### 4.12 Service information (planned for 0.6)

`https://linksy.diary.mg/linksy-relay` is a small JSON file the apps read: **information
only**, never something to execute. It's rebuilt with the website, so editing it needs
no app release (in linksy-site: `src/data/service.ts`).

```json
{
  "schema": 1,
  "generated": "2026-10-21T11:40:54Z",
  "service": { "name": "Linksy", "website": "…", "privacy": "…", "issues": "…", "releases": "…" },
  "relays": [ { "url": "https://linksy-relay.diary.mg", "health": "https://linksy-relay.diary.mg/ping" } ],
  "protocol": { "current": 1, "minimum": 1 },
  "latest": {
    "linux": { "version": "0.5.7", "notes": "…", "downloads": { "flatpak": "…", "deb": "…", "rpm": "…", "tarball": "…" } },
    "android": { "download": "…/linksy.apk" }
  },
  "notices": [ { "id": "…", "level": "info|warning", "title": "…", "body": "…", "platforms": ["android"], "until": "ISO 8601" } ],
  "stats": { "downloads": { "linux": 2, "android": 15, "total": 17 }, "devices_online": 0, "updated": "…" }
}
```

How the apps use it:
- **When:** at start-up and at most once a day, over HTTPS, with a 5 s timeout. Cache the
  last good copy; offline or on any error, use the cache, then the defaults built into
  the core (the relay `https://linksy-relay.diary.mg`). Never block anything on it.
- **`relays`:** away-from-home access (0.6) uses the first relay whose `url` is `https`;
  the others are fallbacks. A device's own relay is what it tells its peers. A relay set
  in Preferences (self-hosting) always wins.
- **`latest`:** "Linksy X is available" when newer than this app, with a link to `notes`
  or the download. Never downloads or installs anything by itself.
- **`protocol.minimum`:** above this app's protocol version, say "Update Linksy to keep
  connecting"; otherwise ignore.
- **`notices`:** show each once (remember `id`), as plain text (no markup, no links
  made clickable beyond https), only for listed `platforms`, not after `until`.
- **`stats`:** totals for display only (an About page, say): downloads from GitHub and
  devices connected through the relay right now. Either may be `null`. There is no user
  count: Linksy has no accounts and sends nothing about its users anywhere.
- **Robustness:** ignore unknown fields; skip the whole file if `schema` isn't 1, if it
  isn't valid JSON, or if it's over 64 KiB. What it says can't reach anyone's data: a
  wrong relay can only fail to connect (everything is end-to-end encrypted with keys
  pinned at pairing).

### 4.13 Away from home (desktop 0.6.0)

Paired devices reach each other over the internet, through **iroh** (`iroh` 1.4, already
in `linksy-core`): a direct path when the networks allow it, else through Linksy's relay
`https://linksy-relay.diary.mg` (§4.12 lists it; `linksy-server`). Everything stays
Linksy's own QUIC, encrypted end to end with the keys pinned at pairing: iroh only
carries its packets, like the USB cable (§4.11).

- **In the core, nearly all done.** Set `Config.away = true` and `Config.relay =
  Some(<first relays[].url from §4.12, cached; else https://linksy-relay.diary.mg>)`. The
  node then makes an away key (`away.key` in the data dir), puts its away address in
  `DeviceInfo.away` (`{"id","relay","addrs"}`, in the handshake only), takes away
  connections from paired devices, and **falls back to away by itself** when a device
  doesn't answer on the network (`ensure_connection`). `node.connect_away(id)` forces it.
- **Links.** Away connections come from `127.0.0.2` (the bridge), USB from other loopback
  addresses, the network from anywhere else: `link_of(addr)` / `node.link(id)` →
  `Link::{Usb, Lan, Away}`. A better link replaces a worse one to the same device (both
  sides agree), and finding the device on the network again replaces away.
  `DeviceView.away` is true when connected away.
- **The lease: 7 days.** A device may connect from away for a week after it was last
  connected on the same network or by cable (`TrustedDevice.last_met`, renewed on every
  such connection; pairing counts). After that its away connections are refused until
  they meet again. `DeviceView.away_until` says until when, and is `None` until the
  device's away address is known (they met on the same network with both running 0.6) or
  while away from home is off here: only show "Reachable from away" when it's set.
- **What's refused from away** unless the user allowed everything for that device
  (`node.set_away_all(id, true)`, `TrustedDevice.away_all`): every media or input session
  (screen, control, display, camera, microphone, input), answered
  `{"type":"session_decline","reason":"away"}` (`Error::NotFromAway` for the requester),
  and file browsing (`fs`, answered with an error). Always allowed: clipboard, links,
  files sent (asked as usual), ring, battery, notifications, media controls, locate.
- **Lock When Away** must ignore heartbeats that arrive from away (`node.link(id) ==
  Some(Link::Away)`): far away is not nearby. The computer does.
- **Settings:** `Settings.away` (default on) turns it off entirely.

**Phone to-do:** turn it on in `Config` (above); a Settings switch "Away from home"
(`Settings.away`) and, per computer, "Allow everything from away" (`set_away_all`);
show "Connected away from home" when `DeviceView.away`, and "Reachable from away until
<date>" from `away_until`. With "Keep running in the background" the phone stays
reachable from away; otherwise it connects when opened. Test against a computer:
`fake_phone --away` plays the phone, and the core test
`away_from_home_connects_and_keeps_screens_local` shows the whole flow.

### 4.14 Locate (desktop 0.6.0)

The computer asks a phone where it is, once; nothing is kept and there's no tracking.

```json
{"type":"locate"}                                                     // computer → phone
{"type":"location","lat":-18.8792,"lon":47.5079,"accuracy_m":15,"at_ms":1792...}  // phone → computer
{"type":"location_refused","reason":"Location is off on this phone"}  // or this
```

Core: `Event::LocateRequest { from }` on the phone; answer with `node.send_location(from,
lat, lon, accuracy_m, at_ms)` or `node.refuse_location(from, reason)` within 30 s. The
computer shows the position with an "Open Map" link (OpenStreetMap). **Phone to-do:** a
setting "Let computers locate this phone" (off by default); when on, take one fresh fix
(`FusedLocationProviderClient.getCurrentLocation`, or `LocationManager`), which needs
`ACCESS_FINE_LOCATION` granted while in use (and `ACCESS_BACKGROUND_LOCATION` for when the
app is in the background; fine for a sideloaded app, and the setting explains why);
otherwise refuse with a reason. Announce `locate` in `features` (§4.10); the computer
only offers it then. It works away from home too (§4.13).

### 4.15 Terminal (desktop 0.7.0)

A real shell on the computer, shown on the phone: Claude Code, neovim, htop, git, builds.
**The phone opens it; the computer never opens one on the phone.** Built on the desktop
in 0.7.0 exactly as below (core: `node.request_terminal(to, TerminalOpen { code, cols,
rows, resume })` returns the `Session` and the terminal id; `node.list_terminals(to)` and
`Event::Terminals`). Test against a computer with `fake_phone --terminal CODE` (`--type`,
`--resume`, `--close`). Not in the desktop's Flatpak (its shell would be the sandbox's):
those computers don't announce `terminal`.

**Safety, in layers (all enforced by the computer, never by the phone):**

1. Paired devices only, end to end encrypted as everything else.
2. **Off by default, per phone**: "Allow Terminal" on the computer, for that phone.
3. **A terminal code set on the computer** (Preferences › Terminal: 6+ digits or a
   passphrase). The computer keeps only a hash (Argon2id). The phone asks for it **every
   time a terminal opens** and never stores it (no "remember me", no biometric shortcut).
   5 wrong codes in a row lock the terminal for that phone until it's unlocked on the
   computer, which says so in a notification.
4. **The computer asks its user** the first time each day ("Pixel 9 wants a terminal",
   Allow / Don't Allow), as for screens.
5. **Not from away** (§4.13) unless "Allow Everything From Away" is on for that phone.
6. Visible while open: a notification on the computer ("Terminal open from Pixel 9",
   End), and each opening in the activity history.
7. The user's normal account, never root (`sudo` still asks the computer's password).
   Unpairing ends that phone's terminals at once.

**Feature:** computers that can do it announce `"terminal"` in `features` (§4.10); the
phone shows "Terminal" only for them. Whether it's allowed for *this* phone is only known
by asking (it can change any time).

**Opening (a media-style session, §4.8; the phone is the requester):**

```json
{"type":"session_request","id":"<uuid>","kind":"terminal",
 "code":"<the code as typed>","cols":80,"rows":24,
 "resume":"<terminal id>"}                    // optional: reattach (below)
```

Answers:

```json
{"type":"session_accept","terminal":"<terminal id>"}
{"type":"session_decline","reason":"off"}      // terminal not allowed for this phone
{"type":"session_decline","reason":"code"}     // wrong code; "tries_left": 3
{"type":"session_decline","reason":"locked"}   // too many wrong codes; unlock on the computer
{"type":"session_decline","reason":"away"}     // from away and not allowed
{"type":"session_decline","reason":"limit"}    // 8 terminals already run for this phone
{"type":"session_decline"}                     // the computer's user said no
{"type":"session_decline","reason":"unsupported"}
```

Show each reason as such ("Wrong code, 3 tries left", "Turn on Allow Terminal for this
phone on the computer", "Unlock the terminal on the computer"…).

**After `session_accept`:**

- **Computer → phone:** the raw terminal output (bytes from the pseudo-terminal, UTF-8
  with escape sequences), no framing, until the stream ends. Feed it straight to the
  terminal view. The shell runs with `TERM=xterm-256color`, `COLORTERM=truecolor`, in the
  user's home, as their login shell.
- **Phone → computer:** framed JSON (u32 BE length + JSON, as `control` sessions):

  ```json
  {"type":"term_input","data":"ls -la\r"}      // what was typed, as text (Ctrl+C is "\u0003")
  {"type":"term_resize","cols":64,"rows":30}    // on rotation, keyboard shown or hidden, font size
  {"type":"term_close"}                         // end the shell (the computer then ends the stream)
  ```

  Keys map to the usual xterm sequences: Enter `\r`, Backspace `\u007f`, Tab `\t`, Esc
  `\u001b`, arrows `\u001b[A`…`\u001b[D`, Ctrl+letter = letter & 0x1f, Alt+key = `\u001b`
  then the key. Mouse reporting and bracketed paste are whatever the program asks for;
  send them as `term_input` too.

**Closing, the rules both sides keep (so nothing is left hanging):**

- **Closing a terminal on the phone** (its tab's close button, `exit` typed in it): send
  `term_close`. The computer ends the shell and everything started in it, then finishes
  the stream. Nothing keeps running.
- **Leaving it** (switching apps, the phone locking, the network dropping, the app being
  killed): just finish the stream, or let it drop. The shell stays, detached (next point).
- **Ended on the computer** (End on the phone's page in Linksy there, the code turned
  off, the phone no longer allowed, or unpaired): the stream finishes right away. Show
  "Ended on the computer" and drop that tab; `resume` won't find it.
- **The shell exits by itself**: the stream finishes too; close the tab.
- Once a terminal's stream ends, the session is gone on both sides: the connection goes
  back to idle and closes as usual (§4.6) when nothing else uses it.

**Terminals outlive the connection.** Finishing the stream *without* `term_close`
(network change, app closed, phone locked) only detaches: the shell keeps running on the
computer for up to 24 hours. Reattach with `"resume":"<terminal id>"` (the code is asked
again); the computer redraws the screen on attach. Several terminals at once are fine
(tabs), each its own session. To list the running ones:

```json
{"type":"terminal_list"}                                            // phone → computer (connection message, §4.5)
{"type":"terminals","list":[{"id":"…","title":"claude — ~/Project","started_ms":1792…}]}
```

`title` follows the shell's window title (OSC 0/2). A terminal whose shell exits finishes
its stream and leaves the list.

**Phone to-do:** a terminal view (xterm.dart or similar) with true colour, mouse and
resize; a key bar above the keyboard (Esc, Ctrl, Alt, Tab, arrows, `|`, `~`, `/`, `-`);
the code prompt before each open (numeric or text keyboard, masked); tabs for several
terminals and reattach from `terminal_list`; a clear "Detached, still running on the
computer" state. Until the desktop ships it, computers don't announce `terminal`.

## 5. Core API the app would call (`linksy_core::Node`)

```text
Node::start(Config) -> Node                 Config { data_dir, download_dir, name, kind, port, bind, discovery, stay_connected, idle_after }
node.info() -> DeviceInfo                   node.subscribe() -> stream of Event
node.devices() -> [DeviceView]              DeviceView { device, trusted, connected, nearby }
node.pair_device(id) / node.pair(addr)      node.respond(request_id, accept)
node.send_files(id, paths) -> transfer_id   node.cancel(transfer_id)
node.import_files(id, paths) -> transfer_id // photo auto-import (offer with "import")
node.send_clipboard(id, text)               node.notify(id, title, body)
node.send_clipboard_file(id, path) -> transfer_id   // Event::ClipboardFile { from, path } on the receiver
node.watch_clipboard(id, on)                // Event::ClipboardWatch { from, on } on the receiver
node.send_picture(id, base64)               // Event::Picture { from, image } on the receiver
node.send_camera_state(id, session, json)   // Event::CameraState { from, session, state }
node.connect(addr)                          // USB: dial the computer through the bridge (§4.11)
node.connect_away(id)  node.link(id) -> Link  node.set_away_all(id, on)   // away from home (§4.13)
node.locate(id)  node.send_location(id, lat, lon, acc, at_ms)  node.refuse_location(id, why)  // §4.14
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
- **2026-10-21** — Desktop 0.5.5: second display in the phone's shape: `session_accept`
  may carry `width`/`height`, core `respond_session` (§4.8). `docs/android-app.md` §2.1 is done on
  this side; the phone can send its size.
- **2026-10-21** — Desktop 0.5.6, `docs/android-app.md` §2.3/§2.4: **images and files on the
  clipboard**: offers may carry `"clipboard":true` (one file ≤ 10 MiB, §4.5), core
  `send_clipboard_file`, `Event::ClipboardFile`; the computer's Send Clipboard sends a
  copied image or file this way to devices with `clipboard_file`. **Clipboard watch**:
  `{"type":"clipboard_watch","on":true}`, core `watch_clipboard`; while on, the computer
  sends `clipboard` each time its clipboard text changes. Only where the desktop allows
  it (KDE, wlroots; not GNOME), announced as the `clipboard_watch` feature: check it
  before offering the setting. Phone to-do: announce `clipboard_file` once a received
  `ClipboardFile` lands on the phone's clipboard (`ClipData` with a `content://` URI from
  a FileProvider); its share sheet can send images this way. Try with
  `fake_phone --clip-file <image>` and `fake_phone --watch-clipboard`.
- **2026-10-21** — Desktop 0.5.7: **lower webcam latency** (the computer's decoder no
  longer holds frames: about 400 ms less; phone-side tips in §4.8). **USB cable** (§4.11):
  Android Open Accessory, manufacturer/model `Linksy`, QUIC packets framed as 2-byte
  length + payload, a loopback connection wins over Wi-Fi; the phone needs the accessory
  filter and the small UDP bridge. **Microphone** sessions (§4.8): raw 16-bit 48 kHz mono
  PCM; announce `microphone`. From `docs/android-app.md`: **§2.8 camera controls** (the computer
  sends `camera_control`; the phone sends `camera_state` with core `send_camera_state`)
  and **§2.7 pictures** (`picture` message, base64 JPEG/PNG ≤ 128 KiB, sent after
  connecting; the computer sends the user's account picture and shows the phone's as its
  avatar). Try with `fake_phone --camera --picture <png>` and `linksy mic <device>`.
- **2026-10-21** — Planned for 0.6 (nothing to build yet): §4.12 service information at
  `https://linksy.diary.mg/linksy-relay` (relays, latest versions, protocol minimum,
  notices), read at start-up and cached; the relay server is `../linksy-server`.
- **2026-10-21** — Desktop 0.5.8: keep the screen on while showing a second display (§4.8;
  it dropped after the screen timeout on a realme 15 Pro); the computer offers
  "Reconnect" when a display drops. Desktop also checks `/linksy-relay` (§4.12) once a day
  for updates now; the phone can do the same with `latest.android`.
- **2026-10-21** — Desktop 0.5.9: 60 fps webcams (`docs/android-app.md` §2.8): the computer follows
  `fps` from `camera_state` or its own `camera_control`, restarting its webcam device at
  that rate in the same session; a "60 Frames per Second" switch in its camera settings.
- **2026-10-21** — Desktop 0.6.0: 1080p webcams (§4.8): `camera_control {"size":[w,h]}`,
  `camera_state` gains `size_options`; the computer follows `width`/`height`.
- **2026-10-21** — Desktop 0.6.0: **away from home** (§4.13): iroh in the core carries
  Linksy's QUIC between paired devices over the internet, through Linksy's relay when
  needed; 7-day lease renewed on the same network or cable; screens, control, display,
  camera, microphone, input and file browsing refused from away unless allowed per device
  (`reason: "away"`); `Config.away`/`relay`, `Settings.away`, `DeviceInfo.away`,
  `Link`, `DeviceView.away`/`away_all`/`away_until`. **Locate** (§4.14): `locate`,
  `location`, `location_refused`. `Message` lost `Eq` (positions are floats).
- **2026-10-21** — Desktop 0.6.1: `DeviceView.away_until` is `None` until the device's
  away address is known (and while away from home is off), so "Reachable from away"
  isn't shown before the two have met on the same network running 0.6. Phone: bump
  the core and keep showing the line only when `away_until` is set; nothing else changes.
- **2026-10-21** — Desktop 0.6.3: the screen viewer has a tip for Android phones (the
  info button in its header): "Top bar black? … Developer options › Disable screen share
  protections, then share again", answering docs/android-app.md §2.2. Desktop 0.6.2 redesigned
  the device page; nothing changes in the protocol.
- **2026-10-21** — §4.15 Terminal: the contract for a shell on the computer from the
  phone (session `terminal`, a code set on the computer, `term_input`/`term_resize`/
  `term_close`, detach and `resume`, `terminal_list`). Planned for desktop 0.7; spec
  first so the phone can build its side. §2 status refreshed from phone 0.1.16. Desktop
  0.6.4 saves received files to Downloads/Linksy, like the phone.
- **2026-10-22** — Desktop 0.7.0: the terminal (§4.15) is built: reason `limit`, the
  closing rules (`term_close` ends it, dropping the stream detaches, End on the computer
  finishes the stream), `fake_phone --terminal` to test. Finish Setup also opens the
  firewall (UDP 47200 and mDNS) on computers that have one.
- **2026-10-22** — LinkSy-mg/linksy-releases has a new front page (README) for both apps:
  Android downloads (the `/releases/latest/download/linksy*.apk` links, unchanged, so a
  phone release needs no README edit), Linux downloads (between `<!-- linux:start -->`
  and `<!-- linux:end -->`, rewritten by the desktop's release script), screenshots
  (the phone ones are the website's), features, privacy. Please edit it in place rather
  than replacing it, and keep "latest" on the APK releases.
