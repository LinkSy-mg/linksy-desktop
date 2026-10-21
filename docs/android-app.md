# Linksy for Android → desktop handoff

The phone side's answer to `docs/protocol.md` in linksy-desktop. It says what the Android app
does, what it needs from the desktop, and what it **can't** build, so the desktop doesn't
offer those features to Android phones.

- App: `app.linksy.mg` (since 0.1.8; 0.1.x up to 0.1.7 was `app.linksy`), Flutter + the
  real `linksy-core` through flutter_rust_bridge. Repo: LinkSy-mg/linksy-mobile.
- Core pinned at `e407e80` (desktop 0.5.2). Builds for arm64-v8a, armeabi-v7a, x86_64.
- Releases: LinkSy-mg/linksy-releases (`/releases/latest/download/linksy.apk`).
- Distributed as a sideloaded APK, not through the Play Store. That shapes everything in
  §3: Android's fraud protection blocks installing sideloaded apps that declare certain
  permissions, with no way for the user to skip it (v0.1.3 hit this).

## 1. What the phone does (protocol as in `docs/protocol.md`)

| Feature | Phone side | Notes for the desktop |
|---|---|---|
| Discovery, presence, pairing (code, address, QR link) | Done | QR: any camera app opens `linksy://pair…`; the in-app scanner is Camera2 + ZXing |
| Files both ways, accept/decline, cancel | Done | Incoming offers also show as a notification with **Accept / Decline** when the app isn't on screen |
| Received files | Moved to **Downloads/Linksy** (Android 10+) | Folder offers keep their shape: `Downloads/Linksy/Photos/a.jpg` |
| Clipboard, ping, rename | Done | Incoming clipboard is put on the phone's clipboard at once |
| Open on PC (`open_url`) | Done both ways | Share sheet: a link opens on the computer; other text goes as `clipboard`; files are sent |
| Battery (`battery`) | Done | Sent on connect, when the computer is found nearby, at ±5 points and on plug/unplug |
| Ring / Find (`ring`, `ring_stop`) | Done both ways | Alarm stream at full volume, "Found it" screen, 30 s max |
| Media: the computer's players on the phone | Done | Phone sends `media_watch {on}` while the computer's page is open, shows each player with play/pause/next/previous (`media_command`) |
| Calls (`call_state`) | Done | From Android's audio mode (no phone permission), so WhatsApp/VoIP calls count too. **`contact` is never sent** |
| Lock when away (`heartbeat_watch` / `heartbeat`) | Done | Heartbeats come from the core's runtime; a `connectedDevice` foreground service + partial wake lock keep them going with the screen off (tested in forced Doze). Only while the app process lives: send `heartbeat_watch` again on every connect |
| Photo auto-import (`import: true`) | Done | Per computer, off by default. Source is **a folder the user picks** (usually DCIM/Camera), not the whole gallery. Sends folders named `YYYY-MM` (so files land in `Pictures/<phone>/YYYY-MM/…`), ≤ 40 files / ~300 MB per offer, only photos taken after it was turned on. A declined import is shown as "turned off on the computer" |
| Files sidebar (`fs`, §4.9) | Done | `/` is **one folder the user picked** (Android's folder picker), not shared storage. Android refuses Download and the storage root there. Until a folder is chosen and the switch is on, the phone answers `fs_error` "This phone doesn't share its files…" |
| Touchpad / keyboard / slides (`input`) | Done | `pointer_by`, `button`, `scroll`, `text`, `key` (BackSpace, Return, Left/Right, F5, Escape). Volume keys change slides |
| Phone shares its screen (`screen`) | Done | Always the **entire screen** (Android 14+ otherwise preselects "a single app" and the bars come out black) |
| Phone as webcam (`camera`) | Done | 1280×720 at 30 fps, with full camera controls on the phone (lenses, zoom, torch, focus, exposure, white balance, stabilization, quality), live; the computer can send `camera_control` on the session (§2.8) |
| Phone as second display (`display`) | Done | Decodes whole access units (AUD / first-slice boundaries), so any slicing works |
| **Phone watches the computer's screen** (`screen`, and `control` with touch) | **New** | The phone *requests* `screen` or `control`. Touch in `control`: one finger = absolute `pointer` + left button held, two fingers = `scroll`. Uses the same receiver as `display` |
| Remote control *of the phone* (`control` asked by the computer) | Not possible | Answered with `decline_unsupported` at once; see §3 |
| Push notifications | Done | Firebase project `linksy-mg`, topic `all`. Nothing for the desktop to do |

The phone keeps running in the background only when a computer watches heartbeats, or when
the user turns on "Keep running in the background". Otherwise most things work while the
app is open.

## 2. What the phone needs from the desktop

1. **Second display at the phone's shape** (`docs/protocol.md` §7, still open). The phone
   letterboxes a 1920×1080 monitor on a ~20:9 screen. Proposal: the phone sends its
   landscape size when it accepts a `display` session:
   `{"type":"session_accept","width":2400,"height":1080}` (optional fields; older
   desktops ignore them). Core: `node.respond_session(request, accept, Some((w, h)))` or a
   `Session::size`. Desktop: create the virtual monitor at that aspect, scaled to at most
   1080 px high. The phone is ready to send it once the core has the field.
2. **Capabilities, so the desktop hides what a phone can't do.** Today the desktop finds
   out by trial: `sms_*` requests get no answer (the 15 s timeout), `control` gets
   `decline_unsupported`, notification and phone-media features simply never arrive.
   Proposal: an optional list in the handshake / `DeviceInfo`, for example
   `"features":["files","clipboard","open_url","battery","ring","media_remote",
   "call_state","heartbeat","import","fs","input","screen","camera","display","watch"]`.
   The desktop then greys out or hides Messages, notification mirroring, phone media,
   and "Control phone" for this app (see §3). Until then, please treat Android phones
   as not having those (`DeviceInfo.os == "android"`).
3. **Clipboard images and files** (planned on both sides). Proposal: an offer flag like
   `import`, `"clipboard": true`, one file ≤ 10 MiB, which the receiver puts on its
   clipboard instead of saving (images as image data, other files as a file
   reference). The phone side is small once the core has it.
4. **Automatic clipboard sync**, desktop → phone. Android lets an app read the clipboard
   only while it's on screen (Android 10+), so the phone can't watch its own clipboard.
   It can apply what arrives, at any time. Proposal: `clipboard_watch {on}` (like
   `media_watch`): while on, the desktop sends `clipboard` whenever its clipboard
   changes. Phone → desktop stays on demand (button, share sheet).
5. **`heartbeat_watch` on every connect.** The phone forgets watches when Android kills
   the app. The desktop already sends it when the phone connects; please keep that.
6. **`fs` labels.** The phone's `/` is a folder the user chose (often "Documents"), not
   the whole phone. Wording like "Linksy on Pixel (Documents)" would match. A small
   `fs_info` answer with the folder's name is easy to add on the phone if you want it.

7. **Device pictures (optional).** Since 0.1.10 the phone lets the user give each
   paired device a picture, shown as its avatar and as a banner on its page. It's
   local to the phone. If the desktop wants the same, a `picture` message (a small
   JPEG, base64, ≤ 128 KiB) that a device sends about *itself* after connecting would
   let both sides show each other's picture without setting it twice.

8. **Camera controls from the computer** (phone side done in 0.1.11). The phone's
   webcam has a camera app's controls: lenses (front, back and every back camera the
   phone exposes: ultra-wide, main, tele), zoom, torch, tap-to-focus and manual
   focus, exposure compensation, exposure and white-balance locks, white-balance
   presets, stabilization and stream quality. They work from the phone now; the
   computer can drive them too.

   **Computer → phone, already understood by the phone:** on the `camera` session,
   write framed JSON on the computer's send side (u32 big-endian length + body, as
   `ControlMsg` on a `control` session), at most 64 KiB each:

   ```json
   {"type":"camera_control","zoom":2.0}
   {"type":"camera_control","facing":"front"}
   {"type":"camera_control","lens":"2"}
   {"type":"camera_control","torch":true}
   {"type":"camera_control","focus_point":[0.42,0.61]}
   {"type":"camera_control","focus":"manual","focus_distance":0.3}
   {"type":"camera_control","focus":"auto"}
   {"type":"camera_control","exposure":2,"exposure_lock":false}
   {"type":"camera_control","white_balance":"daylight","white_balance_lock":true}
   {"type":"camera_control","stabilization":false}
   {"type":"camera_control","quality":"high"}
   {"type":"camera_control","orientation":"portrait"}
   {"type":"camera_control","fps":60}
   ```

   Any subset of the keys in one message; unknown keys are ignored.
   - `zoom`: a ratio within `zoom_min`..`zoom_max` (below 1 reaches the ultra-wide on
     phones whose main camera includes it). `facing`: `front` | `back`. `lens`: one of
     the `lenses[].id`; switching resets zoom, focus and exposure.
   - `focus`: `auto` (continuous) | `point` (with `focus_point`, held there) |
     `manual` (with `focus_distance` 0 = far … 1 = nearest). `focus_point` is
     `[x, y]`, 0..1 across and down **the picture as streamed** (not mirrored), and
     also meters exposure there.
   - `exposure`: compensation steps in `exposure_min`..`exposure_max`; in EV that's
     `exposure * exposure_step`.
   - `white_balance`: `auto` | `daylight` | `cloudy` | `shade` | `incandescent` |
     `fluorescent` (only those in `white_balances`).
   - `quality`: `saver` (2 Mbps) | `standard` (4) | `high` (8), applied live; at 60
     fps each gets half as much again (3 / 6 / 12).
   - `fps`: `30` (default) | `60`, offered only when the camera does 60 at a usable
     size (`fps_options`). The phone restarts its encoder at the new rate: the
     stream goes on with SPS/PPS and a keyframe, same 1280×720. **Desktop:** the
     virtual webcam is offered as YUY2 at 30 fps today; at 60 it should either
     advertise 60 fps or drop every other frame.
   - `orientation`: `landscape` (default: the picture fills the 16:9 frame, upright
     with the phone held sideways either way round) | `portrait` (an upright
     portrait picture with black bars at the sides, phone held upright). The stream
     keeps its 1280×720 size either way. `rotation` in the state is the clockwise
     turn applied now (0/90/180/270). `focus_point` is on the stream as shown.
   - Each change takes effect on the running stream: no restart. A lens or facing
     change is followed by a keyframe with SPS/PPS.

   **Phone → computer, needs the core:** what the camera can do and its current
   values, so the computer can draw controls and follow changes made on the phone.
   The phone has it as one JSON object (the same keys) and needs a connection
   message to send it, for example `{"type":"camera_state","session":"<uuid>",
   "state":{…}}` with `Event::CameraState { from, session, state }`, sent when the
   session starts and after every change. The state:

   ```json
   {"width":1280,"height":720,"facing":"back","lens":"0",
    "lenses":[{"id":"0","facing":"back","zoom":1.0,"main":true},
              {"id":"2","facing":"back","zoom":0.6,"main":false},
              {"id":"1","facing":"front","zoom":1.0,"main":true}],
    "zoom":1.0,"zoom_min":0.6,"zoom_max":10.0,"zoom_presets":[0.6,1,2,5,10],
    "torch":false,"has_torch":true,
    "focus":"auto","focus_modes":["auto","point","manual"],"focus_distance":0.0,
    "focus_point":null,
    "exposure":0,"exposure_min":-12,"exposure_max":12,"exposure_step":0.1667,
    "exposure_lock":false,
    "white_balance":"auto","white_balances":["auto","daylight","cloudy","shade",
     "incandescent","fluorescent"],"white_balance_lock":false,
    "stabilization":true,"has_stabilization":true,"quality":"standard",
    "orientation":"landscape","rotation":0,
    "fps":30,"fps_options":[30,60],"camera_width":1280,"camera_height":720}
   ```

   Suggested desktop UI: a camera panel next to "Use as Webcam" with the zoom chips
   (`zoom_presets` plus the other `lenses`), a torch toggle, click-to-focus on the
   preview, an exposure slider and white-balance menu, mirroring the phone's screen.

   The stream is always 1280×720. Each camera delivers its own size (`camera_width`
   × `camera_height`, informative); the phone fits it into the stream without
   stretching (bars if its shape differs).

   **Later, with the desktop:** 1080p would need the webcam device on the computer
   to follow a size change; the phone can restart its encoder at a new size if the
   desktop can take it (`camera_control {"size":[1920,1080]}` is the proposed
   shape).

