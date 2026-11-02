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
| 0.9 | Works on every desktop | Hyprland, Sway, niri, X11 desktops and distributions without systemd ([desktop support](docs/desktop-support.md)) · Older releases (Ubuntu 24.04) and a list of every dependency · The cable when you ask for it · Upgrades in one click (`linksy upgrade`, Update Now) · Problem logs (`logs.txt`) · Private discovery · One Accept for a batch (desktop done in 0.9.0; screen control on Hyprland and Sway moves to 0.10) |
| **0.10** | **Lighter and smoother** | Lighter at rest (desktop done in 0.11.3) · Transfers in a corner panel (desktop done) · Choose which monitor to view (desktop done) · Screens on X11 without a portal (desktop done) · Screen control on Hyprland and Sway (desktop done) · A Qt 6 app for KDE Plasma (desktop done early, in 0.8.4) |
| **0.11** | **Never miss a notification** | The phone's notifications on the computer, with reply, which brings texts from every messaging app · Snap to computer (desktop 0.11 shipped Linksy Cable and phone control by cable early, from 0.13 and 0.16) |
| **0.12** | **Fast on any link** | Lower latency on every connection · 60 fps screens · Wi-Fi Direct at full speed on any router · Transfers with no shared network · Bluetooth: connect, and know when the phone is near · Connection Doctor |
| **0.13** | **The cable, for real** | USB that works on every phone, end to end (desktop done in 0.11.0; the phone next) · The phone's photos on the computer |
| **0.14** | **Your relay, or none** | Away from home becomes optional: off, removed, or your own relay (desktop done in 0.11.3; your own relay in Android 0.2.3) |
| **0.15** | **Calls** *(maybe)* | Answer, decline and hang up from the computer; talk through it over Bluetooth · The phone's controls on the computer |
| **0.16** | **Optional extras** | Control the phone's screen from the computer · Automatic clipboard sync · The phone's sound with its screen · Deck · Videos open where you left off · The computer's notifications on the phone |
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
- **First:** Linksy's own password dialog when no polkit agent runs (desktop done in
  0.10.1), the installed screen locker for Lock When Away, the right portal on niri,
  clipboard watching on X11.
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
*Desktop 0.9: the .deb installs on Ubuntu 24.04 and Linux Mint 22 (libadwaita 1.5, video
without GStreamer's GTK 4 sink there), tested in CI on seven releases.*
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

### The Flatpak, everything — M, desktop done in 0.10.0
*Most people pick the Flatpak, so it does everything the packages do. It has host access
(`org.freedesktop.Flatpak`, the `linksy-host` crate): Finish Setup checks the computer, not
the sandbox, and sets up the webcam driver, the USB cable and the firewall with the
computer's own password prompt; Ready When You Log In writes an autostart entry; the
terminal is the computer's shell; Update Now installs the new .flatpak; Send with Linksy
appears in Dolphin, GNOME Files and Thunar; lockers, volume, Hyprland and Sway outputs, iw
and X11's clipboard tools are the computer's. Tested with a stand-in flatpak-spawn; the
real bundle is built and installed by CI.*
- **Still to do:** Flathub stays on hold (its rules for AI-written manifests). The website
  and the webcam blog post say what the Flatpak does since 0.10.0.

### Problem logs — S, desktop done
*Desktop: crash files in `~/.local/state/linksy/crashes/`, `logs.txt` in About (GTK:
Troubleshooting, with Save and Copy; Qt: Save Logs… in the menu), offered once after a
crash, and `linksy logs > logs.txt`.*
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

### Private discovery — M, desktop done
On a café's or an office's Wi-Fi, Linksy no longer tells everyone the device's name and a
permanent id ([#38](https://github.com/LinkSy-mg/linksy-desktop/issues/38)).
- **Desktop:** done. A tag that changes every 15 minutes, which only paired devices
  recognise; the name only while Add Device is open; a new key when a device is forgotten.
- **Phone:** the same (`docs/protocol.md` §4.16, mobile#12).
- **Done when:** a stranger on the same network sees no name and nothing to follow.

### One Accept for a batch — M
Receiving 20 photos asks once, or not at all ([#32](https://github.com/LinkSy-mg/linksy-desktop/issues/32)).
- **Desktop:** a selection is one offer from every send path; offers from one device a few
  seconds apart merge into one prompt; "Always accept from this device" on the prompt and
  the device's page.
- **Phone:** the same, both ways.
- **Done when:** sending 20 photos either way asks once.

## 0.10 — Lighter and smoother

### Lighter at rest — M
*Service done in 0.11.2: built for size (fat LTO, `opt-level = "s"`, QUIC, TLS and hashing
kept at full speed), it averages 14.2 MB at rest with away from home on, sampled every 10 s
for 15 minutes, from 17.7 MB; transfers as fast as before. About 5 MB of it is its own
memory, the rest its program code. Away from home keeps its network checks every 20 s on
purpose, for the reactivity. App, after 0.11.2: the timeline builds its newest 20 entries
and more as it scrolls (19.3 to 13.5 MB of live heap with 150 entries), and the allocator's
free memory goes back every minute, so the window settles at about 41 MB instead of 44 to
53. A blank GTK window takes about 31 MB of that on an AMD laptop (GTK and the Vulkan driver).*
Linksy runs all day, so what it holds while idle matters more than its speed. Before: the
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
- **Done when:** idle, the service stays under 15 MB, the app under 45 MB (35 MB isn't
  reachable: a blank GTK window takes about 31 MB on an AMD laptop) and the phone app under
  100 MB in the background, measured the same way each release.

### Screens on X11 without a portal — S, desktop done
*Asked for by [@AndyM48](https://github.com/LinkSy-mg/linksy-desktop/issues/54) (Openbox).
Desktop: on any X11 session the helper reads the primary monitor with `ximagesrc` and
applies the phone's input with XTest (control and touchpad too); tested on an X server with
no portal, through the service.*
Most X11 desktops have no ScreenCast portal, so the phone can't watch the computer's screen.
- **Desktop:** when no portal answers, `linksy-screen` captures the X display with
  `ximagesrc`; Finish Setup stops asking for a portal there.
- **Phone:** nothing.
- **Done when:** the phone watches the screen on Openbox and XFCE with no portal installed.

### Screen control on Hyprland and Sway — M, desktop done
*Desktop: the portal shares the screen, and the phone's input goes through the compositor's
virtual pointer and keyboard (`zwlr_virtual_pointer_v1`, `zwp_virtual_keyboard_v1`), with a
keymap of its own so any character types. Tested on a headless Sway with
xdg-desktop-portal-wlr: control, the second display and the touchpad. Not yet on a real
Hyprland.*
Their portals share screens but have no RemoteDesktop, so the phone could watch and not
drive, and its taps on a second display did nothing.
- **Desktop:** where the compositor offers both protocols, `linksy-screen` takes the input
  itself; positions land on the shared output (by name for a second display, else where the
  portal says the stream is).
- **Phone:** nothing.
- **Done when:** on Hyprland and Sway, the phone controls the screen, types into apps and
  uses its second display by touch.

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

## 0.11 — Never miss a notification

### The phone's notifications, with reply — M
*Asked for by [@yurividal](https://github.com/LinkSy-mg/linksy-desktop/issues/30). Texts
come with it: WhatsApp, Telegram, Signal and the SMS app all post a notification with a reply
field, so one listener brings every messaging app, with no SMS permission.*
The computer's side has been there since 0.2.2 (`notification_posted`, `notification_removed`,
`notification_reply`, `notification_action`, docs/protocol.md §4.5).
- **Phone:** a `NotificationListenerService` in the Linksy app itself (decided 2026-11-02,
  instead of a separate app): each notification with its app, title, text, picture,
  conversation and actions; Reply through its `RemoteInput`; removed when it goes on the
  phone. Asked for in the app, with the steps Android needs for an app installed from the
  web: once the switch is greyed out, App info › ⋮ › Allow restricted settings, then on.
  Linksy's own notifications and ongoing ones (music, navigation) aren't sent. Announce
  `notifications` in `features` when it's on.
- **Installing:** the listener makes Play Protect block a browser install in many countries,
  with no "Install anyway". The download page and the app's update sheet say how: pause
  "Scan apps with Play Protect" while installing, then turn it back on.
- **Desktop:** each one as a notification with its app's icon and Reply; the same
  conversation grouped; a per-app switch on the device page; the last ones listed in the
  app, so one missed isn't gone. Both apps.
- **Not possible without developer options:** one-time codes, which Android 15 hides from
  listeners like this one.
- **Done when:** a WhatsApp or SMS message shows on the computer within a second, a reply
  from the computer is sent from the phone, and reading it on either side clears it on both.
- **Later, with calls (0.15):** the SMS app's own threads and writing new texts.

### Snap to computer — S
Take a photo or a page with the phone, and it's on the computer straight away.
- **Phone:** a "Snap to Computer" action (in the app and as a Quick Settings tile) that
  opens the camera and sends the picture the moment it's taken; for a page, its edges are
  found and it's straightened on the phone, with no Google services.
- **Desktop:** the picture lands on the clipboard and in Downloads/Linksy, with a
  notification to open it. "Snap from Phone" on the phone's page asks the phone to open its
  camera. Both apps.
- **Done when:** from the computer, Snap from Phone, take the picture, and paste it into a
  document within two seconds.

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

### Wi-Fi Direct at full speed on any router — M
A direct link ran at about 79 MB/s when the phone picked the channel the computer's Wi-Fi
was on, and about 17 MB/s when it picked another (5220 MHz against 5240): the computer's
one Wi-Fi chip keeps hopping between the two.
- **Desktop:** say which channel its Wi-Fi is on when it asks for a link.
- **Phone:** create the group on that channel (`setGroupOperatingFrequency`) and let the
  computer join it, instead of choosing during negotiation. First check that
  NetworkManager can join a group that already exists.
- **Done when:** a big transfer runs at full speed whatever channel the router uses.

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

### Connection Doctor — M
Most "it doesn't connect" reports come down to the network or the phone's battery saving.
One button finds which, says it in a sentence, and fixes what it can.
- **Desktop:** "Why Can't I See My Phone?" in the device list and Finish Setup checks, in
  order: Linksy's port and mDNS in the firewall; a VPN, or the phone on another network; a
  router that keeps devices apart (their announcements never arrive, the phone's last
  address doesn't answer); the cable's state. Each finding with a button where Linksy can
  act: open the firewall, connect by address, use the cable.
- **Phone:** the same from its side: Wi-Fi on and the same network, battery restrictions
  on Linksy (with the steps for the phone's brand), and whether it hears the computer's
  announcements.
- **Done when:** a guest network that isolates devices, a VPN, a closed firewall and a
  phone with battery saving on are each named correctly, with the right fix.

## 0.13 — The cable, for real

### USB that works on every phone — L
The 0.5.7 cable (Android Open Accessory) depends on each phone maker: a realme 15 Pro and a
Samsung A05 switch fine, then the app never takes the cable (0 packets; releases#8). It
also needs a udev rule (a password), a list of phone makers to spot phones (a ZTE was
missed, desktop#67), stops file transfer by cable and asks which app should open it.
**Linksy Cable** keeps accessory mode, fixed, and nothing else: no developer options (ADB is
out: Linksy shouldn't need them), no toggle on each plug-in (USB tethering is out, and it
stalled on the realme anyway). Android Auto's wired mode runs on accessory mode, so every
phone has it; the failures were ours, as Android's source shows (docs/protocol.md §4.11).
- **Desktop (done, 0.11):** framing version 2 shared with the phone (`linksy_core::cable`:
  hello, keepalives, a stalled cable let go in 5 s), batches ending on a short USB packet,
  the switch tried again around Android's 10 s window, phones in charging-only mode
  recognised by maker (udev rule version 2).
- **Phone:** the accessory's file handed to Rust (`phone_bridge`, 16 KB reads, since Android
  drops what a smaller read can't hold), and the activation fixes in §4.11.
- **Done when:** plugging in and pressing Connect gives "Connected by USB" on a realme, a
  Samsung, a Pixel, a Xiaomi and a Motorola, every time.

### The phone's photos on the computer — M
Builds on photo backup (0.4): browsing instead of copying everything.
- **Protocol:** the phone's albums; an album's photos and videos, newest first, a page at a
  time; a small thumbnail of each; the full file through the usual transfer; and a note
  when the library changes.
- **Phone:** answers from the phone's media library (asked for photo and video access when
  first used), and says when a photo or screenshot is added.
- **Desktop:** a Photos page for the phone: albums, a grid of thumbnails loaded as you
  scroll, one picture opened large, saved or dragged to any folder; a new screenshot
  offered as soon as it's taken. Both apps.
- **Done when:** a library of 10,000 photos scrolls smoothly on the computer, and a
  screenshot taken on the phone shows there within a second.

## 0.14 — Your relay, or none

### Away from home becomes optional — S, desktop done in 0.11.3
*Your own relay is built by [@yurividal](https://github.com/yurividal) and shipped in desktop 0.8.4 and Android 0.2.3 ([desktop#31](https://github.com/LinkSy-mg/linksy-desktop/pull/31), [mobile#7](https://github.com/LinkSy-mg/linksy-mobile/pull/7)); the hosting guide is [server#2](https://github.com/LinkSy-mg/linksy-server/pull/2).
Desktop 0.11.3: Away From Home off (Preferences, or `linksy settings away off`) closes the
relay connection at once and opens none after a restart; with Check for Updates off too,
nothing leaves the local network. `cargo build -p linksyd --no-default-features` leaves
iroh out (the service is 8.5 MB instead of 14). Tested on two services on loopback; the
phone side remains.*
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

### The phone's controls on the computer — S
- **Phone:** its volumes (media, ringtone, alarm), ringer mode, Do Not Disturb (with
  Android's Do Not Disturb access) and the flashlight, set when the computer asks and
  reported when they change.
- **Desktop:** sliders and switches for them on the phone's page; "Keep Do Not Disturb in
  Sync": the computer's and the phone's follow each other (GNOME and KDE). Both apps.
- **Done when:** muting the phone from the computer, or turning on Do Not Disturb on the
  computer, silences the phone at once.

## 0.16 — Optional extras

### Control the phone's screen from the computer — M
The phone's screen already shows on the computer. This adds the mouse and keyboard driving
it, like scrcpy, with no developer options.
- **Over the cable (desktop done early, in 0.11):** Android Open Accessory 2.0 plugs a
  virtual touchscreen and keyboard into the phone, as scrcpy's OTG mode does; no phone
  change needed. Taps land where clicked, keys go by position so the phone's own layout
  applies, text goes through the clipboard. Works on a realme 15 Pro; reported working by
  scrcpy users on Samsung, Xiaomi, Pixel, Motorola, Huawei, vivo and Sony, failing on a
  few older phones (Galaxy J7 Pro, Mi 5s).
- **Without the cable (phone):** an accessibility service gives the same over any link.
  Sideloaded installs need "Allow restricted settings" in Linksy's App info once.
- **Phone:** a lasting "controlled from <computer>" notice with Stop; the rotation for
  landscape.
- **Done when:** a phone app can be used from the computer without touching the phone,
  by cable on every phone tested, and over Wi-Fi with the accessibility service.

### Automatic clipboard sync — M
Today, sending the clipboard is a button.
- **Computer to phone:** automatic wherever the desktop allows watching the clipboard
  (KDE, Hyprland, Sway, X11; not GNOME).
- **Phone to computer:** Android only lets the app in front, or the keyboard, read the
  clipboard. Automatic while Linksy is open, and through a "Copy to Computer" entry in
  Android's text menu otherwise.
- **Done when:** text copied on one side is ready to paste on the other, with no button.

### The phone's sound with its screen — M
- **Phone:** while it shares its screen, it also captures what its apps play (Android 10+,
  for apps that allow it) and sends it with the picture.
- **Desktop:** played in step with the picture, with a mute button in the viewer. Both
  apps.
- **Done when:** a video playing on the phone is heard on the computer, in sync, while its
  screen is shown.

### Deck — M
The phone as a deck of buttons for the computer, laid out exactly the way you want.
- **Phone (owns the layout):** pages of tiles on a grid, edited right on the deck: move
  them, resize them (one cell up to wide, tall or big), drag them between pages and into
  folders, pick each one's label, colour, icon or picture; sliders for volume. Tiles show
  live state (playing, the volume, the microphone muted). Saved on the phone, a deck per
  computer, with a Quick Settings tile to open it.
- **Desktop (says what's allowed):** a list of what this computer offers, each on or off
  for that phone: play/pause, next and previous, volume and mute, microphone mute, the
  sound output, keyboard shortcuts, opening an app, OBS scenes and recording, lock, sleep,
  and commands of your own (a name and a command line). The phone only ever asks for an
  action by its id: anything off, or not on the list, is refused. Live state is sent while
  the deck is open. Both apps.
- **Protocol:** the computer sends its actions (id, name, kind, on or off) and their state;
  the phone sends "run this action", with a value for a slider.
- **Done when:** a deck of mixed-size tiles is arranged on the phone by dragging and every
  tile works on the computer, and an action turned off on the computer stops at once.

### Videos open where you left off — S
Builds on Open on PC (0.2) and the notification listener (0.11), which lets the phone see
what's playing and where.
- **Phone:** "Continue on Computer" for the video playing now: its link and position.
- **Desktop:** opens it at that position (YouTube and other sites that take a time in the
  link; the plain link elsewhere).
- **Done when:** a YouTube video playing on the phone carries on on the computer within a
  second of where it was.

### The computer's notifications on the phone — S
- **Desktop:** the computer's own notifications, chosen per app and off by default, sent
  to the phone; never the ones Linksy shows for the phone itself.
- **Phone:** shown as Android notifications, grouped per computer, with their buttons.
- **Done when:** a "build finished" or new-mail notification on the computer shows on the
  phone within a second.

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
- [@AndyM48](https://github.com/LinkSy-mg/linksy-desktop/issues/54): the camera and Finish
  Setup on Artix without systemd (0.9.13, 0.9.14).
- [@bearyjd](https://github.com/LinkSy-mg/linksy-desktop/pull/56): device names cleaned
  before they're shown, so they can't disguise a pairing prompt (0.9.14).

## How each feature ships

1. The protocol messages are added to `linksy-core` with tests, and documented in
   `docs/protocol.md` §4.
2. The desktop side lands and is tested against a fake phone in the end-to-end tests.
3. The phone side lands (working from `docs/protocol.md`).
4. Both are tested together on a real phone, then released.
