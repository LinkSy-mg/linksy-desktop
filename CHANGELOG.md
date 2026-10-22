# Changelog

Linksy for Linux. Each version's section becomes its release notes.

## Unreleased

### Added
- **Your own relay.** Preferences › Connection › Relay Server takes the address of a
  relay you host (`linksy-server`), as `https://relay.example.com`; blank means Linksy's
  own. It wins over the relay the website lists and takes effect at once: the away
  endpoint starts again on it. Also `linksy settings relay <url>|default` and, for a
  whole machine, `LINKSY_RELAY` on the service.
- **openSUSE.** Each release now has an .rpm for openSUSE Tumbleweed
  (`linksy-X.Y.Z-1.suse.x86_64.rpm`, installed with zypper), and Finish Setup knows
  openSUSE's packages: the camera driver as the running kernel's KMP, GStreamer's GTK 4
  sink from gstreamer-plugins-rs, openh264 and libav, and PipeWire's plugin, installed
  with zypper.

### Changed
- **Linksy is open source**, under the GNU Affero General Public License v3.0 or later.

### Fixed
- **On openSUSE, Linksy asked for the administrator's password every time it opened,
  twice, and again after every setup step.** Asking firewalld anything, even whether it
  runs, needs it there (firewalld's polkit policy). Linksy now asks systemd whether
  firewalld runs, asks polkit before reading the zone and, where a password would be
  needed, doesn't: the firewall shows as to set up until Linksy has opened it itself,
  once, when asked to.
- **Opening the firewall failed with firewalld 2.x** (openSUSE Tumbleweed, Fedora 42+),
  which refuses to add a port and a service in one firewall-cmd call. The setup helper and
  the .deb and .rpm post-install now make two.

## 0.8.2

### Fixed
- **OBS's camera source listed the phone's camera as a blank entry.** The phone's camera
  is now also offered to PipeWire apps directly as "Linksy Camera (phone)", next to the
  Linksy Camera device that browsers and Zoom use.

## 0.8.1

### Fixed
- **A phone whose Linksy app doesn't take the USB cable was switched again and again.**
  When the phone drops back without answering, Linksy now stops and the Connect by USB
  Cable guide says so, with Try Again. The service's log also says how much crossed the
  cable each time, to tell where a connection stops.

## 0.8.0

### Added
- **Linksy speaks your language**: English, Deutsch, Español, Français, Italiano, 日本語,
  Malagasy, Português, Русский and 中文, the same as Linksy for Android. It follows your
  system, or choose one in Preferences.
- **Connect by USB Cable**: a guide from a phone's page ("Faster by USB Cable") or the
  main menu walks through each step live (allow the cable, plug in, switch, open Linksy
  on the phone, connected) with a Connect button, instead of a notification that was
  easy to miss.
- **Microphone preview**: while a phone's microphone is on, its card shows how loud it is
  right now, with Listen to hear it on this computer and Stop.

### Changed
- **Finish Setup works on KDE and other desktops**: it installs what's missing with your
  system's own package tool (dnf, apt or pacman) in the same single password prompt,
  turns on RPM Fusion on Fedora for the camera driver, and shows each step as it goes.
  When Secure Boot holds the camera driver, it gives the exact steps and a code to type
  at the next restart. "Prefer a Terminal?" shows the same setup as one command.
- **Downloads from linksy.diary.mg** come from the site itself and resume when a
  connection drops; APKs are named with their version.

### Fixed
- **Watching a screen failed with "gtk4paintablesink" missing** (KDE): GTK's video part
  is now required by the .rpm and .deb, checked by Finish Setup, and the viewer offers
  Finish Setup when a video part is missing.
- **Stop on the webcam card could stop the microphone instead** (and the other way
  round) when both were on.

## 0.7.2

### Fixed
- **Updating from 0.6.0 or older kept the old background service running**, so new
  features (like the terminal) didn't show and Preferences named the old version. The
  app now restarts a service that old instead of waiting for it to stop by itself.

## 0.7.1

### Changed
- **The terminal is set up from the phone's page**: a Terminal card there says what's
  next (choose a code, allow this phone, unlock after wrong codes) and does it in one
  click, instead of a switch that stayed greyed out until a code was set in Preferences.

### Fixed
- **Preferences sometimes didn't open**: when the background service was older than the
  app (right after an update), a question it didn't know went unanswered and the window
  waited forever. The service now always answers, and Preferences opens anyway.

## 0.7.0

### Added
- **Terminal from your phone**: open a shell on this computer from Linksy for Android,
  and run anything a terminal runs (Claude Code, neovim, git, builds). It's off until you
  set a terminal code in Preferences and turn on "Allow Terminal" on the phone's page.
  The phone asks for the code every time; 5 wrong codes lock it out until you unlock it
  here; this computer asks you once a day; from away only if you allow everything from
  away for that phone. A terminal keeps running when the phone leaves, so you can come
  back to it (for a day), and ends for good when closed on the phone or with End on the
  phone's page here. Needs the Linksy for Android update. `linksy terminal` does the
  same from the command line.
- **Set Up Everything**: Finish Setup now also checks the firewall (and opens UDP 47200
  and mDNS so your devices can find this computer), that Linksy starts when you log in,
  and that your desktop can share its screen, and fixes everything missing in one go,
  asking for your password once. It opens by itself the first time Linksy starts with
  something missing. The .deb and .rpm open the firewall when they're installed.

## 0.6.4

### Changed
- **Received files go to Downloads/Linksy** by default (like the phone), instead of
  straight into Downloads. A folder you chose in Preferences stays as it is.
- **The device page fits any window**: it lays itself out by the space it has beside
  the device list, stacking the actions, the "Use as" cards and the header as it gets
  narrower, instead of running off the edge.
- The Linksy logo sits at the left of the sidebar.

## 0.6.3

### Added
- **Black top bar when watching an Android phone?** The screen viewer's info button
  explains it: some phones hide their status bar from screen sharing, and how to turn
  that off on the phone.

## 0.6.2

### Changed
- **Redesigned device page**: the device at the top with its battery as a ring around
  its picture and how it's connected; a big Send Files area (click, or drop files
  anywhere); one-click tiles for Clipboard, View Screen, Ring and Locate; "Use as"
  cards for Second Display, Webcam and Microphone, with the webcam showing when it's
  live; and the activity as a timeline you can filter by files, clipboard or links.
