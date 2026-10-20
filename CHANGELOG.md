# Changelog

Linksy for Linux. Each version's section becomes its release notes.

## 0.3.1

### Added
- **Pause on calls**: when your phone rings, what plays on this computer pauses, and it
  picks up again when the call ends (only what Linksy paused). Ringing also shows who's
  calling. Needs the Linksy for Android update that adds it.

## 0.3.0

### Added
- **Media controls, both ways**: what plays on your phone shows in GNOME's media controls
  ("Spotify on realme"), and the play, pause and skip buttons and your keyboard's media
  keys control it. Your phone can control what plays on this computer, too. Needs the
  Linksy for Android update that adds it.

## 0.2.3

### Fixed
- **Use as Second Display** on a phone stayed black: video is now sent as one piece per
  frame, which phones' decoders need.

## 0.2.2

### Added
- **Phone notifications**: messages and alerts from your phone show up as notifications
  here. **Reply** opens a small window to answer (WhatsApp, Signal, SMS…), the app's own
  button works too, and **Mute** silences an app. Dismissing one here dismisses it on
  the phone. Turn it off, or unmute apps, in Preferences. Needs the Linksy for Android
  update that adds it.

### Fixed
- The background service quits right away when asked, even with Linksy notifications
  still on screen.

## 0.2.1

### Added
- **Phone battery**: your phone's battery level shows next to it in the sidebar and on its
  page, and Linksy tells you once when it drops to 15%.
- **Ring**: make your phone ring loudly, even on silent, to find it (with a Stop button).
  When your phone looks for this computer, it shows an urgent notification with a sound.
- Both need the Linksy for Android update that adds them; `linksy ring <device>` works
  from the terminal.

## 0.2.0

### Added
- **Open on PC**: a link shared from your phone opens in this computer's browser, and
  shows in the device's activity with a button to open it again. Only web links
  (http/https) are ever opened. Needs the Linksy for Android update that adds it to the
  share menu; between computers, `linksy open <device> <url>` does it today.

## 0.1.2

### Fixed
- **Use as Webcam** works in OBS (Video Capture Device (PipeWire)) and other apps that pick
  a camera's format: the camera is offered as YUY2 at 30 fps.
- The Flatpak starts its background service by itself, and at every login.

### Changed
- The app ID is now `mg.diary.linksy.Linksy`. Installing over an older tarball install
  cleans up the old menu entry.

## 0.1.1

### Added
- **Packages**: Linksy now comes as a Flatpak, a .deb (Ubuntu, Debian) and an .rpm
  (Fedora), besides the tarball.
- **Camera preview** right in the app: click Preview on a device's camera card.

### Changed
- Screen windows fit the shape of the shared screen, phones included, with no black bars.
- When the other device can't do something yet, Linksy says so, instead of "declined".

### Fixed
- Quitting never hangs on a device that went quiet.

## 0.1.0

First release of Linksy for Linux: share between your own devices, directly over your
local network, with no account, cloud or server.

### Added
- **Devices**: computers nearby show up on their own; pair once by scanning a QR code,
  entering an address, or comparing a 6-digit code on both screens.
- **Device pages**: send files and folders (or drop them anywhere in the window), send
  your clipboard, ping, and see everything shared with each device as a timeline.
- **Files** arrive with your OK (or automatically from your devices, if you choose),
  resume if interrupted, are checked byte for byte, and open or drag out from the
  timeline. "Send with Linksy" is in the Files right-click menu.
- **View Screen**: watch another computer's screen live; **Take Control** to use its
  mouse and keyboard. The other side always says yes first.
- **Use as Second Display**: extend your desktop onto another device's screen.
- **Use as Webcam**: another device's camera becomes "Linksy Camera" in your apps.
- **Battery friendly**: devices connect only when there's something to share.
- **Lightweight**: the background service uses about 7 MB of memory when idle.
