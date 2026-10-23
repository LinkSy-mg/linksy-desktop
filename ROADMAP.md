# Linksy roadmap

Linux desktop (GTK, and Qt for KDE Plasma) + Android phone. No Windows or macOS plans. Each feature lists what the
desktop (`linksy-desktop`) and the phone (`linksy-mobile`, Flutter) each have to build, so
both sides can work in parallel. Protocol details go into `docs/protocol.md` §4 as each one lands.

Effort: **S** = a day or two, **M** = a few days, **L** = a week or more.

| Release | Theme | Features |
|---|---|---|
| 0.2 | Everyday basics | Open on PC (desktop done in 0.2.0) · Battery & Ring (desktop done in 0.2.1) · Notifications with reply (desktop done in 0.2.2) |
| 0.3 | Feels like one device | Media controls (desktop done in 0.3.0) · Pause on calls (desktop done in 0.3.1) · Lock when you walk away (desktop done in 0.3.2) |
| 0.4 | Phone as a tool | Photo auto-import (desktop done in 0.4.0) · Touchpad, keyboard & presentation remote (desktop done in 0.4.1) |
| 0.5 | Phone on the desktop | Texting from the desktop (desktop done in 0.5.0) · Phone in the Files sidebar (desktop done in 0.5.1) · USB cable, microphone, camera controls (desktop done in 0.5.7) |
| 0.6 | Anywhere | Away from home over the internet, with a 7-day lease · Locate the phone · 1080p webcam (desktop done in 0.6.0) |
| 0.7 | Your computer in your pocket | Terminal from the phone, locked by a code set on the computer (desktop done in 0.7.0; `docs/protocol.md` §4.15) · Set Up Everything (0.7.0) |
| 0.8 | Easier to set up | One-password Finish Setup · Connect by USB Cable guide · Microphone level and Listen · Ten languages (desktop done in 0.8.0) |
| **0.9** | **Works on every desktop** | Hyprland, Sway, niri, X11 desktops and distributions without systemd ([desktop support](docs/desktop-support.md)) · Older releases (Ubuntu 24.04) and a list of every dependency · The cable when you ask for it · Upgrades in one click (`linksy upgrade`, Update Now) · Problem logs (`logs.txt`) · Lighter at rest |
| 0.10 | At home on KDE | A Qt 6 app for KDE Plasma, beside the GTK one (desktop done early, in 0.8.4) |
| **0.11** | **Never miss a text** | Texts as notifications on the computer, with reply |
| **0.12** | **Fast on any link** | Lower latency on every connection · 60 fps screens · Transfers with no shared network · Bluetooth: connect, and know when the phone is near |
| **0.13** | **The cable, for real** | USB that works on every phone, end to end |
| **0.14** | **Your relay, or none** | Away from home becomes optional: off, removed, or your own relay (your own relay done in desktop 0.8.4 and Android 0.2.3) |
| **0.15** | **Calls** *(maybe)* | Answer, decline and hang up from the computer; talk through it over Bluetooth |
| **0.16** | **Optional extras** | Control the phone's screen from the computer · Automatic clipboard sync |
| **1.0** | **Feature freeze** | No new features: fixes, performance, polish, stores · Onboarding |

After 0.16, Linksy is feature complete: everything after that is fixing. Bold releases are
next; the rest have shipped.

Not planned: automatic clipboard sync on GNOME (GNOME doesn't let apps watch the clipboard;
it needs a Shell extension, and manual Send Clipboard stays). Windows and macOS.
Unlocking the computer from the phone.

---

## 0.9 — Works on every desktop

### Hyprland, Sway, niri, X11 and no systemd — L
Everything in [docs/desktop-support.md](docs/desktop-support.md): where a desktop lacks
something Linksy relies on, Linksy brings its own fallback instead of asking for a package.
- **First:** Linksy's own password dialog when no polkit agent runs, the installed screen
  locker for Lock When Away, the right portal on niri, clipboard watching on X11.