- **Hide the device list**: the button at the top left (or F9) shrinks the sidebar to
  a strip of device pictures, for a bigger page.

## 0.6.1

### Fixed
- **Updates take effect right away**: the background service used to keep running the
  old version after an update until you logged out, so new features (like away from
  home) didn't work yet. It now restarts into the new version by itself within a
  minute, and opening Linksy restarts a service left on an older version.
- **"Allow Everything From Away"** only shows for a device once the two can actually
  reach each other away from home: after they've met on the same Wi-Fi with both
  running Linksy 0.6.

## 0.6.0

### Added
- **Away from home**: your paired devices reach each other over the internet, not only
  on the same Wi-Fi: send files, the clipboard and links, ring your phone or see its
  battery from anywhere. Linksy finds a direct path when it can, and otherwise goes
  through its relay, which can't read anything: everything stays encrypted end to end.
  For safety it lasts a week after the devices were last on the same Wi-Fi (being
  together renews it), and screens, control, browsing files, the camera and the
  microphone stay on the same network unless you allow them for a device. Lock When
  Away still only counts a phone that's nearby. Turn it off in Preferences. Needs the
  Linksy for Android update to work with phones.
- **Locate**: ask your phone where it is, once, and open it on a map. The phone can say
  no, and nothing is kept.
- **1080p webcam**: with a phone that can, turn on 1080p in the camera's settings.

### Changed
- **Linksy is freeware**: free to use and to share unmodified, not open source. The
  license is at linksy.diary.mg/license, and the open-source parts it builds in are
  credited in THIRD-PARTY-NOTICES.

## 0.5.9

### Added
- **60 fps webcam**: with a phone that films at 60 frames per second, turn on
  "60 Frames per Second" in the camera's settings (or on the phone): the Linksy Camera
  follows, for smoother motion in calls and OBS. It restarts in your apps for a moment.

## 0.5.8

### Added
- **Update check**: once a day Linksy looks at linksy.diary.mg for a newer version and
  says so in a notification and at the bottom of its sidebar. Nothing about you is sent;
  turn it off in Preferences.