## 3. What the phone can't build (please don't offer these for Android)

| Feature | Why not |
|---|---|
| **Texting from the computer** (`sms_list_threads`, `sms_list_messages`, `sms_send`, `sms_message`) | Needs READ_SMS / RECEIVE_SMS / SEND_SMS. Android's fraud protection blocks installing sideloaded apps that declare SMS permissions; Play also restricts them to default SMS apps |
| **Phone notifications on the computer**, reply, actions, dismiss (`notification_*`) | Needs a NotificationListenerService (`BIND_NOTIFICATION_LISTENER_SERVICE`), on the same install-block list, and "restricted settings" blocks enabling it for sideloaded apps |
| **The phone's own music on the computer** (`media_state` from the phone, `media_command` to it) | `MediaSessionManager.getActiveSessions` requires notification-listener access (see above) |
| **Controlling the phone from the computer** (`control` requested by the computer) | Injecting touch needs an AccessibilityService, also on the install-block list (it's what blocked v0.1.3/0.1.4). Answered with `decline_unsupported` |
| **Caller's name in `call_state`** | Would need READ_CONTACTS plus call permissions; the phone reads only the audio mode, which has no number. `contact` is always absent |
| **Browsing the whole phone** in the Files sidebar | Needs MANAGE_EXTERNAL_STORAGE ("All files access"), an alarming special permission for a sideloaded app; Android's picker won't grant Download or the root. Only one chosen folder is shared |
| **Watching the phone's clipboard** (auto sync phone → computer) | Android 10+ only lets the app on screen read the clipboard |
| **Everything with Linksy fully closed** | Without the foreground service (lock when away, or "Keep running in the background"), Android stops the app; transfers and requests then wait for the app |

If Linksy ever ships through the Play Store, the notification, media and control rows
become possible again, but each needs a Play policy declaration, and SMS only for a
default SMS app.

## 4. Not done yet on the phone (possible, just not built)

- Bluetooth LE presence (planned on both sides; needs the nearby-devices runtime
  permission, which is allowed).
- NsdManager discovery fallback for networks that drop multicast (mDNS from the core
  works on the networks tested; pairing by QR or address covers the rest).

## Changelog

- **2026-10-21** — Webcam: 30 or 60 fps (`fps`), the phone fits each camera's real
  output size into the 1280×720 stream (fixes a stretched picture on phones whose
  camera doesn't offer 1280×720), `orientation` landscape | portrait.
- **2026-10-21** — §2.8 camera controls: `camera_control` on the camera session
  (the phone understands it now) and the proposed `camera_state` message.
- **2026-10-21** — First version: phone status up to 0.1.8+ (watching the computer's
  screen, camera switch, Accept/Decline notifications, Downloads), requests to the
  desktop (§2), and what Android won't allow (§3).
