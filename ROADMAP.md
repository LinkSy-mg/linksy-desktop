# Linksy roadmap

Linux desktop (GTK, and Qt for KDE Plasma) + Android phone. No Windows or macOS plans. Each feature lists what the
desktop (`linksy-desktop`) and the phone (`linksy-mobile`, Flutter) each have to build, so
both sides can work in parallel. Protocol details go into `docs/protocol.md` §4 as each one lands.

Effort: **S** = a day or two, **M** = a few days, **L** = a week or more.

Linksy for Linux is at **0.16.0** and Linksy for Android at **0.3.6** (2026-11-14). They have their
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
| **The Deck**: the phone as buttons for the computer; the computer decides what it may run (0.12.1), your own commands with groups, icons and switches, and layouts saved from the phone's own editor (0.14.0) | 0.14.0 | 0.3.5, saving to do |
| Snap from Phone, the Connection Doctor ("Why Can't I See My Phone?"), a phone's volumes, ringer, Do Not Disturb and flashlight on its page | 0.13.0 | its half built, see Now |
| This computer's notifications on the phone, the phone's sound with its screen, screens at up to 60 fps, the Inbox and Gallery turned on from the computer | 0.13.0 | see Now |
| Phone to phone: the route a transfer takes, a check before sending, sending apps | 0.13.0 | in progress |
| **A welcome on the first run**, in both apps: the phone's app, pairing and Finish Setup in five slides | 0.15.0 | |
| `linksy doctor` checks what the camera, microphone and screens need and tries them; Finish Setup checks every piece they use | 0.14.1, 0.15.0 | |
| Away from home carries files through the relay when there is no direct path, at a speed shared between the devices sending | the relay, November 2026 | nothing to do |

## Now

