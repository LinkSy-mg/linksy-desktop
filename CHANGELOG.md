# Changelog

Linksy for Linux. Each version's section becomes its release notes.

## 0.15.0

### Added
- **A welcome on the first run,** in both apps: what Linksy does, a QR code to the phone's
  app, pairing right there (a phone that pairs moves it on), Finish Setup with its buttons,
  and where things are. Five slides, skippable, shown once, and never to someone who already
  used Linksy on the computer.
- **`linksy doctor` checks the camera, microphone and screens:** what each lacks here, then a
  test picture encoded and decoded and a test tone, for real (`linksy-screen check`). The
  tarball's installer says the same at the end, since a tarball brings no dependencies.

### Fixed
- **The packages bring gst-plugins-bad:** `h264parse` comes from it, and neither the phone's
  camera nor screen sharing runs without it. It was only recommended.
- **Paired phones reconnect after a restart on GNOME.** The service also started for the
  login screen, where it took Linksy's port, so yours got another one and phones no longer
  recognised the computer. It now starts for people only (thanks, Lur1fa).
- **Arch: `linksy-common` no longer needs a window package,** which made the two depend on
  each other and could fail upgrades with yay (thanks, ichirou2910).

### Changed
- **Finish Setup has a Microphone and Sound line:** what plays the phone's sound and sends
  this computer's microphone was never checked.
- What each feature needs from GStreamer is now one table that Finish Setup, the check above
  and a test share: an element a pipeline uses that nothing checks fails the build.

## 0.14.1

### Fixed
- **The phone's camera no longer closes at once where GStreamer lacks `v4l2sink`** (an
  install without gst-plugins-good, as from the tarball). It is then offered to PipeWire apps
  only, and the log says so. The same the other way round: without PipeWire's plugin it stays
  on the Linksy Camera device.
- **A session that can't start for a missing piece says so.** A notification on the computer
  tells that GStreamer lacks something and points to Finish Setup; before, the phone's screen
  just closed and the reason was only in the log.
- **Finish Setup checks what the camera needs:** `h264parse` (gst-plugins-bad) and `v4l2sink`
  (gst-plugins-good) are looked for and installed with the rest. It said everything was ready
  without them.
- **The phone's microphone is 12 dB louder.** A phone sends what its microphone hears without
  the gain its own recorder adds, so voices came in very quiet; they are now level with the
  phone's recorder. Nothing to set, and it holds between sessions.
- **The phone's camera doesn't hold the phone's stream back while no app reads it** (computers
  without the Linksy Camera device): frames nobody takes are dropped, so an app that opens the
  camera later gets the picture of that moment.

## 0.14.0

### Changed
- **The deck is edited on the phone.** The computer's own deck editor is gone (it was in the
  GTK app only). A phone's deck page on the computer is now what it may do here: a grid of
  tiles, a switch each, by group, with Turn All On and Off for the ones that don't ask first;
  what its saved deck asked for; and your own commands. Both apps have it, the same. Layouts
  you made on the computer before stay, and show on the phone.
- **The Qt app's device page matches the GTK app's:** Send Files is a big dashed tile (also
  where files are dropped, and Ctrl+O now chooses them there too), with Clipboard, View
  Screen, Ring and Locate as cards beside it, in the style of the other cards. The devices
  column starts narrower, and the list's sides line up with the buttons under it.
- **A phone can't turn on an action by putting a button for it on its deck.** Restart, Shut
  Down, Airplane Mode and your commands stay off until you allow them on the computer. When a
  saved deck uses one, the page lists it under "Asked for by" the phone, and a notification
  offers Allow.

### Added
- **The computer keeps the deck the phone saves** (`deck_layout_save`, docs/protocol.md
  §4.26): saved on the version the computer has, kept one version on and sent back; a stale
  one is refused with the current layouts, a broken one with why. The phone side of saving is
  in the next phone release.
- **Your commands can have a group, an icon and a question first,** and be a switch: one
  command to turn on, one to turn off (or the same flips it), and a check that succeeds while
  it is on. The deck shows the switch's state, asked every 3 seconds while it is open.
- **Try It Now** beside a command line: runs it once and shows the exit code and what it
  printed.
- Network, Power and Time group names are translated in the Qt app, like in the GTK app.

### Fixed
- **A device's page no longer shifts as you change tabs.** In the GTK app the page was
  narrower on tabs with narrower content (Inbox, Files), and in the Qt app it moved by the
  width of the scrollbar when a tab was long enough to scroll.

## 0.13.3

