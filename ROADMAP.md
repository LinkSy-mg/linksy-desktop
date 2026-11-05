# Linksy roadmap

Linux desktop (GTK, and Qt for KDE Plasma) + Android phone. No Windows or macOS plans. Each feature lists what the
desktop (`linksy-desktop`) and the phone (`linksy-mobile`, Flutter) each have to build, so
both sides can work in parallel. Protocol details go into `docs/protocol.md` §4 as each one lands.

Effort: **S** = a day or two, **M** = a few days, **L** = a week or more.

Linksy for Linux is at **0.12.4** and Linksy for Android at **0.3.4** (2026-11-05). They have their
own version numbers, and this roadmap has none: a feature ships when it is ready, not with a
number. It says what is **Done**, what is **Now**, **Next** and **Later**. Which side has which
feature, feature by feature, is `docs/protocol.md` §2.

## Done

| | Linux | Android |
|---|---|---|
| Files and folders, clipboard (text, pictures, files), Ping, Open on PC, battery, Find | 0.2 | 0.1 |
| The computer's music and volume on the phone, pause on calls | 0.3, 0.9.8 | 0.2.6 |
| Lock when away, photo auto-import | 0.3, 0.4 | 0.1 |
| Touchpad, keyboard and slides | 0.4 | 0.1 |
| The phone's screen, webcam (up to 1080p, 60 fps), microphone and second display | 0.5 to 0.6 | 0.1 |
| The phone watching and controlling the computer's screen, Choose Screen | 0.9.11 | 0.2.12 |
| The phone's files in the Files sidebar and a file manager for them | 0.5.1, 0.12.1 | 0.1 |
| Away from home over the internet (7-day lease), Locate | 0.6 | 0.1 |
| Your own relay | 0.8.4 | 0.2.3 |
| The terminal, behind a code set on the computer | 0.7 | 0.2.0 |
| A Qt 6 app for KDE Plasma | 0.8.4 | |
| One-password Finish Setup, ten languages | 0.8 | 0.2 |
| Private discovery, one Accept for a batch, problem logs | 0.9 | 0.2.4 |
| Hyprland, Sway, niri, X11 and distributions without systemd | 0.9 to 0.10 | |
| Upgrades in one click, Ubuntu 24.04 and Mint 22, the full Flatpak | 0.9 to 0.10 | Update Now 0.2.4 |
| Wi-Fi Direct for big files | 0.9.9 | 0.2.8, between two phones 0.2.9 |
| Lighter at rest, transfers in a corner panel, screens on X11 without a portal, screen control on Hyprland and Sway | 0.10 to 0.11.3 | |
| **Linksy Cable**: the USB cable, version 2 (it works on some phones; every phone is Next) | 0.11.0 | 0.2.2 to 0.3.3 |
| **Controlling a phone's screen over the cable**, with no developer options | 0.11.0 | nothing to do |
| The phone's notifications on the computer, with reply; the Messages page and copied codes | 0.11.4 | 0.3.2 |
| **The Inbox**: other notifications, earlier messages, replies after the notification | 0.12.1 | 0.3.3 |
| **The Gallery**: the phone's photos and videos, page by page | 0.12.1 | 0.3.3 |
| **The Deck**, with layouts made on the computer (the editor is GTK only) | 0.12.1 | 0.3.3 |

## Now

On main and not in a release yet:
- **Turn the Inbox or Gallery on from the computer** (§4.21). The phone side is built and needs the core pin that has the message.
- **Phone to phone** (§4.22): the route a transfer takes, a check before sending, sending apps. The computer side is done; the phone side is in progress.
- Day headings in the Gallery ("Today", "Yesterday", "Nov 2"), a seek bar that works for browser music, and a service that starts again after a crash.