- **Reconnect a second display**: when a phone or tablet used as a second display drops
  (its screen turned off, the Wi-Fi blinked), Linksy says so and offers Reconnect.

### Fixed
- A phone controlling a second display or this screen could do nothing when "Allow
  remote interaction" was off in the sharing dialog, or after a remembered choice without
  it: Linksy now asks again next time instead of reusing it.

## 0.5.7

### Added
- **Phone as a microphone**: "Use as Microphone" on a phone's page makes it
  "Linksy Microphone" in every app (calls, OBS, recorders). Needs the Linksy for Android
  update that shares its microphone.
- **USB cable**: plug a phone in and Linksy can use the cable instead of Wi-Fi, with
  lower, steadier latency for the webcam and microphone, and no network needed. No
  developer options on the phone. Finish Setup adds the permission it needs if your
  package didn't. Needs the matching Linksy for Android update.
- **The phone's camera controls on the computer**: under the camera preview, switch
  between front and back, zoom, turn on the torch, click to focus, and set exposure,
  white balance, quality and orientation.
- **Device pictures**: a phone's picture shows as its avatar, and the phone sees your
  account picture.

### Changed
- **The webcam and screens show up about 0.4 s sooner**: Linksy's video decoder no longer
  holds frames back before showing them.

## 0.5.6

### Added
- **Images and files on the clipboard**: Send Clipboard sends a copied screenshot or file
  (up to 10 MB), and it lands on the other device's clipboard, ready to paste. Bigger or
  several copied files go as a normal transfer.
- **Automatic clipboard sync** for phones that ask for it: text copied on the computer
  reaches the phone without pressing anything. Works on KDE Plasma and wlroots desktops
  (Sway, Hyprland); GNOME doesn't let apps watch the clipboard.

## 0.5.5

### Changed
- **Second display in the phone's shape**: the extra monitor takes the phone's (or
  tablet's) proportions instead of 16:9, so it fills the screen with no black bars. Needs
  the Linksy for Android update that sends its size.

## 0.5.4

### Changed
- Linksy only offers what each device can do. For Android phones that means no Messages,
  no Take Control and no phone-notification settings, which Android doesn't allow for apps
  installed outside the Play Store.
- A phone's shared folder is named in Files ("Pixel 9 (Documents)").

## 0.5.3

### Added
- **Finish Setup**: when something a feature needs is missing (the camera driver for
  browsers and Zoom, video codecs, screen sharing, clipboard tools), Linksy says so at the
  bottom of its sidebar and installs it in one click; your computer asks for your password.
- **The phone's camera in every app**: with v4l2loopback, it becomes "Linksy Camera" in
  Google Meet, Zoom, Discord and any browser, with no settings to change.

## 0.5.2

### Changed
- Flatpak: no access to the Files bookmarks (a Flathub rule); a phone shows in the
  Files sidebar while it's open there instead of as a permanent bookmark.

## 0.5.1

### Added
- **Phone in the Files sidebar**: Browse Files on your phone's page opens it in Files like
  a USB drive, and it stays in the sidebar under its name: open, copy, rename and delete
  its files, and it shows up in every app's file dialog. Needs the Linksy for Android
  update that adds it, with file sharing turned on there.

## 0.5.0

### Added
- **Texting from the desktop**: open Messages on your phone's page to read your
  conversations, answer, or write a new text. New texts show up as they arrive. Nothing
  is kept on the computer. Needs the Linksy for Android update that adds it.

## 0.4.1

### Added
- **Phone as touchpad, keyboard and presentation remote**: your phone can move this
  computer's pointer, click, scroll and type, and turn slides with its volume keys,
  after you allow it here. It doesn't see your screen. Needs the Linksy for Android
  update that adds it.

## 0.4.0

### Added
- **Photo auto-import**: new photos from your phone land in Pictures, in a folder named
  after the phone, without asking, with one notification per batch. Turn it off in
  Preferences. Needs the Linksy for Android update that adds it.

## 0.3.2

### Added
- **Lock When Away**: turn it on on your phone's page, and this computer locks itself
  about a minute after your phone leaves. It locks once per departure and never unlocks
  by itself. Needs the Linksy for Android update that adds it.

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
