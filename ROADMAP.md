# Linksy roadmap

Linux desktop (GTK) + Android phone. No Windows or macOS plans. Each feature lists what the
desktop (`linksy-desktop`) and the phone (`linksy-mobile`, Flutter) each have to build, so
both sides can work in parallel. Protocol details go into `docs/protocol.md` §4 as each one lands.

Effort: **S** = a day or two, **M** = a few days, **L** = a week or more.

| Release | Theme | Features |
|---|---|---|
| 0.2 | Everyday basics | Open on PC (desktop ✅ 0.2.0) · Battery & Ring · Notifications with reply |
| 0.3 | Feels like one device | Media controls · Pause on calls · Lock when you walk away |
| 0.4 | Phone as a tool | Photo auto-import · Touchpad, keyboard & presentation remote |
| 0.5 | Phone on the desktop | Texting from the desktop · Phone in the Files sidebar |

Not planned: automatic clipboard sync on GNOME (needs a Shell extension; manual Send
Clipboard stays).

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

## How each feature ships

1. The protocol messages are added to `linksy-core` with tests, and documented in
   `docs/protocol.md` §4.
2. The desktop side lands and is tested against a fake phone in the end-to-end tests.
3. The phone side lands (working from `docs/protocol.md`).
4. Both are tested together on a real phone, then released.
