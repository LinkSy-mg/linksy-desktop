# Linksy for Android → desktop handoff

The phone side's answer to `docs/protocol.md` in linksy-desktop. It says what the Android app
does, what it needs from the desktop, and what it **can't** build, so the desktop doesn't
offer those features to Android phones.

- App: `app.linksy.mg` (since 0.1.8; 0.1.x up to 0.1.7 was `app.linksy`), Flutter + the
  real `linksy-core` through flutter_rust_bridge. Repo: LinkSy-mg/linksy-mobile.
- Core pinned at `760cb98` (desktop main, deck layouts), as of phone 0.3.4. Builds for arm64-v8a, armeabi-v7a, x86_64.
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
| The computer's volume (`volume_watch`, `volume_state`, `volume_set`) | Done (0.2.6) | Only for computers announcing `volume`. Watched with the players; a slider and mute under them, the phone's volume keys step it 5 % while that page is in front |
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
| USB cable (AOA) | Built, untested on hardware | Accessory filter `Linksy`/`Linksy`; Kotlin opens the accessory and hands `detachFd()` and `accessory.version` to `linksy_core::cable::phone_bridge` (framing version 2 when the computer announces it); then `connect_address(127.0.0.1:P)`, up to 3 tries, and pairing over the cable only if the computer says `not_paired` or nobody answers. "Connecting over the USB cable…" while it dials, then that computer's page (from the home screen). "Connected by USB" from `DeviceView.usb`. Each cable step (permission, open, bridge, unplugged) goes to the Logs page |
| Service file (§4.12) | Done | `android` notices on the home screen, each until dismissed; "Update Linksy to keep connecting" when `protocol.minimum` > 1 |
| `features` | Announced | `files clipboard open_url battery ring media_remote call_state heartbeat import fs input screen display camera watch clipboard_file microphone picture locate deck_layout`, plus `notifications` and `inbox_history` while notifications go to the computers and `gallery` while it's on, changed live with `set_features` |
| `fs` `info` | Done | `fs_info` with the picked folder's name |
| Phone notifications on the computer (`notification_*`, §4.5) | Done (0.3.2) | A `NotificationListenerService` in the app, on once the user allows Notification access (Android 13+: guided through Allow restricted settings). Inbox fields: `category`, `conversation` (never the user's own name), `sender`, `mine`, `group`, `time`, `picture`, `messages` (up to 25, oldest first, from `getMessages` and `getHistoricMessages`), `subject` for mail (`CATEGORY_EMAIL`). Android 15 hides some content from listeners it doesn't trust (`Ranking.hasSensitiveContent`): those go as plain notifications |
| Choosing where received files go | Done | Settings › Files › Save location opens Android's folder picker (`pickFolder('received')`, a read and write grant, so no new permission). `Downloads.publish` writes there through the document tree, keeping subfolders; a folder that's gone falls back to Downloads/Linksy. `forgetFolder` goes back to the default. Android doesn't offer the Download folder itself in the picker, only folders inside it |
| Turning on the Inbox and Gallery | Done | Without digging through Settings: the first-run setup page's rows "Messages on your computers" and "Photos on your computers", and a one-time home card for people updating (pref `turn_on_card`: gone when dismissed or both are on; setup marks it done). `phone_notifications` is on unless `off`; nothing goes until Notification access is allowed. Once, at start (`inbox_default`): no pref with access already given means the switch was turned off, so it's written `off`. On Android 13+ the restricted-settings steps come first, and start with tapping the greyed-out switch: App info only has ⋮ › Allow restricted settings after Android refused once (the app op goes `deny`/default → `ignore` → `allow`) |
| Answering after the notification is gone (`reply_kept`) | Done | A conversation's reply action is kept for a day after its notification goes; `notification_reply` uses it, and forgets it once the app cancels the intent |
| Earlier messages (`inbox_history`) | Done | What the listener saw, per app and `conversation`: the newest 300 messages of the 300 most recent conversations, in Linksy's own files; forgotten when the user turns notifications on the computers off. Announced with `notifications` |
| Gallery (`gallery`, §4.18) | Done | Announced while Linksy may read photos (`READ_MEDIA_IMAGES`/`READ_MEDIA_VIDEO`, or All files access) and Settings › Files › Photos on your computers is on. Straight from MediaStore: newest first by `COALESCE(DATE_TAKEN, DATE_ADDED×1000)` then `_ID`; albums are MediaStore buckets ("Phone storage" for the root); thumbnails from `loadThumbnail`, JPEG ≤ 256 KiB; `send` reads the file itself when it may, else a cache copy; `gallery_changed` from a MediaStore observer, at most every 2 s |
| Deck (`deck`, §4.19) | Done | Device page › Control › Deck, for computers that announce `deck`. `deck_open` while it shows, `deck_close` when it goes or the app leaves the screen; a slider's value at most every 80 ms while dragged, the last one always; a toggle flips at once and `deck_state` follows. An action with `confirm` asks that question first (Cancel, or the action's `name`) and runs only on yes; a state's `text` shows on its tile (`sound.output`'s output). Group names translated, `Network` and `Power` included. A computer that sends no layouts (an older desktop) gets the phone's own: pages of tiles (small, wide, tall, big, full row), 8 colours, 32 icons, made from the first `deck_actions`, after 1.5 s without `deck_layout` |
| Deck layouts (`deck_layout`, §4.20) | Done | Announced always. Each `deck_layout` replaces the last, kept per computer with its `version` in the app's files (deleted when the computer is forgotten), shown straight away next time and while the computer is away, its tiles dimmed until it's back. The `active` layout, the face for how the phone is held (the other one when that face has no tiles), pages swiped sideways, square cells as big as the grid fits. Kinds: `button` (toggle `on`, state `text`), `volume` (the slider, across or up a 1 × 3), `music` (`media_state`: cover on 4 × 2, title, artist, progress, previous/play/next as `media_command`), `clock` (the phone's time, 24-hour as the phone has it, its date; `props.zone` not drawn); any other kind a plain tile with its label; a tile whose action isn't in `deck_actions` dimmed, and a tap says so. Themes `stock`, `dots`, `eink`, `term`, `pixel` drawn as the desktop's editor draws them (its icons' SVG paths, dotted or 12 × 12 pixel icons, 5 × 7 glyphs of dots or squares, dotted or block bars, covers in dots or the 16-colour palette), an unknown theme as `stock`; and the phone's own `signal`, `glass` and `midnight`. Holding (or tapping) the deck's name picks one, kept on the phone per computer and layout until "As on <computer>" is picked again. While the deck shows: `FLAG_KEEP_SCREEN_ON`, the window's `screenBrightness` at 0.2 after 60 s untouched (never above the user's own when set by hand), a touch brings it back (that touch runs nothing), leaving gives the screen back |
| Files tab (§4.9) | Done | Eight threads answer `fs` streams at once; `..` or `.` in a path, and deleting or moving `/`, get `fs_error` |
| **Phone watches the computer's screen** (`screen`, and `control` with touch) | **New** | The phone *requests* `screen` or `control`. Touch in `control`: one finger = absolute `pointer` + left button held, two fingers = `scroll`. Uses the same receiver as `display`. **Choose Screen** in the corner ends the session and requests it again with `"choose": true` (§4.8, desktop#15) |
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
| **The phone's own music on the computer** (`media_state` from the phone, `media_command` to it) | `MediaSessionManager.getActiveSessions` requires notification-listener access (see above) |
| **Controlling the phone from the computer** (`control` requested by the computer) | Injecting touch needs an AccessibilityService, also on the install-block list (it's what blocked v0.1.3/0.1.4). Answered with `decline_unsupported`. Over the cable, desktop 0.11 drives a plain `screen` share through AOA 2.0's virtual touchscreen and keyboard instead, which needs nothing from the app; the phone says "Controlled from <computer>" in the share's notification while that touchscreen is there (an external `SOURCE_TOUCHSCREEN` in `InputManager`), and Stop ends the share |
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

- **2026-11-05** — Inbox on by default (`phone_notifications` != `off`, migrated once), rows
  for the Inbox and Gallery on the first-run setup page, a one-time home card for people
  updating, the restricted-settings steps first on Android 13+. A computer's ask
  (`turn_on_request`): a sheet with Linksy open, a notification opening that sheet when
  not, at most once a minute per computer and feature; already on, the features are told
  again (backwards then back, since the core only tells a list that changed). Waiting on
  the core: `turn_on` goes in `features` (lib/main.dart) with the pin that has the message,
  so computers don't show Ask the Phone before the phone can hear it.

- **2026-11-03** — Phone on core `760cb98` (desktop next release): deck layouts made on the
  computer (`deck_layout`, announced always): kept per computer with their `version`,
  the `active` layout's face for the phone's orientation, `button`/`volume`/`music`/`clock`
  tiles and unknown kinds as plain tiles, the five themes and three of the phone's own,
  picked by holding the deck's name; `confirm` asked before running, a state's `text`
  shown; the screen kept on while the deck shows and dimmed to 20 % after a minute.

- **2026-11-02** — Phone on core `07909c2` (desktop next release): the Inbox fields
  (`messages`, `subject`, `mine`, a one-to-one chat named for the other person even when
  the newest messages are the user's), `reply_kept` for a day, `inbox_history` from what
  the listener kept; the Gallery (`gallery`: albums, items, thumbnails, `send` as the
  computer's transfer, `gallery_changed`), with `READ_MEDIA_IMAGES`/`READ_MEDIA_VIDEO`
  asked from Settings › Files; the Deck (`deck_open`/`deck_close`, `deck_run`, the layout
  on the phone); `fs` streams answered on eight threads, `..` and deleting `/` refused.

- **2026-11-01** — Phone 0.2.13: `H264Encoder` fits the size to each encoder's
  `VideoCapabilities` (same shape, width a multiple of 16) and, when `configure` refuses
  anyway, falls back to `c2.android.avc.encoder` / `OMX.google.h264.encoder`; the screen's
  virtual display and the webcam's renderer draw at the size that won (releases#6: the
  A70's `OMX.qcom.video.encoder.avc` refused a portrait 864×1920 its capabilities allowed).
- **2026-11-01** — Phone 0.2.12, more: Choose Screen on the screen viewer (`start_view(…,
  choose)` → `request_session_choosing`), a partial wake lock and a high-performance Wi-Fi
  lock held by the share service while anything is shared (the webcam froze when the phone
  locked), the video encoder, camera and cable steps on the Logs page (releases#6: the
  encoder's name, size, profile, first frame or none after 3 s, a summary at the end), and
  no Android backup or device transfer of the app's data (mobile#18, PR #19).
- **2026-11-01** — Phone 0.2.12 on core `7e310dd` (desktop 0.11.2): the cable through
  `cable::phone_bridge` (framing version 2: hello, keepalives, close, 16 KB reads, padded
  writes), the dial through it retried up to 3 times before pairing is offered (the Samsung
  A05 case), and "Controlled from <computer>" while the computer drives the phone over the
  cable. Not done: why the realme never got the accessory (needs a phone on a cable), the
  rotation for control in landscape (waits for the protocol), and control without the cable
  (an accessibility service, §3).
- **2026-10-24** — Phone 0.2.5, same core: the computer's players as an Android media
  notification (it sends `media_watch {on: true}` to each connected computer, again after
  each reconnect, unless turned off), `seek` from the page and the notification, a number
  pad on the presenter remote (keysyms `0x30`-`0x39`, Enter, Backspace), reverse
  scrolling, and Shift releases (`0xffe1`/`0xffe2` up) after shifted `text`, since KWin
  leaves Shift held. Computer volume still needs a protocol message.
- **2026-10-24** — Phone 0.2.6 on core `fb63c93` (desktop 0.9.8): the computer's volume
  (`volume_watch` alongside `media_watch`, `volume_set` with `level` and/or `muted`,
  newest `seq` kept), the number pad steps aside while the keyboard is up on the presenter
  remote, and a redesigned Transfers page that is always a tap away.
- **2026-10-24** — Phone 0.2.7, same core: home's bottom pill (Transfers, Send files, Add a
  device); no protocol change.

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
