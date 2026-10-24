# Linksy for Android → desktop handoff

The phone side's answer to `docs/protocol.md` in linksy-desktop. It says what the Android app
does, what it needs from the desktop, and what it **can't** build, so the desktop doesn't
offer those features to Android phones.

- App: `app.linksy.mg` (since 0.1.8; 0.1.x up to 0.1.7 was `app.linksy`), Flutter + the
  real `linksy-core` through flutter_rust_bridge. Repo: LinkSy-mg/linksy-mobile.
- Core pinned at `6282520` (desktop main with private discovery), as of phone 0.2.4. Builds for arm64-v8a, armeabi-v7a, x86_64.
- Releases: LinkSy-mg/linksy-releases, titled "Android X", APKs named `linksy-X.apk` / `-arm64` / `-armv7` with a `SHA256SUMS` (the app's Update Now checks against it); the newest is always https://linksy.diary.mg/download/android.
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
| Phone as webcam (`camera`) | Done | 1280×720 or **1920×1080** (`size`; in `size_options` only when the camera really delivers 1920-wide 16:9), 30 or 60 fps, full controls on the phone; `camera_control` from the computer and **`camera_state` to it** on every change. Low latency: `KEY_LATENCY`/`KEY_LOW_LATENCY` 1, realtime priority, no B-frames, an **AUD before every access unit** |
| Phone as microphone (`microphone`) | Done | Asks the user, then Android's `RECORD_AUDIO`; `AudioRecord` VOICE_COMMUNICATION (MIC as fallback), 48 kHz mono s16le, 960-byte writes, in the share service (type `microphone`), so it goes on with the screen off. Measured 96 kB/s |
| Phone as second display (`display`) | Done | Accepts with its **landscape size** (e.g. 2400×1080); keeps the screen on while showing (and while watching the computer's screen); doesn't end on pause. Decodes whole access units |
| Device pictures (`picture`) | Done | Sends "Picture of this phone" (Settings) as a JPEG ≤ 128 KiB after each connect and on change, to devices announcing `picture`. A computer's own picture shows for it unless the user chose one on the phone |
| Files on the clipboard (`clipboard_file`) | Done | A received clipboard file goes on Android's clipboard as a `content://` URI (not into Downloads). Share sheet: one file ≤ 10 MiB → "Put it on the computer's clipboard" (computers announcing `clipboard_file`) |
| Clipboard watch (`clipboard_watch`) | Done (phone asks) | Per computer, "Get its clipboard" on its page, only when the computer announces `clipboard_watch`; sent again on every connect while on. The phone doesn't announce `clipboard_watch` itself (§3) |
| Locate (`locate`) | Done | "Let computers locate this phone", off by default. On: one fix (one under a minute old, else `getCurrentLocation`, 20 s max), foreground location only. Else `location_refused` with a reason to show as is ("Locating is turned off in Linksy on this phone", "Location is off on this phone", "Open Linksy on the phone to share where it is"…) |
| Away from home | Done | `Config.away` on, relay from the service file (§4.12, cached, fetched at most daily; default `https://linksy-relay.diary.mg`). Settings switch (`Settings.away`), "Relay server" (`Settings.relay`, the user's own, wins over the service file), per computer "Allow everything from away" (`set_away_all`), "Connected away from home", "Reachable from away until <day>" |
| USB cable (AOA) | Built, untested on hardware | Accessory filter `Linksy`/`Linksy`; UDP bridge on 127.0.0.1, 2-byte BE length framing, batched writes; then `connect_address(127.0.0.1:P)`. "Connected by USB" from `DeviceView.usb` |
| Service file (§4.12) | Done | `android` notices on the home screen, each until dismissed; "Update Linksy to keep connecting" when `protocol.minimum` > 1 |
| `features` | Announced | `files clipboard open_url battery ring media_remote call_state heartbeat import fs input screen display camera watch clipboard_file microphone picture locate` |
| `fs` `info` | Done | `fs_info` with the picked folder's name |
| **Phone watches the computer's screen** (`screen`, and `control` with touch) | **New** | The phone *requests* `screen` or `control`. Touch in `control`: one finger = absolute `pointer` + left button held, two fingers = `scroll`. Uses the same receiver as `display` |
| Remote control *of the phone* (`control` asked by the computer) | Not possible | Answered with `decline_unsupported` at once; see §3 |
| Push notifications | Done | Firebase project `linksy-mg`, topic `all`. Nothing for the desktop to do |

The phone keeps running in the background only when a computer watches heartbeats, or when
the user turns on "Keep running in the background". Otherwise most things work while the
app is open.

## 2. What the phone needs from the desktop

Nothing open. Everything asked here before (display in the phone's shape, `features`,
clipboard files and watch, `heartbeat_watch` on connect, `fs` labels, pictures, camera
controls, `camera_state`, 1080p) is done on both sides as of desktop 0.6.0 and phone
0.1.15; the protocol lives in `docs/protocol.md` (§4.5, §4.8–§4.14). Worth checking on the
desktop side with a real phone:

1. **USB cable: the realme blocker is addressed in phone 0.2.2** (please retry on the
   realme 15 Pro 5G). The phone no longer relies on `USB_ACCESSORY_ATTACHED` or the
   "open with" prompt: while Linksy is open it looks at `UsbManager.accessoryList`
   (on resume and every 1.5 s), also listens for attached/detached at run time, and
   asks `requestPermission` when it has none (once per plug if refused). The bridge is
   opened after Flutter's channel exists, so its port always reaches the dial. Logcat
   tag `linksy` / `flutter`: "usb cable: asking permission", "permission granted",
   "bridge on 127.0.0.1:P", "dialing", "connected" or "dial failed (…)", "unplugged".
   If no paired computer answers through the cable within 6 s, the phone offers to
   **pair over the cable** (`pair(127.0.0.1:P)`, then connects); once connected it
   says "Connected to <name> by USB cable" and the computer's page shows the cable.
   Android only switches apps on its own while Linksy is in the background if the
   system's prompt opened it: with Linksy open in front the cable is taken within
   ~1.5 s, well inside the 17 s.
2. **Terminal (§4.15): done on the phone in 0.2.0**, on core `e6957b4` (desktop 0.7.0):
   `request_terminal`, the session's output to the terminal view, `term_input` /
   `term_resize` / `term_close` frames, `list_terminals` + `Event::Terminals` (5 s
   wait), every decline reason including `limit`. Closing follows your rules: Close
   (button or the tab's ×, confirmed) sends `term_close` then finishes our side; `exit`
   or End on the computer finishes the stream and the phone, finding it gone from
   `terminal_list`, drops the tab ("… ended on <computer>"); leaving or a dropped link
   only detaches (if the list can't be fetched, the tab stays detached). Tested
   end to end against a stand-in computer on the 0.7 core hosting a real login shell
   (code, wrong code with tries, detach/reattach, exit, close). Not yet tried against
   linksyd itself: please run a real phone against it.
3. **Black status bar / notification shade when watching an Android phone.** Android
   15's screen share protections, which some phones (realme, seen on a realme 15 Pro)
   extend to the whole status bar and notification shade, so those come out black in
   the stream. No app can capture them; the user must turn on **Developer options ›
   "Disable screen share protections"** and share again. The phone (0.1.16) says so
   under its sharing banner while the protection is on. Please add a quiet tip in the
   desktop's viewer for Android phones too, e.g. under the video or in its ⋯ menu:
   "Top bar black? On the phone: Developer options › Disable screen share
   protections, then share again."
4. **Locate** works only while Linksy is open on the phone (no background location, to
   keep the sideloaded APK's permissions small); show the phone's `location_refused`
   reason as it is.

## 3. What the phone can't build (please don't offer these for Android)

| Feature | Why not |
|---|---|
| **Texting from the computer** (`sms_list_threads`, `sms_list_messages`, `sms_send`, `sms_message`) | Needs READ_SMS / RECEIVE_SMS / SEND_SMS. Android's fraud protection blocks installing sideloaded apps that declare SMS permissions; Play also restricts them to default SMS apps |
| **Phone notifications on the computer**, reply, actions, dismiss (`notification_*`) | Needs a NotificationListenerService (`BIND_NOTIFICATION_LISTENER_SERVICE`), on the same install-block list, and "restricted settings" blocks enabling it for sideloaded apps |
| **The phone's own music on the computer** (`media_state` from the phone, `media_command` to it) | `MediaSessionManager.getActiveSessions` requires notification-listener access (see above) |
| **Controlling the phone from the computer** (`control` requested by the computer) | Injecting touch needs an AccessibilityService, also on the install-block list (it's what blocked v0.1.3/0.1.4). Answered with `decline_unsupported` |
| **Caller's name in `call_state`** | Would need READ_CONTACTS plus call permissions; the phone reads only the audio mode, which has no number. `contact` is always absent |
| **Browsing the whole phone** in the Files sidebar | Needs MANAGE_EXTERNAL_STORAGE ("All files access"), an alarming special permission for a sideloaded app; Android's picker won't grant Download or the root. Only one chosen folder is shared |
| **Watching the phone's clipboard** (auto sync phone → computer) | Android 10+ only lets the app on screen read the clipboard, so the phone doesn't announce `clipboard_watch` |
| **Locating the phone in the background** | Needs `ACCESS_BACKGROUND_LOCATION`; left out to keep the APK's permissions small. Works while Linksy is open |
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

- **2026-10-24** — Phone 0.2.4 on core `6282520`: private discovery (§4.16), with
  pairing mode while Add Device, the scanner or Pair by address is open; Update Now
  (newest `vX.Y.Z` on linksy-releases, checked against its `SHA256SUMS`, installed by
  Android's installer); What's New the first time a version opens. No protocol change
  beyond §4.16.

- **2026-10-23** — Phone 0.2.3 on core `293c7bc`: the relay setting (PR #7, with
  desktop PR #31), each paired device as a direct target in Android's share sheet,
  the photo picker for many photos at once, a transfers bubble with a page per file,
  an upright webcam picture when the phone stands, and less memory in the background
  (`docs/memory.md`). No protocol change.

- **2026-10-22** — Phone 0.2.2 (desktop 0.8.0): the USB cable is found without the
  system's prompt (accessory list, run-time receiver, permission request), its port
  always reaches the dial, every step is logged, and an unpaired computer on the cable
  is offered for pairing over it (§2.1). Strings quoting the desktop use its names
  ("Add Device", Preferences › Terminal, "Allow Everything From Away", "Import Photos
  From Phones") in each of the 10 languages.

- **2026-10-22** — Phone 0.2.0 on core `e6957b4` (desktop 0.7.0): the terminal works
  end to end (§2.2), with the closing rules of docs/protocol.md §4.15.

- **2026-10-22** — Phone 0.1.19 on core `61b08e0` (desktop 0.6.4): the Terminal's phone
  side (§2.2), with what the core needs for it. Phone 0.1.17 redesigned home and the
  computer page (battery ring, link chip, "This phone as", grouped switches, activity
  filters); no protocol change.

- **2026-10-21** — Phone 0.1.16: explains Android 15's screen share protection (black
  status bar and shade on some phones) under the sharing banner; §2 asks for the same
  tip in the desktop's viewer.
- **2026-10-21** — Phone 0.1.15 on core `1c8ebe4` (desktop 0.6.0): `features`,
  microphone, display in the phone's shape with the screen kept on, `camera_state` to
  the computer, 1080p, low-latency encoder with AUDs, pictures, clipboard files and
  watch, locate, away from home, the service file, the USB accessory bridge,
  `fs_info`. §2 has nothing open.
- **2026-10-21** — Webcam: 30 or 60 fps (`fps`), the phone fits each camera's real
  output size into the 1280×720 stream (fixes a stretched picture on phones whose
  camera doesn't offer 1280×720), `orientation` landscape | portrait.
- **2026-10-21** — §2.8 camera controls: `camera_control` on the camera session
  (the phone understands it now) and the proposed `camera_state` message.
- **2026-10-21** — First version: phone status up to 0.1.8+ (watching the computer's
  screen, camera switch, Accept/Decline notifications, Downloads), requests to the
  desktop (§2), and what Android won't allow (§3).