The computer's side of everything below is released. What is left is on the phone
(as the phone's side last reported it; the computer has not seen all of its messages yet):
- **Saving a deck to the computer** and **custom themes** (§4.26): the editor is in 0.3.5, saving and themes are to do.
- **Phone to phone** (§4.22): in progress.
- **Snap to Computer** and **Copy to Computer**, the phone's half of the **Connection Doctor**, and a **USB button**: built, not published.
- **The phone's controls**, **this computer's notifications on the phone**, **the phone's sound with its screen** and **60 fps** decoding: to do.
- **Connections that survive a network change**: a phone drops its links when Android reports a network change, even when the Wi-Fi in use hasn't changed, and a transfer cut that way fails instead of carrying on (seen 2026-11-13, phone to phone, 410 MB).

On the computer, next:
- **Deck themes**: your own looks, made on the computer and tried live on the phone (§4.26).

Not done, and why: datagrams for video and the pointer, BBR over the internet and a bitrate that follows the link while it runs (each needs measuring on a real link first); transfers with no shared network and Bluetooth (the first needs the computer to host a hotspot, which cuts its own Wi-Fi; the second needs a phone); the Wi-Fi Direct speed measurement (needs a phone).


## Now, in detail

### Snap to computer — S
Take a photo or a page with the phone, and it's on the computer straight away.
- **Phone (built, not published yet):** a "Snap to Computer" action (in the app and as a Quick Settings tile) that
  opens the camera and sends the picture the moment it's taken; for a page, its edges are
  found and it's straightened on the phone, with no Google services.
- **Desktop (done, 0.13.0):** the picture lands on the clipboard and in Downloads/Linksy, with a
  notification to open it. "Snap from Phone" on the phone's page asks the phone to open its
  camera. Both apps.
- **Done when:** from the computer, Snap from Phone, take the picture, and paste it into a
  document within two seconds.

### Connection Doctor — M
Most "it doesn't connect" reports come down to the network or the phone's battery saving.
One button finds which, says it in a sentence, and fixes what it can.
- **Desktop (done, 0.13.0):** "Why Can't I See My Phone?" in the device list and Finish Setup checks, in
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
- **Desktop (done, 0.13.0):** sliders and switches for them on the phone's page; "Keep Do Not Disturb in
  Sync": the computer's and the phone's follow each other (GNOME and KDE). Both apps.
- **Done when:** muting the phone from the computer, or turning on Do Not Disturb on the
  computer, silences the phone at once.

## Next

### Wi-Fi Direct at full speed on any router — M
A direct link ran at about 79 MB/s when the phone picked the channel the computer's Wi-Fi
was on, and about 17 MB/s when it picked another (5220 MHz against 5240): the computer's
one Wi-Fi chip keeps hopping between the two.
- **Desktop (done on branch `phone-work`, not measured):** says which channel its Wi-Fi is on when it
  asks for a link (`ask` carries `frequency`, §4.17).
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
The computer's screen on a phone now encodes at up to 60 fps (30 before), falling back to 30 by
itself when its encoder can't keep up; the phone's own screen at 60 needs the settings in
`docs/protocol.md` §4.8, and decoding at 60 on the phone is its half. Measured here: a GPU
encoder keeps 60 fps at 1080p; x264 on the CPU keeps 60 at 1080p and 53 at 1440p; a 144 Hz
screen is cut to 60. The first phone-side measurements are not in yet.
Before: screens ran at up to 30 fps (the webcam already did 60).
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
- **Desktop (done, 0.13.0):** the computer's own notifications, chosen per app and off by default, sent
  to the phone; never the ones Linksy shows for the phone itself.
- **Phone:** shown as Android notifications, grouped per computer, with their buttons.
- **Done when:** a "build finished" or new-mail notification on the computer shows on the
  phone within a second.

### The phone's sound with its screen — M
- **Phone:** while it shares its screen, it also captures what its apps play (Android 10+,
  for apps that allow it) and sends it with the picture.
- **Desktop (done, 0.13.0):** played in step with the picture, with a mute button in the viewer. Both
  apps.
- **Done when:** a video playing on the phone is heard on the computer, in sync, while its
  screen is shown.

### The Deck, edited on the phone — M
The phone as a deck of buttons for the computer, laid out exactly the way you want, on the
phone, where it is touched. The computer decides what the phone may run.
- **Desktop (decides, keeps, runs):** one page per phone in both apps. What this computer
  offers, a switch each, by group: media, volume and mute, microphone mute, the sound
  output, lock, Do Not Disturb, Wi-Fi, Bluetooth, airplane mode, power modes, restart, shut
  down, and commands of your own (buttons, or switches with a check that says whether
  they're on, in groups you name, with an icon and an "ask first"). The phone only ever
  asks for an action by its id: anything off, or not on the list, is refused, and a layout
  the phone saves never turns one on: what its tiles need that is off shows as asked for,
  with a notification to allow it. The computer keeps each phone's layouts (§4.26). Done in
  0.14.0; the GTK editor of 0.12.1 is gone.
- **Phone (edits and draws it):** the editor is done (0.3.5); saving to the computer
  (`deck_layout_save`) and custom themes (`deck_themes`) are to do on the phone.
- **Themes (next, desktop):** your own looks, made on the computer from one of the five and
  shown live on the phone before they're kept (§4.26, `deck_theme_draft`).
- **Done when:** a deck is arranged on the phone, saved to the computer, and every tile
  works; an action turned off on the computer stops at once, and one the phone asks for
  waits for your OK.

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
- [@MrAvana](https://github.com/LinkSy-mg/linksy-desktop/issues/89): the phone's
  microphone going silent on PipeWire 1.6, with the cause, the fix and the levels measured
  on a real phone (0.13.3, 0.14.1).
- [@bikathi](https://github.com/LinkSy-mg/linksy-desktop/issues/85): the camera closing at
  once where GStreamer lacks a plugin (0.14.1).
- [@ichirou2910](https://github.com/LinkSy-mg/linksy-desktop/pull/88): the Arch packages no
  longer depending on each other (0.15.0).
- [@Lur1fa](https://github.com/LinkSy-mg/linksy-desktop/pull/90): the service no longer
  starting for the login screen, which kept phones from reconnecting on GNOME (0.15.0).

## How each feature ships

1. The protocol messages are added to `linksy-core` with tests, and documented in
   `docs/protocol.md` §4.
2. The desktop side lands and is tested against a fake phone in the end-to-end tests.
3. The phone side lands (working from `docs/protocol.md`).
4. Both are tested together on a real phone, then released.