Built on the phone, not published (from the phone's side; the desktop has not seen its messages yet):
- **Snap to Computer** and **Copy to Computer**, the phone's half of the **Connection Doctor**, and a **USB button** (`usb_plugged`, `usb_connect`, which the computer already answers).

Built on the computer, on branch `phone-work` (not on main yet):
1. Snap from Phone: the picture lands in Downloads/Linksy and on the clipboard, with a notification to open it (§4.23).
2. The Connection Doctor: "Why Can't I See My Phone?" in the device list and Finish Setup.

Being built next: the phone's controls (volumes, ringer, Do Not Disturb, flashlight).


## Now, in detail

### Snap to computer — S
Take a photo or a page with the phone, and it's on the computer straight away.
- **Phone (built, not published yet):** a "Snap to Computer" action (in the app and as a Quick Settings tile) that
  opens the camera and sends the picture the moment it's taken; for a page, its edges are
  found and it's straightened on the phone, with no Google services.
- **Desktop:** the picture lands on the clipboard and in Downloads/Linksy, with a
  notification to open it. "Snap from Phone" on the phone's page asks the phone to open its
  camera. Both apps.
- **Done when:** from the computer, Snap from Phone, take the picture, and paste it into a
  document within two seconds.

### Connection Doctor — M
Most "it doesn't connect" reports come down to the network or the phone's battery saving.
One button finds which, says it in a sentence, and fixes what it can.
- **Desktop:** "Why Can't I See My Phone?" in the device list and Finish Setup checks, in
  order: Linksy's port and mDNS in the firewall; a VPN, or the phone on another network; a
  router that keeps devices apart (their announcements never arrive, the phone's last
  address doesn't answer); the cable's state. Each finding with a button where Linksy can
  act: open the firewall, connect by address, use the cable.
- **Phone (built, not published yet):** the same from its side: Wi-Fi on and the same network, battery restrictions
  on Linksy (with the steps for the phone's brand), and whether it hears the computer's
  announcements.
- **Done when:** a guest network that isolates devices, a VPN, a closed firewall and a
  phone with battery saving on are each named correctly, with the right fix.

### The phone's controls on the computer — S
- **Phone:** its volumes (media, ringtone, alarm), ringer mode, Do Not Disturb (with
  Android's Do Not Disturb access) and the flashlight, set when the computer asks and
  reported when they change.
- **Desktop:** sliders and switches for them on the phone's page; "Keep Do Not Disturb in
  Sync": the computer's and the phone's follow each other (GNOME and KDE). Both apps.
- **Done when:** muting the phone from the computer, or turning on Do Not Disturb on the
  computer, silences the phone at once.

## Next

### Wi-Fi Direct at full speed on any router — M
A direct link ran at about 79 MB/s when the phone picked the channel the computer's Wi-Fi
was on, and about 17 MB/s when it picked another (5220 MHz against 5240): the computer's
one Wi-Fi chip keeps hopping between the two.
- **Desktop:** say which channel its Wi-Fi is on when it asks for a link.
- **Phone:** create the group on that channel (`setGroupOperatingFrequency`) and let the
  computer join it, instead of choosing during negotiation. First check that
  NetworkManager can join a group that already exists.
- **Done when:** a big transfer runs at full speed whatever channel the router uses.

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

## Later

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

### The computer's notifications on the phone — S
- **Desktop:** the computer's own notifications, chosen per app and off by default, sent
  to the phone; never the ones Linksy shows for the phone itself.
- **Phone:** shown as Android notifications, grouped per computer, with their buttons.
- **Done when:** a "build finished" or new-mail notification on the computer shows on the
  phone within a second.

### The phone's sound with its screen — M
- **Phone:** while it shares its screen, it also captures what its apps play (Android 10+,
  for apps that allow it) and sends it with the picture.
- **Desktop:** played in step with the picture, with a mute button in the viewer. Both
  apps.
- **Done when:** a video playing on the phone is heard on the computer, in sync, while its
  screen is shown.

### The Qt Deck editor — M
The phone as a deck of buttons for the computer, laid out exactly the way you want.
- **Desktop (keeps the layouts):** an editor with the phone drawn true to size, portrait
  and landscape: tiles (buttons, volume, music, clock) dragged in from a library, moved,
  resized and themed (Stock, Dot Matrix, E-Ink, Terminal, Pixel), several layouts per
  phone. What this computer offers, each on or off for that phone: media, volume and
  mute, microphone mute, the sound output, lock, Do Not Disturb, Wi-Fi, Bluetooth,
  airplane mode, power modes, restart, shut down, and commands of your own. The phone only
  ever asks for an action by its id: anything off, or not on the list, is refused. Live
  state is sent while the deck is open. The GTK editor is done (0.12.1); the Qt app has the deck's actions and layouts but not
  the editor yet.
- **Phone (draws it):** the layout from the computer, kept for when it's away, with its
  own themes if it likes; asks first for restart, shut down and the like; keeps the screen
  on while the deck shows, dimmed after a minute.
- **Protocol:** the computer sends its actions, their state and the layouts (§4.19,
  §4.20); the phone sends "run this action", with a value for a slider.
- **Done when:** a deck of mixed-size tiles is arranged on the phone by dragging and every
  tile works on the computer, and an action turned off on the computer stops at once.

### Calls on the computer — L
- **Control:** see who's calling, answer, decline and hang up from the computer (the phone
  answers with `ANSWER_PHONE_CALLS`). This part needs no Bluetooth.
- **Audio:** talking through the computer's microphone and speakers needs the computer to
  act as a Bluetooth hands-free device (HFP) for the phone, through PipeWire. It depends on
  0.11, and on how well each computer's Bluetooth does HFP.
- **Done when:** a call can be answered on the computer and held through its headset.
- **Maybe** because the audio part depends on hardware we don't control. If it isn't
  reliable, only call control ships.

### Automatic clipboard sync — M
Today, sending the clipboard is a button.
- **Computer to phone:** automatic wherever the desktop allows watching the clipboard
  (KDE, Hyprland, Sway, X11; not GNOME).
- **Phone to computer:** Android only lets the app in front, or the keyboard, read the
  clipboard. Automatic while Linksy is open, and through a "Copy to Computer" entry in
  Android's text menu otherwise.
- **Done when:** text copied on one side is ready to paste on the other, with no button.

### Videos open where you left off — S
Builds on Open on PC (0.2) and the notification listener (0.11), which lets the phone see
what's playing and where.
- **Phone:** "Continue on Computer" for the video playing now: its link and position.
- **Desktop:** opens it at that position (YouTube and other sites that take a time in the
  link; the plain link elsewhere).
- **Done when:** a YouTube video playing on the phone carries on on the computer within a
  second of where it was.

### Onboarding — M
At most five slides, skippable, shown once: welcome; get the phone app (a QR code to the
download); pair; finish setup (the dependency check, each missing piece with its fix
button); done, with where the main features are.
- **Done when:** a fresh install goes from first launch to a paired phone, with every
  feature the computer can do working, without opening a terminal.

---

## Not planned

- **Controlling the phone's screen without the cable.** It needs an accessibility service,
  which a sideloaded app can't have. Over the cable it works already.
- **Texting from the computer through SMS permissions.** The same block applies. Texts reach
  the computer as the phone's notifications, with Reply.
- **Automatic clipboard sync on GNOME.** GNOME doesn't let apps watch the clipboard; manual
  Send Clipboard stays.
- The caller's name, browsing the phone's whole storage, Windows and macOS, and unlocking the
  computer from the phone.

After the features above, Linksy is complete and everything after that is fixing: fixes,
performance, polish and stores (Flathub, AUR).

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