### Fixed
- **The phone's microphone gave no sound on PipeWire 1.6** (#89): a PipeWire node
  published the way Linksy did it stops delivering after a moment there (PipeWire's own
  issue 5378). The microphone is now made through PipeWire's PulseAudio layer (`pactl`): a
  null sink the audio is played into, and a microphone listening to it. Both are removed
  when the session ends, and the service asks the helper to stop before it kills it, so
  they go every time. Where there is no `pactl` it works as before. The camera's PipeWire
  node (OBS's camera source) is published the same way and is not changed yet; browsers
  and Zoom use the Linksy Camera device.

## 0.13.2

### Fixed
- **Firefox still showed twice on the phone** (#87): 0.13.1 matched Plasma's browser
  integration to the browser's own player only by an identical track title, which a real
  Firefox doesn't give. The browser's own player now also goes when it names the same
  desktop entry or app.
- **The camera ended at once on some installs** (#85): without GStreamer's `capssetter`
  (the "debug" plugin of gst-plugins-good), every camera session stopped with "no element
  capssetter". The frame rate is set another way there, and the log says so.
- **A Ping showed "From <phone>" twice** on the computer: the phone's text already says it.
- **The wheel on a volume slider jumped to 0, 50 or 100%:** it moves by 5% now, in the
  GTK app (and by one stop on the camera's exposure).
- **Notices piling up** when a phone leaves and re-enters its touchpad page: a session
  allowed without asking is announced once a minute, not at every start.

## 0.13.1

### Fixed
- **A browser's track showed twice on the phone** (#87): with Plasma's browser
  integration, Firefox's own player and Plasma's both went to the phone. Where Plasma's
  has the track, the other copy is left out.
- **Taps on the phone as a second screen, on KDE** (#51): with the phone's display at
  100% and the computer's own screen scaled (125%), taps landed too far right and down.
  They now follow the first screen's scale in that case, and the log says which scale.
- **Finish Setup leaves a trace** (releases#9): what the setup helper printed goes to
  `~/.local/state/linksy/setup.log`, and Save Logs includes the newest of it. A helper
  that couldn't run no longer says the password wasn't given, and the camera helper says
  when it couldn't write the service that brings the camera back after a restart.
- **The deck editor showed colours that did nothing:** a tile's colour is drawn by the
  Pixel theme only, and the layout's accent by Stock and Dots only. The other themes no
  longer offer them.

## 0.13.0

Most of what is new here works with Linksy for Android 0.3.5.

### New
- **A phone's volumes, ringer, Do Not Disturb and flashlight on its page:** Sound and
  Silence, under Use As, in both apps (and `linksy controls`). Keep Do Not Disturb in
  Sync carries the switch both ways between the phone and GNOME or Plasma, whichever you
  changed last.
- **This computer's notifications on the phone:** a switch on the phone's page, off until
  you turn it on, and `linksy forward`. Linksy's own, volume and brightness pop-ups,
  apps you muted and anything while Do Not Disturb is on stay here.
- **The phone's sound with its screen:** when a phone's screen is shown here, what it
  plays comes out of this computer, and stops with the screen.
- **Snap from Phone:** a tile on the phone's page (and `linksy snap`) opens its camera;
  the picture lands in Downloads/Linksy and on the clipboard.
- **Why Can't I See My Phone?:** in the device list menu, in Finish Setup and as
  `linksy doctor`. It looks for a VPN carrying the default route, a phone last seen on
  another network, a phone that doesn't answer at its last address and a cable that
  isn't connected, and says what to do, with a button where Linksy can act.
- **Turn on a phone's Inbox or Gallery from here:** both tabs show while the feature is
  off, with Ask the Phone.
- **Phone to phone:** transfers say which way they go (direct link, USB cable, same
  network or over the internet) and why there wasn't a direct link. A phone without
  room for a file says how much it needs and has. Apps travel between phones as
  their APK files.
- **Gallery by day:** photos sit under a heading for each day, in both apps.
- **Controlling a phone in landscape:** taps and scrolls land where they belong when the
  phone is turned (over the cable).

### Improved
- **Screens at up to 60 frames a second** (a 144 Hz screen is cut to 60), falling back to
  30 by itself when the computer can't keep up. Over the internet a screen is sent at 4
  Mbit/s and 30 fps; the local network and the cable keep 16 Mbit/s.
- **Direct links on the right channel:** the computer tells the phone which Wi-Fi channel
  it is on, so Wi-Fi Direct can start there.
- **Pair by Address on a device you already paired** reconnects it instead of starting a
  new pairing; this is how a phone on another subnet of the same home gets back.
- **The service starts again by itself** after a crash when nothing else restarts it
  (the Flatpak, an autostart entry, the app starting it).
- **A phone's player bar and seeking:** browsers that report no position now show it, and
  a seek goes exactly where it was dragged.
- **Gallery and Files from away:** when the phone refuses because it isn't on the same
  network, the message names the setting that allows it.

### Fixed
- **Quit in the tray left the window open** (#84). Both apps now close with the service.
- **The deck editor's right panel was hidden on a narrow window,** with nothing to say it
  was there. A Details tab on the right edge of the phone opens it.
- **Flatpak: cover art and this computer's picture** were missing on the phone, because
  the sandbox can't see the folders they live in. They are now read from the computer,
  and Finish Setup says when the Flatpak has lost the access it needs for that.
- **A picture over 128 KiB** (many account photos) was never sent to the phone. It is
  now scaled down to fit.
- **Two services at login** (#82): the Flatpak started the service from two autostart
  entries, and services started together could both stay. One stays now.

## 0.12.4

### Fixed
- **Linksy stopped in the background:** clicking or swiping away a phone's notification
  on the computer, or pressing one of its buttons, made the service quit, and the app
  then said it wasn't running. It no longer does.
- **The Gallery tab keeps up with deletions:** photos deleted on the phone left the
  Gallery until Linksy restarted. They now go at once, new ones come in where they
  belong, and what you scrolled to and selected stays. A change while another tab
  shows is caught up when you come back. Both apps (#81).

## 0.12.3

### Improved
- **Lighter at rest:**
  - With no devices yet, the logo by "Looking for devices…" pulsed for as long as the
    window stayed open, redrawing it every frame (about 9% of a core). It now pulses
    three times when it appears, then holds still (GTK app).
  - A transfer waiting to start no longer keeps the window redrawing (GTK app).
  - A phone's page checks how fast it answers only while the window is in use, and
    right away when you come back to it (both apps).

## 0.12.2

### New
- **Start When You Log In, on or off:** a switch in Preferences, in both apps. Turned
  off, Linksy keeps running until you log out and doesn't start at the next login, and
  Finish Setup doesn't turn it back on. Works with packages, the tarball and the Flatpak
  (#80).

### Improved
- **The Qt app for KDE Plasma, redesigned in Plasma's own style:**
  - A phone's page opens with how it's connected in plain words and Send Files beside
    its name.
  - The everyday actions sit in one bar.
  - Each card has its icon in a round tint and its actions in a section of their own.
  - Icons are Breeze's one-colour ones, tinted like the GTK app's; text is calmer and
    counts are soft pills.
  - Your own messages show in a soft tint of your accent colour.
  - The device list ends in a plain Send Files button.
  - Light and Breeze Dark alike.
- **Calmer chips and counts in the GTK app:** how a device is connected, its battery
  and the unread counts no longer shout in bold.

## 0.12.1

### New
- **A device's page in tabs:** Use As, Inbox, Gallery, Files, Transfers, History and
  Settings, with the link, its latency and the battery in the header. The tabs stay at the
  top while the page scrolls. Both apps.
- **Files, a file manager for the phone:** browse the folder the phone shares, copy files
  and folders both ways, make folders, rename and delete. Copies run in the background
  with their progress and Cancel. Works with the Android app you have now.
- **Inbox:** Messages became the Inbox tab, with filters and sorting. The phone's other
  notifications open in full beside the list, earlier messages load from the phone, mail
  shows its subject, and a reply still works after the notification is gone. Those new
  parts need a coming version of Linksy for Android.
- **Gallery:** the phone's photos and videos a page at a time, by album, with each
  thumbnail fetched once. Pick some and save them to Pictures. Needs a coming version of
  Linksy for Android.
- **Deck:** the phone as buttons for this computer: media, volume, mute, microphone, the
  sound output, lock, Do Not Disturb, Wi-Fi, Bluetooth, airplane mode, power modes,
  restart, shut down and commands of your own. Each is on or off for each phone; restart,
  shut down, airplane mode and your commands stay off until you allow them, and the phone
  asks before running them. Needs a coming version of Linksy for Android.
- **Deck editor, in the GTK app (preview):** build the phone's deck on the computer, on the
  phone drawn true to size in portrait and landscape. Drag widgets in, move and resize
  them, pick a theme (Stock, Dot Matrix, E-Ink, Terminal or Pixel) and keep several
  layouts. The Qt app keeps its list of actions for now and gets the editor next.
- **The Qt app catches up:** a phone's running terminals in its Terminal card, each with
  End; how long a device stays reachable from away, in its header; the History tab's
  filters and day headings.

### Changed
- **Use As cards** sit four in a row on a wide page, two on a medium one and one on a
  narrow one, with each title beside its icon.
- **Transfers** moved from a page of its own to each device's Transfers tab.
- **A phone's notification** shows its app in its title, and the phone's name only when
  several phones are paired.

### Fixed
- **Controlling the right phone:** with two phones paired and one plugged in, viewing the
  other one sent your clicks and keys to the phone on the cable. Now only a phone on the
  cable is controlled, and only when it's the one you're viewing
  (LinkSy-mg/linksy-releases#10).
- **A pulled cable is noticed at once:** the phone no longer shows as connected by USB for
  up to 30 seconds after it's unplugged, so it can come back over Wi-Fi right away.
- **Take Control** says Release Control while you control the screen, and the window says
  Screen again once you let go (#78).
- **Names from phones show as typed** in the Qt app, never as formatting.
- **A volume tool that hangs** (wpctl or pactl) is stopped after 5 seconds instead of
  holding the volume up.

## 0.11.4

### New
- **Messages, from the phone's notifications:** a phone's chats, texts and mail in one place
  on its page: WhatsApp, Telegram, Signal, Messenger, Discord, Slack, the SMS app, Gmail,
  Outlook and others. Each conversation shows its picture, its newest message and the app it
  came from, and group chats show who wrote what. Answer from the computer when the app lets
  its notification be answered. Nothing is kept on disk. Needs a phone that sends its
  notifications, which a coming version of Linksy for Android adds.
- **Verification codes copied:** a code that arrives in a message goes straight to the
  clipboard, and its notification says so and has a Copy button. Turn it off with Copy Codes
  Automatically in Preferences. Android 15 and newer hide some codes from apps like Linksy;
  those can't be copied.
- **In the terminal:** `linksy inbox` lists the conversations, `linksy inbox <id>` shows one,
  and `linksy reply <id> <text>` answers it.
- **Phone Notifications in the Qt app's Preferences:** show them, copy codes and unmute apps,
  as in the GTK app.

### Changed
- **Messages no longer lists the phone's SMS threads or writes new texts:** the phone app
  can't read texts, so they come from the SMS app's notifications, like every other
  messaging app.

## 0.11.3

### Improved
- **A lighter window:** with a phone's page open, the app settles at about 41 MB instead of
  44 to 53. The activity timeline builds its newest 20 entries and more as you scroll, so a
  device's page also opens faster, and memory the app is done with goes back to the system
  every minute (in the Plasma app too) (#26).

### New
- **Away From Home, fully off:** turning it off in Preferences, or with
  `linksy settings away off`, closes the connection to the relay at once. With Check for
  Updates off too, Linksy never reaches past your network.
- **For distributions:** `cargo build -p linksyd --no-default-features` builds the service
  without away from home at all, with no relay code.

## 0.11.2

### Improved
- **Lighter in the background:** Linksy is built for size now, with file transfers,
  encryption and checksums kept at full speed. The background service averages about 14 MB
  at rest instead of 18 (with away from home on), its program is 14 MB instead of 23, and
  transfers are as fast as before (#26).
- **The devices column can be dragged wider or narrower** in the Qt app, and keeps its width
  (with Kirigami 6.16 or newer; thanks pwnflakes).

## 0.11.1

### Fixed
- **Icons on KDE Plasma and other themes:** the GTK app brings the few icons that Breeze and
  some other icon themes don't have, so no button shows a broken image any more (#70, thanks
  chethan62).
- **The Qt app's "Use as" cards** come back after you close a camera preview (#72), and keep
  their text above the Start button after the window is resized (thanks pwnflakes).
- **Linksy's own password prompt** starts with the cursor in the password field, and asks for
  "the password for" an account (#67).

## 0.11.0

### New
- **Control a phone from the computer over the cable, with no developer options.** Take
  Control on a phone's screen while it's plugged in: the computer plugs a virtual
  touchscreen and keyboard into the phone (Android Open Accessory 2.0), so clicks land where
  you click, the wheel and touchpad scroll, Escape goes back, and keys type with the phone's
  own layout. Text goes through the phone's clipboard, so any language works. A plugged-in
  phone opens straight in control (turn it off once and that phone opens to watch), and
  its screen stays on while you use it.
- **The screen viewer fits a phone:** in a narrow window, Take Control and the other buttons
  sit on their own row under the phone's name.
- **Linksy Cable:** the USB cable checks itself and lets go of a stalled phone in seconds,
  ends each batch so the phone reads it at once, tries again when a phone doesn't switch in
  Android's 10 seconds, and recognises phones from more makers in charging-only mode (ZTE,
  Honor, Nokia and others; the permission rule is updated by the packages, Finish Setup
  elsewhere).

## 0.10.1

### Fixed
- **Linksy asks for the password itself where no password prompt runs** (Openbox, Hyprland,
  Sway and other desktops without a polkit agent), for Finish Setup and Update Now. It no
  longer starts another program's prompt in your session, which could take over the
  desktop on Openbox without showing up (#67). A wrong password can be typed again.

## 0.10.0

### New
- **The phone sees and controls X11 screens** (Openbox, XFCE, i3, MATE…), where no screen
  sharing portal works: Linksy reads the screen itself and moves the pointer and types with
  XTest. The phone as touchpad and keyboard works there too. Thanks to
  [@AndyM48](https://github.com/AndyM48) for asking (#54).
- **The phone controls the screen on Hyprland and Sway**, types into apps and works as a
  touchpad there, and a second display takes its taps: Linksy uses the compositor's
  virtual pointer and keyboard, since their portals can only share the screen (#11). Sway
  keeps these from sandboxed apps, so there it needs the packages, not the Flatpak.
- **The Flatpak does everything the packages do.** Finish Setup sets up the webcam driver,
  the USB cable and the firewall with one password, and turns on starting at login. The
  terminal for phones works, Update Now installs the new version, and Send with Linksy shows
  up in Dolphin, GNOME Files and Thunar. On Hyprland, Sway and X11 desktops, the screen
  locker, volume, Second Display and the clipboard work from the Flatpak too. It now asks
  for host access to do this.
- **Everything Linksy needs, in one page**, for installing it by hand: each feature, the
  packages per distribution, and the camera, cable and firewall steps without Finish Setup.
  Finish Setup links to it (#35).

### Fixed
- **Finish Setup no longer offers GNOME's screen sharing portal on X11 desktops**, where it
  can't help (#67). Second Display says it needs a Wayland session there.
- **Finish Setup stops waiting for a password after two minutes** and says what to do
  instead, and the password prompt it starts on desktops without one now stops when setup
  is done, or when Linksy is closed, rather than staying behind in the session (#67).
  Thanks to [@AndyM48](https://github.com/AndyM48) for the idea.
- **Clipboard text from a phone reaches X11 desktops that also have wl-clipboard
  installed**: the copy stopped at wl-copy before trying xclip and xsel.

## 0.9.17

### Security
- **A device's name can't hold markup anywhere.** `<` and `>` in names from other devices
  become `‹` and `›`, so the Qt app's pairing prompt and KDE's notifications show a
  stranger's name as text too, not only the labels fixed in 0.9.16 (#63).

### New
- **Send with Linksy in more file managers**: Thunar's Send To menu, and GNOME Files'
  right-click menu where nautilus-python is installed (Fedora installs it with Linksy).
  Thanks to [@AndyM48](https://github.com/AndyM48) (#62) and
  [@Lur1fa](https://github.com/Lur1fa) (#64) for asking.

### Changed
- **The microphone card shows what the phone hears** as a live wave, in both apps. It
  replaces Listen: no echo, no headphones needed to check the phone is picking you up.

### Fixed
- **The webcam's driver builds on Manjaro and other kernels with their own headers**, from
  the Flatpak too: Finish Setup installs the running kernel's headers (`linux73-headers`,
  say), not only `linux-headers`. Thanks to
  [@AwwsmGaurav](https://github.com/AwwsmGaurav) (releases#7).

## 0.9.16

### Security
- **Names from other devices show as plain text in the Qt app.** A device named like a
  web page could make Linksy fetch a picture from the internet, telling that site you're
  online, or style its name to look like something else. Thanks to
  [@bearyjd](https://github.com/bearyjd) (#63, #65).

### Fixed
- **"Linksy X Is Available" goes away after Update Now.** A window opened from the tray
  before 0.9.14 kept talking to the service from before the update, which still offered
  the version just installed. Thanks to [@AndyM48](https://github.com/AndyM48) (#66).

## 0.9.15

### Security
- **Linksy's private files are private.** The pairing key, your paired devices, the
  history and the file sharing token were readable by other accounts on the same
  computer. They're now yours only, and an existing install is fixed when it starts.
  Thanks to [@bearyjd](https://github.com/bearyjd) (#60, #61).
- **The terminal code's lockout holds** when many guesses arrive at once: each try counts
  before it's checked, so 5 wrong codes lock as intended. Thanks to
  [@bearyjd](https://github.com/bearyjd) (#58, #59).

### New
- **Send with Linksy in Dolphin's right-click menu**: pick a device and the files go.
  Thanks to [@sispus](https://github.com/sispus) for asking (#57).

### Fixed
- **Open With Linksy works on KDE Plasma**: the files reach the window instead of being
  dropped.
- **The camera driver builds for custom Fedora kernels**: Finish Setup installs that
  kernel's headers, and says what went wrong instead of "v4l2loopback isn't installed".

## 0.9.14

### Security
- **Device names can't disguise a pairing prompt.** A device on the network could send a
  name with line breaks, hidden right-to-left characters or thousands of letters, and push
  the code out of the pairing notification. Names are now cleaned as they arrive, and cut
  at 48 characters. Thanks to [@bearyjd](https://github.com/bearyjd) (#55, #56).

### New
- **View Screen and Send Clipboard in the tray menu**, in place of Ring, Ping and Locate.
  Send Clipboard works on GNOME too, with the window closed.

### Fixed
- **Show Linksy in the tray raises the open window** instead of opening a second one;
  the same for a second display arriving while Linksy is open.
- **Finish Setup installs what's missing on Artix** and other Arch-based systems that
  don't say so, instead of asking you to do it by hand.
- **The camera setup asks for mokutil** when Secure Boot needs the driver approved and
  mokutil isn't installed, instead of asking for a restart that couldn't help.

## 0.9.13

### Security
- **Safer pairing codes.** Someone on the same network at the moment you pair could have
  made both screens show the same 6-digit code. Devices now each lock in a random number
  before seeing the other's, so the codes can't be steered. Devices on older versions
  still pair as before, with a note suggesting an update. Devices already paired and QR
  code pairing were never affected.

### Fixed
- **Linksy Camera stays after a restart without systemd** (Artix, Void, Gentoo). Finish
  Setup had it come back at boot through a systemd service only; on other systems the
  driver now brings it as it loads.
- **Finish Setup helps tiling compositors without systemd too.** On Hyprland, Sway, niri
  or river without systemd, it now gives the line that starts Linksy at login, instead of
  saying nothing. Desktops keep starting it on their own.
- **Restart Now works without systemd**, through elogind, for the camera driver's approval
  after Secure Boot.

## 0.9.12

### Fixed
- **Taps on KDE's extended screen land where you tap, again.** 0.9.10's fix didn't find
  the second display on some computers, so with scaling on (125 %, 150 %) taps still landed
  lower right. Linksy now finds it by the name KDE gives it, follows its scale while it's
  shared, and says in Save Logs what it found.
- **The cursor shows at once after a tap on a still screen.** When nothing moved on the
  shared screen, the frame with the new cursor could arrive up to a second late. The last
  frame now goes again every 100 ms.
- **Linksy reconnects far sooner after a network switch.** When this computer's network changes,
  links that stopped answering close at once instead of after 30 seconds, and devices that
  stay connected are dialed again. A device not seen on the network is tried away from home
  right away rather than after its old addresses fail, and one that comes back away is no
  longer refused while its old link lingers. Phones get the same with the Linksy for
  Android update that reports network changes.

### Changed
- **The service uses about a quarter less memory at rest** (24 MB to 18 MB here): it gives
  back what it isn't using every few minutes.
- **Away from home waits for a device that can use it.** Until a paired device can reach
  this computer from away (none paired yet, or none met on the same network in the last
  week), the service doesn't run it, and takes 7 MB instead of 16.
- **Big transfers no longer swell the history.** A transfer of thousands of files kept
  every name in memory for good (about 4 MB for 20,000); older ones now keep their first
  file and how many there were.

## 0.9.11

### Added
- **Transfers, like on your phone.** A Transfers page shows the speed, with files running
  between the two devices at their real pace and how they go (same Wi-Fi, USB cable, away
  from home or Wi-Fi Direct), each file with its progress, and recent transfers with every
  device. Ctrl+T opens it.
- **Transfers in a corner.** While files go either way, a panel at the bottom right shows
  each transfer on every page, with Cancel; it folds to one bar and goes away a few seconds
  after the last one ends, with Open and Show in Folder until then.
- **The island.** At the bottom of your devices, Transfers, Send Files and Add Device sit
  in one pill, as on the phone: Send Files lists your devices, Add Device offers the QR
  code, an address or the USB cable.
- **Choose which screen a phone sees.** A phone can ask for the screen to be picked again
  (with the Linksy for Android update that adds Choose Screen), and Preferences › Screen
  Sharing › Choose Again does the same from here.

### Changed
- Transfer history keeps each file's size.

## 0.9.10

### Added
- **Always Allow, for screen and control.** A device of yours can see and control this
  computer's screen, or use its mouse and keyboard, without asking each time: choose Always
  Allow when it asks (in the notification or in the app), or turn on Always Allow Screen
  and Control on its page. Linksy still shows a notification when it starts, and Stop is in
  the tray. Away from home it always asks, and so do the camera, the microphone, a second
  display and terminals.

### Fixed
- **Taps on KDE's extended screen land where you tap.** With display scaling on (125 %,
  150 %), taps from the phone landed further right and down than meant, and near the
  right and bottom edges not at all.
- **Paired devices no longer stay "not nearby".** After a paired device changed its private
  name (every 15 minutes), this computer could miss it for up to an hour, until the service
  restarted. It now looks again a few seconds after each change, and right after a device
  goes.
- **The .deb and .rpm packages install one file at a time.** `linksy-common` and the app
  each needed the other, so gdebi, GNOME Software and Discover refused both. Open
  `linksy-common` first, then the app; with only `linksy-common`, Linksy in the menu opens
  the download page.
- **The Arch package builds with LTO turned on in makepkg.conf**, and from the release's
  git tag.

## 0.9.9

### Added
- **Wi-Fi Direct is here, and it's damn fast.** Big files go straight from your phone to this
  computer over a Wi-Fi link of their own, not through the router: **about 80 MB/s** in our
  tests (a realme 15 Pro and a ThinkPad E14), four times the 20 MB/s through the router, and
  at AirDrop's level. It sets itself up for files of 256 MB and up, in about 15 seconds,
  and the computer stays on its Wi-Fi network, internet included. If anything goes wrong,
  the transfer goes the usual way. The computer joins the phone's link through
  NetworkManager, with no root and no password; it needs a Wi-Fi card that does Wi-Fi
  Direct (most do) and the Linksy for Android update that brings it. `linksyd --no-direct`
  turns it off.
- **Steadier transfers on busy Wi-Fi.** Linksy's connection asks the system for much bigger
  network buffers, so bursts at Wi-Fi speed no longer overflow them and get dropped.

### Fixed
- **Sending a link to a file (a symlink) sends that file.** It used to send nothing; links
  inside a folder you send are still left out.

## 0.9.8

### Added
- **This computer's volume, ready for the phone.** A paired phone can show the computer's
  volume and mute, follow them as they change, and set them (through WirePlumber, or
  PulseAudio's pactl). The slider itself comes with a phone app update. Not in the Flatpak.

### Fixed
- **The tray menu's icons were full-colour pictures on GNOME** that looked like emoji next
  to the text. They're one colour now, like the rest of the top bar.

## 0.9.7

### Added
- **Linksy sits in the system tray** on KDE Plasma, GNOME (with the AppIndicator
  extension), other desktops and in the Flatpak. The icon fills in while a device is
  connected. Its menu has, for each paired device, what you can do with it (send files,
  ring or ping, locate or browse a phone, use it as a second display, webcam or
  microphone), a Stop for anything being shared, and Show Linksy. The tray belongs to the
  background service, so no window stays open for it. Menu and icons by yurividal.

### Changed
- **Cover art from browsers reaches the phone.** A cover too big to send (a video's
  thumbnail is often twice the size allowed) is now made smaller instead of left out.

### Fixed
- **The phone camera still failed after enrolling the Secure Boot key** (0.9.6's fix asked
  akmods to retry, which kept the unsigned driver). The driver is now really built again,
  signed with the enrolled key, and when it's still refused, the setup says why instead of
  asking for another restart.
- **A window the service opened shared its small memory limit,** the limit that made
  Second Display slow in 0.9.4. It now has its own.

## 0.9.6

### Fixed
- **Second Display didn't start on KDE.** KDE makes its virtual monitor 1920×1080, or the
  resolution picked in its sharing dialog, not in the phone's shape, and Linksy wouldn't
  take any other size. It now uses KDE's size, and the phone fits it to its screen.
- **The phone's pointer stopped at the right and bottom edge on KDE** ("Invalid position"
  in the log).
- **Shift could stay held** after using the phone as a keyboard: keys still down when a
  remote input session ends are now released.
- **The phone camera kept asking for a restart after enrolling the Secure Boot key** when
  the driver had been signed with another key: it's now built again, signed with the
  enrolled one.

## 0.9.5

### Changed
- **Second Display at up to 60 frames a second with a GPU encoder.** The screen now goes
  from the desktop to the graphics chip in graphics memory, which converts and encodes it
  without the processor: on a Ryzen 5 laptop, every frame GNOME draws (about 35 a second
  while moving windows) instead of 18. Where the desktop can't hand over graphics memory,
  frames are uploaded (about 30 a second); without a GPU encoder, the processor encodes
  them as before.
- **Finish Setup suggests RPM Fusion's full graphics driver** on Fedora with AMD graphics,
  whose own driver leaves out video encoding.

### Fixed
- **Screen sharing that failed to start ended there.** Linksy now tries the next way to
  encode (the GPU from graphics memory, then uploaded, then x264, then OpenH264) until one
  works, and Save Logs says which part refused and why.
- Finish Setup's notification daemon (mako) couldn't be installed by the setup helper.

## 0.9.4

### Fixed
- **Second Display lagged when Linksy ran as a service.** The service's memory limit
  (64 MB) also covered the screen helper, which needs 150 to 250 MB for a phone-shaped
  display, so the system kept taking memory back from it and it fell to a few frames a
  second. Screen helpers now get a limit of their own (512 MB, released when the session
  ends), and the service keeps its small one.
- **A second display froze after changing GNOME's display settings.** It now starts again
  by itself once the settings are kept.
- **Screen helpers could outlive a crashed service.** They now end with it.

### Known issue
- Changing GNOME's display settings while a second display is on can revert them. Change
  them with the second display off.

## 0.9.3

### Fixed
- **Second Display was still slow on a laptop without a hardware encoder.** With windows
  moving on it, the encoder's finer motion search cost ten times more than expected and
  the display fell to a few frames a second on battery. It now uses the fastest setting at
  twice the bitrate: 30 frames a second, and sharp.

## 0.9.2

### Fixed
- **Second Display lagged on computers without a hardware video encoder** (Fedora's AMD
  drivers, for one). A phone-shaped display is about 2.5 million pixels, and encoding it
  on one processor core took longer than a frame lasts, so the delay kept growing. It now
  runs at up to 30 frames a second on several cores and keeps up.
- **The Flatpak's Save Logs had no log in it.** The Flatpak's service now writes its log
  to a file of its own, and Save Logs includes it.

## 0.9.1

### Changed
- **What's New covers every version you missed.** Coming from an older Linksy, it lists
  each version's highlights since the one you last used, under its own heading, instead
  of only the newest version's.

### Fixed
- **Update Now closed Linksy without starting it again.** The update replaces the program
  while it runs, and Linksy tried to start the replaced one. It now starts the new one
  once the old window has closed. (Updating from 0.9.0 still needs opening Linksy by hand
  once: the old version does the restarting.)

## 0.9.0

### Added
- **Ubuntu 24.04 and Linux Mint 22.** The .deb now installs on Ubuntu 24.04 LTS and
  everything built on it (Linux Mint 22, Pop!_OS, Zorin, elementary): the GTK app needs
  libadwaita 1.5, and where GStreamer's GTK 4 video sink has no package, Linksy draws
  screens and camera previews itself. The KDE app still needs Plasma 6 (Qt 6.8).
- **Hyprland, Sway, niri and X11 desktops.**
  - **Second Display on Hyprland and Sway:** Linksy makes a real output in the phone's
    shape, shares it, and removes it afterwards.
  - **Lock When Away:** runs the installed screen locker (hyprlock, swaylock,
    xflock4, i3lock and others).
  - **Finish Setup:** starts an installed password prompt (polkit agent) when none runs,
    suggests a notification daemon where none runs, gives the line that starts Linksy at
    login on Hyprland, Sway, niri and river, and suggests the right screen-sharing portal
    on niri.
  - **Clipboard:** copying on an X11 desktop reaches phones that watch the clipboard.
  - **Autostart:** the service starts at login on desktops without systemd's graphical
    session (XFCE, MATE, Cinnamon, systems without systemd).
- **Private discovery.** On a café's or an office's Wi-Fi, Linksy no longer announces the
  computer's name and a permanent id to everyone: only a code that changes every 15
  minutes, which only your paired devices recognise. The name shows while Add Device is
  open. Forgetting a device changes the key, so it can't recognise this computer anymore.
- **One Accept for a batch.** Files sent together ask once, and **Always Accept** (on the
  prompt, or Always Accept Files on a device's page) saves a device's files without asking.
- **Problem logs.** A crash is written down, and `logs.txt` (versions, crashes and the
  recent log, without your home folder or user name) is one click away: About >
  Troubleshooting in the GTK app, Save Logs… in the KDE app's menu, offered once after a
  crash, and `linksy logs` in a terminal. Nothing is sent anywhere.
- **The phone's own Connect by USB Cable.** The computer tells a connected phone when it's
  plugged in, and switches it when the phone asks (with the phone app's next version).

## 0.8.7

### Changed
- **Plugging a phone in no longer asks anything.** Most of the time it's only charging, and
  switching it to Linksy stops its file transfer by cable. Connect by USB Cable (on the
  phone's page, or in the menu) uses the cable when you want it, and **Use the Cable
  Automatically** in that guide switches a phone on its own whenever it's plugged in, as
  before. It starts off for every phone.

- **The Flatpak's webcam in every app and USB cable take one command.** The sandbox can't
  install the camera driver or the USB rule itself; Finish Setup and the USB guide now
  give the exact command to paste in a terminal (Linksy's own setup helper for this
  version), instead of saying it can't be done.

### Fixed
- **Second Display on Hyprland and Sway failed halfway.** Their screen sharing can't make
  a virtual monitor yet; Linksy now says so before starting, in the app and in a
  notification, instead of failing. Support for them is planned for 0.9.

## 0.8.6

### Added
- **Upgrades in one step.** `linksy upgrade` checks for a newer Linksy and, when there is
  one, downloads exactly the packages installed here (`linksy-common` and `linksy` and/or
  `linksy-qt`), checks them against the release's checksums, installs them with one
  password and restarts the service. In both apps, "Linksy X Is Available" (and Check for
  Updates… in the menu) does the same with **Update Now**, then Linksy starts again on the
  new version. The tarball updates into your home folder with no password; the Flatpak and
  builds by hand say how to update them instead. Upgrading from this version on.
- **`linksy --version`** (and the running service's version when it differs) and
  **`linksy --revision`**: the exact build and how Linksy was installed, for bug reports.

## 0.8.5

### Added
- **What's New.** The first time a new version opens after an update, both apps show what
  it brings, a few highlights with a link to the full notes, and a way to star Linksy on
  GitHub. It's also in the menu any time. A fresh install shows nothing.

### Fixed
- **Installing only `linksy-common` gave a Linksy in the menu that opened nothing**, and
  a store showing Linksy would have installed just that. `linksy-common` now always comes
  with an app: `linksy` (GTK), or `linksy-qt` when that's the one you install with it.
- **Accept and Decline in a notification did the job but left the notification on
  screen**, and answering in the app left it there too. The prompt now goes away as soon as
  it's answered, wherever.

## 0.8.4

### Added
- **A Linksy for KDE Plasma.** `linksy-qt` is the same Linksy as a Qt 6 and Kirigami app, in
  Breeze, made for Plasma 6: your devices, pairing, sending files, the clipboard, Ring,
  Locate, another device's screen (watch or take control), the phone as second display,
  webcam (with a preview and the phone's camera controls) or microphone, this computer as
  another one's second display, Connect by USB Cable, Finish Setup, update notices and the
  preferences.
- **Packages split the way Transmission's are:** `linksy-common` (the service and tools)
  with `linksy` (the GTK app) or `linksy-qt` (the Qt app). Install linksy-common and the app
  for your desktop; when both apps are installed, Linksy opens the Qt one on Plasma. Upgrades
  from 0.8.3 move over by themselves.
- **Your own relay.** Preferences › Connection › Relay Server takes the address of a
  relay you host (`linksy-server`), as `https://relay.example.com`; blank means Linksy's
  own. It wins over the relay the website lists and takes effect at once: the away
  endpoint starts again on it. Also `linksy settings relay <url>|default` and, for a
  whole machine, `LINKSY_RELAY` on the service. Thanks to yurividal.

### Fixed
- **The phone's camera on CachyOS and other Arch-based kernels.** Finish Setup installed
  the stock `linux-headers` whatever the kernel; it now reads which kernel package is
  running and installs its own headers (`linux-cachyos-headers`…). And where the kernel
  already has the camera driver but not its `v4l2loopback-ctl` tool, as CachyOS's does,
  Finish Setup installs the tool first instead of stopping at "command not found".
- **A second display started from another computer opened nothing when only the Qt app
  was installed**; the service now opens whichever app the desktop uses.
- **Finish Setup on Arch found its setup helper only where Fedora and Debian keep it**;
  it now looks in `/usr/lib/linksy` too, where Arch packages put it.
- **The app store listing's screenshots** come from the public repository, where they are.

## 0.8.3

### Added
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
- **Controlling the computer from the phone failed at once, every time,** once the
  desktop's remembered screen choice went stale (with "no screen was chosen" in the log).
  Linksy now forgets it and asks again.
- **Screen sharing never started on Intel Kaby Lake GPUs**, whose low-power H.264 encoder
  can't keep a constant bitrate. Linksy now checks first and uses the next encoder.
- **On openSUSE Aeon, Kalpa and MicroOS, Finish Setup tried to install with zypper**, which
  fails on their read-only system. It now gives the transactional-update command to run,
  then a restart.

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