- **Then:** autostart without systemd, Linksy's own notifications when no notification
  daemon runs.
- **Then:** screen control and Sidecar on Hyprland and Sway (virtual input, headless
  outputs), and the phone's files in a window inside Linksy where no gvfs file manager is.
- **Finish Setup** checks for each of these and says exactly what's missing; features a
  desktop can't do are hidden.
- **Done when:** on Hyprland and Sway every feature works or says why it can't, tested in a
  nested compositor and on a real install.

### Sending from the phone — done in Android 0.2.3
- **Share sheet:** each paired computer shows up in Android's share sheet, like contacts;
  picking one sends straight to it ([mobile#10](https://github.com/LinkSy-mg/linksy-mobile/issues/10)).
- **Photo picker:** Send files offers Photos and videos, many at once, sent as one transfer
  so the computer asks once ([mobile#9](https://github.com/LinkSy-mg/linksy-mobile/issues/9)).
- **Transfers bubble:** what's being sent or received, with progress and Cancel, without
  scrolling ([mobile#4](https://github.com/LinkSy-mg/linksy-mobile/issues/4)).
- **Webcam upright** when the phone stands ([mobile#6](https://github.com/LinkSy-mg/linksy-mobile/issues/6)).
- **Desktop:** one Accept per batch and a transfers panel are still to come (desktop#32,
  desktop#33).

### Every version, not only the newest — M
*Includes [@stadsport](https://github.com/LinkSy-mg/linksy-releases/issues/4)'s CachyOS setup report (fixed in 0.8.4).*
Linksy should run on as many desktops and distribution versions as it reasonably can.
- **Ubuntu 24.04 LTS** and what's built on it (Linux Mint 22, Pop!_OS, Zorin): the app
  needs libadwaita 1.5 instead of 1.6, so the .deb installs there.
- **A test matrix** in CI: the packages install and start on every supported release of
  Ubuntu, Debian, Fedora, openSUSE, Arch and Linux Mint.
- **Plasma 5** (Kubuntu 24.04, Debian 12): the Flatpak, which brings its own toolkit.
- **Every dependency listed** in one page, feature by feature with each distribution's
  package names, for installing by hand when Finish Setup can't.
- **Done when:** the .deb installs and runs on Ubuntu 24.04, and CI proves every listed
  release.

### The cable when you ask for it — S
Plugging a phone in to charge no longer pops a question. A "Connect by USB Cable" button
on the device's page, in the desktop app and the phone app, switches the cable to Linksy.

### Upgrades in one click — M
Installing is one command (`curl -fsSL https://linksy.diary.mg/install.sh | sh`, done);
upgrading should be as easy, from the terminal and from the apps.
- **`linksy --version`:** Linksy's version, and the running service's when it differs
  ("service 0.8.4: restart it").
- **`linksy --revision`:** the exact build, for bug reports: version, commit, build date and
  how it was installed (rpm, deb, Flatpak, tarball).
- **`linksy upgrade`:** when a newer version is out, asks once, then downloads exactly the
  packages installed (`linksy-common` and `linksy` and/or `linksy-qt`, in this system's
  format), **checks them against the release's checksums**, installs them with one
  password, and restarts the service and the app.
- **Update Now in both apps:** the "Linksy X Is Available" button runs the same steps with
  a progress line, then Linksy comes back by itself and What's New shows what changed.
- **Each way of installing:** packages as above; the tarball updates into `~/.local` with
  no password; the Flatpak through `flatpak update` once it's on Flathub; anything else
  (the AUR, built by hand) says how to update it that way.
- **Later,** with Linksy's own package repository, upgrading becomes the system's own update
  and this only drives it.
- **Done when:** a new release reaches someone with one command or one click, restarts
  cleanly, and never installs a file whose checksum doesn't match.

### The Flatpak, nearly everything — S
The Flatpak's sandbox can't change the system, so the webcam in browsers and calls (the
`v4l2loopback` driver) and the USB cable (a udev rule) don't set themselves up there. Both
work once that one step is done outside it.
- **Finish Setup in the Flatpak** shows the exact command for this system to copy (the
  camera driver, the USB rule), instead of "a Flatpak can't add this", then checks it took.
- **The wording** on the website, in the installer and in the release README says "need a
  one-time setup step" instead of "no webcam in browsers and calls, no USB cable".
- **Done when:** after one pasted command, the Flatpak has the webcam in every app and the
  cable, like the packages.

### Problem logs — S
When something goes wrong, one file says what happened, ready to attach to a bug report.
- **Desktop:** a crash in the app, the service or the screen helper writes what it knows
  (version, what was running, the error) to `~/.local/state/linksy/crashes/`. **Save Logs…**
  in About gathers the recent log of the service and the app, the crash files, and the
  versions of Linksy, the desktop, the kernel and GStreamer into one `logs.txt`. After a
  crash, Linksy offers it the next time it opens.
- **Phone:** the same **Save Logs** in the app's About, as a `logs.txt` to share.
- **Privacy:** nothing is sent anywhere; the person decides whether to share the file.
  File names, texts, notification contents and pairing codes are left out.
- **Done when:** a crash on either side can be turned into a `logs.txt` in two clicks, and
  that file is enough to understand it.

### Lighter at rest — M
Linksy runs all day, so what it holds while idle matters more than its speed. Today: the
service 25 MB, the app 49 MB, the phone app about 200 MB.
- **Service:** a smaller binary (12 MB of the 25 is the 20 MB program itself): trim unused
  dependency features, and weigh `opt-level = "s"` for code that isn't hot. Fewer malloc
  arenas (`MALLOC_ARENA_MAX=2` in the unit) and memory handed back after a transfer or a
  session ends.
- **App:** pages built when first opened, not all at start; pictures decoded at the size
  they're shown; the window's memory released when it closes. Check the GL renderer
  against Vulkan, whose driver alone takes 3 MB.
- **Phone:** when the app is closed, only the service and the core stay; the screens and
  their engine are released. Image cache capped, thumbnails at their shown size, and caches
  dropped when Android asks. *First step in Android 0.2.3: the image cache capped at 24 MB
  and emptied when hidden, pictures decoded at their shown size; about 95 MB PSS idle on an
  emulator (`docs/memory.md` in linksy-mobile). Still to measure on a real phone.*
- **Done when:** idle, the service stays under 15 MB, the app under 35 MB and the phone app
  under 100 MB in the background, measured the same way each release.

## 0.10 — At home on KDE

### A Qt 6 app for KDE Plasma — L, done early in 0.8.4
*Shipped as `linksy-qt`, with every page the GTK app has (Finish Setup, the camera preview
and controls, being a second display, the USB guide). The packages split into
`linksy-common` with `linksy` or `linksy-qt`; the one Linksy in the menu opens the Qt app on
Plasma. The Flatpak and the tarball stay GTK.*
Same service, second front end: the app is only a window on `linksyd`, so a Qt one talks to
it over the same local socket (`linksy-ipc`) and nothing below it changes.
- **Desktop:** a Qt 6 / Kirigami app with the GTK app's pages (devices, device page,
  pairing, transfers, preferences, Finish Setup, USB guide), in Breeze, with KDE's own
  notifications and file dialogs. Packaged as its own package (`linksy-qt`); installing
  either app is enough, and the GTK one stays the default elsewhere.
- **Translations:** the same catalogs as the GTK app (`linksy-i18n`), so every language
  carries over.
- **Phone:** nothing.
- **Done when:** on Plasma, Linksy looks and behaves like a KDE app, every feature reachable.

## 0.11 — Never miss a text

### Texts as notifications, with reply — M
*Asked for by [@yurividal](https://github.com/LinkSy-mg/linksy-desktop/issues/30).*
Builds on 0.5 (the Messages page and `sms_received`).
- **Protocol:** `sms_received` carries what the notification needs (sender, contact name and
  picture, text, thread); reply goes back with `sms_send`.
- **Desktop:** each new text as a notification with Reply and Mark as Read; the Messages page
  opens on its thread when clicked. Not shown twice when the phone's own notification for
  the messaging app is mirrored too.
- **Phone:** push every incoming SMS as it arrives (`SMS_RECEIVED`), with the contact's name.
- **Done when:** a text arrives on the computer within a second, and replying from the
  notification sends it from the phone.

## 0.12 — Fast on any link

### Lower latency on every connection — L
Linksy connects five ways: the local network, directly over the internet, through the
relay, over Bluetooth and over the cable. Each gets measured and tuned, so screens, the
touchpad and the webcam feel immediate.
- **Measure:** round-trip time, frame delay (captured to shown) and input delay for each
  way, written down so each release can compare.
- **Video:** frames go as QUIC datagrams instead of a stream, so a lost packet drops one
  frame instead of holding back the next ones. The bitrate follows the link (today a fixed
  8 Mbit/s); low-latency decoding on the phone (`KEY_LOW_LATENCY`); late frames skipped.
- **Input:** pointer moves as datagrams too, merged when they pile up.
- **Paths:** a session moves to a better path while it runs: relay to direct to the local
  network as soon as one works. Reconnecting after a network change or sleep happens at
  once (network and mDNS events), not on the next 15-second retry. BBR congestion control
  over the internet.
- **Seen:** the device page shows how it's connected and the live delay, so a good link
  shows.
- **Done when:** on the local network, the phone's touch reaches the computer's screen and
  back within 50 ms, and switching networks reconnects in under 2 seconds.

### 60 fps screens — M
Today screens run at up to 30 fps (the webcam already does 60).
- **Both ways:** the computer's screen on the phone, controlling it, Sidecar, and the
  phone's screen on the computer, at 60 fps where both ends can encode and decode it.
- **Phone:** 60 fps decoding and display, and 60 fps capture of its own screen; asks the
  display for its highest refresh rate while showing a screen.
- **Falls back:** to 30 fps on a slow link or device, on its own.
- **Done when:** scrolling a page on the computer is smooth on the phone, at 60 fps, on
  the local network.

### Transfers with no shared network — M
No Wi-Fi network around, or one that blocks devices from seeing each other (hotels,
offices): the phone and the computer make their own link.
- **How:** Wi-Fi Direct where both support it, so the computer keeps its own Wi-Fi; else the
  phone's local-only hotspot (`startLocalOnlyHotspot`, no mobile data used), which the
  computer joins through NetworkManager. The link is set up over Bluetooth, or from a QR
  code, and closes when the transfer ends.
- **Speed:** full Wi-Fi speed, so files, the webcam and screens work too, unlike over
  Bluetooth.
- **Done when:** with no network around, a 1 GB video goes from the phone to the computer
  at Wi-Fi speed, and the computer is back on its usual Wi-Fi afterwards.

### Bluetooth — L
- **Transport:** Linksy's connection over Bluetooth (L2CAP channels; RFCOMM where they're
  missing) when there's no shared network. Same pairing, same encryption: only the carrier
  changes. Bluetooth is slow, so it carries the light features (notifications, texts,
  clipboard, battery, media controls, Ring, Locate); camera, screens and big files say
  they need Wi-Fi or the cable.
- **Presence:** Bluetooth LE tells whether the phone is near, for Lock When Away, without a
  connection.
- **Desktop:** BlueZ through D-Bus in `linksyd`; Bluetooth shown on the device page like
  "Connected by USB".
- **Phone:** a Bluetooth server socket and LE advertising, with the Nearby Devices permission.
- **Done when:** with Wi-Fi off on both, notifications and texts still arrive, and walking
  away still locks the computer.

## 0.13 — The cable, for real

### USB that works on every phone — M
The 0.5.7 cable (Android Open Accessory) switches the phone, but some phones never answer
on it.
- **Desktop and phone:** find out why the accessory link stays silent on those phones and
  fix it; test on several brands (realme, Samsung, Pixel, Xiaomi).
- **Fallback:** where accessory mode can't work, connect over the phone's USB networking
  (USB tethering) instead, so the cable still gives a fast link.
- **Done when:** plugging in and pressing Connect gives "Connected by USB" on every phone
  tested, every time.

## 0.14 — Your relay, or none

### Away from home becomes optional — S
*Your own relay is built by [@yurividal](https://github.com/yurividal) and shipped in desktop 0.8.4 and Android 0.2.3 ([desktop#31](https://github.com/LinkSy-mg/linksy-desktop/pull/31), [mobile#7](https://github.com/LinkSy-mg/linksy-mobile/pull/7)); the hosting guide is [server#2](https://github.com/LinkSy-mg/linksy-server/pull/2). Off and Removed remain.*
- **Off:** a setting that turns Away from home off completely: no contact with the relay at
  all, not even to register.
- **Removed:** a build option that leaves the relay code out, for distributions and people
  who want none of it.
- **Your own relay (done):** the relay's address is a setting on both sides, and the relay
  (`linksy-server`) is published with instructions to host one.
- **Done when:** with Away off, Linksy makes no connection beyond the local network; with a
  self-hosted relay, Away from home works through it.

## 0.15 — Calls *(maybe)*

### Calls on the computer — L
- **Control:** see who's calling, answer, decline and hang up from the computer (the phone
  answers with `ANSWER_PHONE_CALLS`). This part needs no Bluetooth.
- **Audio:** talking through the computer's microphone and speakers needs the computer to
  act as a Bluetooth hands-free device (HFP) for the phone, through PipeWire. It depends on
  0.11, and on how well each computer's Bluetooth does HFP.
- **Done when:** a call can be answered on the computer and held through its headset.
- **Maybe** because the audio part depends on hardware we don't control. If it isn't
  reliable, only call control ships.

## 0.16 — Optional extras

### Control the phone's screen from the computer — M
The phone's screen already shows on the computer. This adds the mouse and keyboard driving
it, like scrcpy.
- **Phone:** taps, swipes and typing replayed through an accessibility service (a
  sideloaded app can't inject input otherwise); asked for once, explained clearly.
- **Desktop:** clicks, scrolls and keys on the phone's screen window sent as input.
- **Done when:** a phone app can be used from the computer without touching the phone.

### Automatic clipboard sync — M
Today, sending the clipboard is a button.
- **Computer to phone:** automatic wherever the desktop allows watching the clipboard
  (KDE, Hyprland, Sway, X11; not GNOME).
- **Phone to computer:** Android only lets the app in front, or the keyboard, read the
  clipboard. Automatic while Linksy is open, and through a "Copy to Computer" entry in
  Android's text menu otherwise.
- **Done when:** text copied on one side is ready to paste on the other, with no button.

## 1.0 — Feature freeze

No new features after 0.16. Releases are fixes only:
- reliability of every feature on more phones and computers;
- performance and battery use on the phone;
- Flathub, AUR and other stores.

The one addition is a first run that finishes the install:

### Onboarding — M
At most five slides, skippable, shown once: welcome; get the phone app (a QR code to the
download); pair; finish setup (the dependency check, each missing piece with its fix
button); done, with where the main features are.
- **Done when:** a fresh install goes from first launch to a paired phone, with every
  feature the computer can do working, without opening a terminal.

---

## 0.2 — Everyday basics

### Open on PC — S
Share a link from any phone app → it opens in the desktop's browser. Share text → it lands
on the desktop clipboard.
- **Protocol:** `open_url { url }` on the control stream; text reuses `clipboard`.
- **Desktop:** open with the default browser (`xdg-open` / OpenURI portal in the Flatpak);
  only `http(s)` URLs, from paired devices. Timeline entry "Opened link".
- **Phone:** "Open on PC" in the Android share sheet (`ACTION_SEND` text/plain), with a
  device picker when several PCs are paired.
- **Done when:** sharing a YouTube link on the phone opens it on the PC in under a second.

### Battery & Ring my phone — S
- **Protocol:** `battery { level, charging }` sent on change (≥5% steps, plug/unplug);
  `ring {}` and `ring_stop {}`.
- **Desktop:** battery on the phone's device page and sidebar row; one notification at
  15% ("realme is low on battery"); a **Ring** tile.
- **Phone:** `BatteryManager` broadcasts → `battery`; `ring` plays the alarm stream at full
  volume (even on silent) until stopped or 30 s, with a full-screen "Found it" button.
- **Done when:** the battery shows within a few seconds of plugging in, and Ring works with
  the phone on silent.

### Notifications with quick reply — M
Phone notifications appear as GNOME notifications; replying from the desktop answers in
the app (WhatsApp, Signal, SMS…).
- **Protocol:** `notification_posted { key, app, app_name, title, body, icon?, can_reply,
  actions[] }`, `notification_removed { key }`, `notification_reply { key, text }`,
  `notification_action { key, action }`, `notification_dismiss { key }`.
- **Desktop:** show with the phone app's name and icon; **Reply** opens a small reply box
  (GNOME has no inline reply, so it's a Linksy dialog); dismissing on one side dismisses
  on the other. Settings: on/off and a per-app mute list. Not stored in history.
- **Phone:** `NotificationListenerService` (special access, guided by onboarding); reply
  via the notification's `RemoteInput`; skip ongoing/silent/Linksy's own notifications.
- **Done when:** a WhatsApp message shows on the PC within 2 s and a reply from the PC
  is delivered.

---

## 0.3 — Feels like one device

### Media controls, both ways — M
- **Protocol:** `media_state { player, title, artist, playing, position?, length?, art? }`
  and `media_command { player, cmd: play|pause|next|previous|seek|volume, value? }`.
- **Desktop:** read and drive players over MPRIS (D-Bus `org.mpris.MediaPlayer2.*`);
  publish the phone's player as an MPRIS player too, so it shows in GNOME's media controls
  and the keyboard's media keys work on it.
- **Phone:** a media-style notification controlling the PC; `MediaSessionManager`
  (needs the notification access above) to expose phone playback.
- **Done when:** the PC's Spotify shows on the phone with working controls, and the
  phone's podcast shows in GNOME's top bar.

### Pause on calls — S (after media controls)
- **Protocol:** `call_state { state: ringing|active|idle, contact? }`.
- **Desktop:** ringing → notification "Incoming call from X"; active → pause playing MPRIS
  players (remember which); idle → resume those only.
- **Phone:** `TelephonyCallback` (READ_PHONE_STATE); contact name only if the user allows
  contacts access.
- **Done when:** a call pauses a playing video on the PC and hanging up resumes it.

### Lock when you walk away — M
The PC locks when the phone leaves; it never unlocks by proximity.
- **Detection:** today's presence (mDNS re-checked every 45 s) is too slow and too
  noisy: phones drop Wi-Fi to save battery. Phase 1: a light `heartbeat {}` every 20 s
  from the phone while it's on the same network, and "away" after 3 missed beats. Phase 2:
  Bluetooth LE signal strength for "walked away" vs "still in the room".
- **Desktop:** opt-in per device, with a grace period (default 1 min); lock with
  `loginctl lock-session` (works on GNOME, KDE and others), only when this session is
  unlocked and active; a "Locked because realme left" line in the timeline.
- **Phone:** send the heartbeat only for devices with the feature on: over the open
  connection when there is one, otherwise a tiny alarm-scheduled ping (WorkManager can't
  run every 20 s). The battery cost must stay negligible; measure it.
- **Done when:** walking out of the room with the phone locks the PC within the grace
  period, and a phone on the desk with its screen off never triggers a lock in a
  2-hour test.

---

## 0.4 — Phone as a tool

### Photo auto-import — M
- **Protocol:** none new: files transfer with an `import` flag so the desktop files them
  under `~/Pictures/<phone name>/YYYY-MM/` without asking.
- **Desktop:** setting per device (on/off, folder); one summary notification per batch
  ("12 photos imported"), not one per photo.
- **Phone:** `MediaStore` observer for new photos/videos; queue and send when the PC is
  nearby and the phone is on Wi-Fi (optionally only while charging); remember what was
  sent so nothing is sent twice.
- **Done when:** photos taken while away are on the PC within a minute of getting home.

### Touchpad, keyboard & presentation remote — M
- **Protocol:** reuses `ControlMsg` (§4.8) in the other direction: a `control` session
  requested by the phone.
- **Desktop:** apply input through the RemoteDesktop portal, as "Take Control" already
  does from the other side; a "Remote" banner with Stop while it's in use.
- **Phone:** touchpad screen (move, tap, two-finger scroll, right-click), a keyboard
  field, and a presentation mode (next/previous with the volume keys, laser-pointer
  dot).
- **Done when:** a slideshow can be run from the phone, volume keys turning pages.

---

## 0.5 — Phone on the desktop

### Texting from the desktop — L
- **Protocol:** `sms_threads`, `sms_messages { thread }`, `sms_send { to, text }`, and
  `sms_received` pushes.
- **Desktop:** a Messages page on the phone's device page: conversations, history,
  compose; new texts as notifications with reply (via the 0.2 notification work).
- **Phone:** SMS read/send permissions (fine for an APK we ship ourselves, not Play
  Store–friendly); MMS later.
- **Done when:** a conversation can be read and continued from the PC.

### Phone in the Files sidebar — L
- **Approach:** the phone serves its shared storage over the existing connection; the
  desktop mounts it with FUSE at `$XDG_RUNTIME_DIR/linksy/<phone>` and adds a bookmark, so
  it shows in Nautilus and every file dialog. (Alternative: SFTP plus GVfs, as KDE Connect
  does.)
- **Desktop:** the FUSE filesystem in `linksyd` (read and write, streamed, no full copies),
  mounted on demand and unmounted when the phone leaves. The Flatpak needs FUSE access.
- **Phone:** a file-serving stream with storage access (`MANAGE_EXTERNAL_STORAGE`, or
  the storage access framework for a narrower choice).
- **Done when:** a phone folder can be opened in Nautilus and a video plays from it
  without copying.

---

## Thanks

People who made Linksy better by reporting, testing or building:

- [@yurividal](https://github.com/yurividal): openSUSE packaging and the firewalld 2.x fix
  (0.8.3), and your own relay (desktop 0.8.4, Android 0.2.3).
- [@giuliocatullo23-create](https://github.com/LinkSy-mg/linksy-releases/issues/2): screen
  sharing on Intel Kaby Lake (fixed in 0.8.3).
- [@Bathoryn](https://github.com/LinkSy-mg/linksy-releases/issues/3): the firewall on
  Fedora 44, with a patch (fixed in 0.8.3).
- [@stadsport](https://github.com/LinkSy-mg/linksy-releases/issues/4): setup on CachyOS
  (fixed in 0.8.4).
- [@judempoyo](https://github.com/LinkSy-mg/linksy-mobile/issues/8): screens dropping on
  Intel graphics.
- [@Natcuber](https://github.com/LinkSy-mg/linksy-mobile/issues/1): a number pad for the
  presentation remote.
- [@consciousY](https://github.com/LinkSy-mg/linksy-desktop/issues/42): Second Display on
  Hyprland (0.9).
- [@Mustafa-Hamade](https://github.com/LinkSy-mg/linksy-mobile/issues/12): devices
  announcing permanent identifiers on every network (private discovery, 0.9).
- [@bramikoko](https://github.com/LinkSy-mg/linksy-mobile/issues/13): Ring with the
  Flatpak.

## How each feature ships

1. The protocol messages are added to `linksy-core` with tests, and documented in
   `docs/protocol.md` §4.
2. The desktop side lands and is tested against a fake phone in the end-to-end tests.
3. The phone side lands (working from `docs/protocol.md`).
4. Both are tested together on a real phone, then released.
