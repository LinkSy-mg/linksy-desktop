# What Linksy needs

Everything Linksy relies on, for installing it by hand: when Finish Setup can't run, a
package was removed, or your distribution isn't one Linksy knows. The .deb and .rpm
packages bring most of it; Finish Setup adds the rest. This page follows Finish Setup's
checks (`crates/linksy-setup/src/lib.rs`), so the two say the same thing.

The Flatpak brings its own GStreamer, codecs and clipboard tool. On the computer itself it
still needs the camera driver, the USB rule and the firewall below, which its Finish Setup
adds.

## Each feature and what it needs

| Feature | Needs |
|---|---|
| Pairing, files, notifications from the phone, battery, Ring, media controls, the terminal, opening links | Nothing beyond the service and an app. |
| The app | GTK 4.14+ and libadwaita 1.5+ (the GTK app), or Qt 6.8+ with Kirigami, Kirigami Addons and qqc2-desktop-style (the Qt app, for KDE Plasma). glibc 2.39+. |
| Seeing screens and cameras | GStreamer with an H.264 decoder (`avdec_h264` from libav, `openh264dec`, or `vah264dec` on the graphics card), and the window's video sink: `gtk4paintablesink` (GTK) or `qml6glsink` (Qt). |
| Sending this computer's screen, a second display | An H.264 encoder (`vah264lpenc` or `vah264enc` on the graphics card, `x264enc`, or `openh264enc`), PipeWire's GStreamer plugin (`pipewiresrc`), and the desktop's screen sharing portal on Wayland. On X11, `ximagesrc` (gst-plugins-good) instead of a portal. |
| A second display | GNOME or KDE Plasma (a virtual monitor from the portal), Hyprland or Sway (a headless output, through `hyprctl` or `swaymsg`). Not on X11 or niri. |
| Controlling this computer, the phone as a touchpad | GNOME or KDE Plasma (the RemoteDesktop portal), or any X11 desktop (XTest, part of the X server). |
| The phone as a webcam in OBS and PipeWire apps | PipeWire, with its GStreamer plugin (`pipewiresink`). |
| The phone as a webcam in browsers, Zoom, Discord | The v4l2loopback driver and its `v4l2loopback-ctl` tool, then the "Linksy Camera" device (below). With Secure Boot on, `mokutil` to approve the driver. |
| The phone as a microphone | PipeWire, with its GStreamer plugin. Apps see "Linksy Microphone (the phone's name)". |
| Text from the phone on the clipboard while Linksy's window is closed | `wl-copy` (wl-clipboard) on Wayland; `xclip` or `xsel` on X11. |
| Copies on the computer reaching the phone by themselves | `wl-paste` (wl-clipboard) on KDE, Hyprland, Sway; `xclip` or `xsel` on X11. GNOME doesn't let apps watch the clipboard. |
| Phones by USB cable | Linksy's udev rule (below). The .deb and .rpm install it. |
| Found by nearby devices | UDP 47200 and mDNS (UDP 5353) open in the firewall, if one runs (below). |
| Notifications, with their buttons | A notification daemon. Most desktops have one; on tiling compositors install mako, dunst or swaync. |
| Finish Setup's password | pkexec (polkit). The desktop's polkit agent asks for the password; where none runs, Linksy asks itself (0.10.1). |
| Ready when you log in | systemd's user service (`linksyd.service`), or the XDG autostart entry on desktops without it. On tiling compositors, a line in their config (Finish Setup shows it). |
| Lock When Away | systemd-logind or elogind, or a screen locker Linksy knows (hyprlock, swaylock, gtklock, waylock, i3lock, xflock4, and others). |
| Volume from the phone | `wpctl` (PipeWire) or `pactl`. |
| The phone in the Files sidebar | GVfs with its WebDAV backend, and a file manager that uses GVfs (GNOME Files, Thunar, Nemo). |
| Send with Linksy in GNOME Files | Nautilus's Python extensions. Dolphin and Thunar need nothing. |
| Big transfers over Wi-Fi Direct | NetworkManager, and a Wi-Fi card that does P2P (`iw list` shows `P2P-client`). |

## Packages, by distribution

The packages Finish Setup installs, as one command each. Leave out what you don't need.

**Fedora** (the camera driver comes from RPM Fusion free):

```sh
sudo dnf install "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm"
sudo dnf install gstreamer1-plugins-good pipewire-gstreamer xdg-desktop-portal \
  gstreamer1-plugin-openh264 gstreamer1-plugin-libav gstreamer1-plugins-bad-free \
  gstreamer1-plugin-gtk4 wl-clipboard akmod-v4l2loopback v4l2loopback
```

The Qt app: `gstreamer1-plugins-good-qt6` in place of `gstreamer1-plugin-gtk4`. With an AMD
graphics card, RPM Fusion's `mesa-va-drivers-freeworld` lets it encode screens (up to 60
frames a second instead of 30).

**Ubuntu and Debian:**

```sh
sudo apt install gstreamer1.0-plugins-good gstreamer1.0-pipewire xdg-desktop-portal \
  gstreamer1.0-libav gstreamer1.0-plugins-ugly gstreamer1.0-plugins-bad \
  gstreamer1.0-gtk4 wl-clipboard v4l2loopback-dkms v4l2loopback-utils "linux-headers-$(uname -r)"
```

The Qt app: `gstreamer1.0-qt6` in place of `gstreamer1.0-gtk4`. Ubuntu 24.04 has no
`gstreamer1.0-gtk4`; the GTK app draws the video itself there.

**Arch and its derivatives:**

```sh
sudo pacman -S --needed gst-plugins-good gst-plugin-pipewire xdg-desktop-portal \
  gst-libav gst-plugins-ugly gst-plugins-bad gst-plugin-gtk4 wl-clipboard \
  v4l2loopback-dkms v4l2loopback-utils "$(cat /usr/lib/modules/$(uname -r)/pkgbase)-headers"
```

The last one is your kernel's headers (`linux-headers`, `linux-lts-headers`,
`linux-cachyos-headers`...). The Qt app: `gst-plugin-qml6` in place of `gst-plugin-gtk4`.

**openSUSE Tumbleweed:**

```sh
sudo zypper install gstreamer-plugins-good gstreamer-plugin-pipewire xdg-desktop-portal \
  gstreamer-plugin-openh264 gstreamer-plugins-libav gstreamer-plugins-bad \
  gstreamer-plugins-rs wl-clipboard "v4l2loopback-kmp-$(uname -r | sed 's/.*-//')" v4l2loopback-utils
```

`gstreamer-plugins-rs` has the GTK app's video sink; the Qt app needs
`gstreamer-plugins-good-qtqml6` instead. The camera driver comes prebuilt for your kernel's
flavor (`default`, `longterm`), with nothing to build or approve.

**The screen sharing portal**, on Wayland, for your desktop: `xdg-desktop-portal-gnome`
(GNOME, niri), `xdg-desktop-portal-kde` (KDE Plasma), `xdg-desktop-portal-hyprland`
(Hyprland), `xdg-desktop-portal-wlr` (Sway, river, Wayfire, labwc). Desktops install theirs
already. Don't add GNOME's on an X11 desktop: Linksy doesn't use it there.

**Send with Linksy in GNOME Files:** `nautilus-python` (Fedora), `python3-nautilus`
(Ubuntu, Debian), `python-nautilus` (Arch).

### Fedora Atomic and openSUSE Aeon, Kalpa, MicroOS

Packages go into the system image, then a restart:

```sh
sudo rpm-ostree install PACKAGES && systemctl reboot                                 # Fedora Atomic
sudo transactional-update --non-interactive pkg install PACKAGES && systemctl reboot  # openSUSE
```

## Setting up by hand

Finish Setup runs its root helper, `linksy-system-setup`, with your password. You can run
it yourself (in `/usr/libexec/linksy/`, or `/usr/lib/linksy/` on Arch):

```sh
sudo /usr/libexec/linksy/linksy-system-setup camera usb firewall
```

Or each step without it:

**The Linksy Camera**, once the driver is installed:

```sh
sudo modprobe v4l2loopback devices=0
sudo v4l2loopback-ctl add --name "Linksy Camera" --exclusive-caps 1 --max-width 1920 --max-height 1920
```

That lasts until the next restart. For every boot, add to
`/etc/modules-load.d/linksy-camera.conf`:

```
v4l2loopback
```

and to `/etc/modprobe.d/linksy-camera.conf`:

```
options v4l2loopback devices=1 card_label="Linksy Camera" exclusive_caps=1 max_width=1920 max_height=1920
```

With Secure Boot on, a driver built on your computer waits for its key to be approved once:
`sudo mokutil --import` with the key (`/etc/pki/akmods/certs/public_key.der` on Fedora,
`/var/lib/shim-signed/mok/MOK.der` on Ubuntu, `/var/lib/dkms/mok.pub` elsewhere), then
Enroll MOK in the blue screen at the next restart.

**USB cable:** `/etc/udev/rules.d/70-linksy-usb.rules`, with:

```
ACTION=="remove", GOTO="linksy_usb_end"
SUBSYSTEM!="usb", GOTO="linksy_usb_end"
ENV{DEVTYPE}!="usb_device", GOTO="linksy_usb_end"
ATTR{idVendor}=="18d1", ATTR{idProduct}=="2d0[0-5]", TAG+="uaccess", GOTO="linksy_usb_end"
IMPORT{builtin}="usb_id"
ENV{ID_USB_INTERFACES}=="*:060101:*", TAG+="uaccess"
ENV{ID_USB_INTERFACES}=="*:ff4201:*", TAG+="uaccess"
LABEL="linksy_usb_end"
```

then `sudo udevadm control --reload-rules`, and plug the phone in again.

**Firewall**, with firewalld:

```sh
sudo firewall-cmd --permanent --add-port=47200/udp
sudo firewall-cmd --permanent --add-service=mdns
sudo firewall-cmd --reload
```

With ufw: `sudo ufw allow 47200/udp && sudo ufw allow 5353/udp`.

**Starting at login:** `systemctl --user enable --now linksyd.service`. Without systemd,
desktops read `/etc/xdg/autostart/linksyd.desktop`; on Hyprland add
`exec-once = linksyd` to `hyprland.conf`, on Sway `exec linksyd`, on niri
`spawn-at-startup "linksyd"`.

## How to check

```sh
gst-inspect-1.0 avdec_h264 openh264dec vah264dec      # a decoder: at least one found
gst-inspect-1.0 vah264enc x264enc openh264enc         # an encoder: at least one found
gst-inspect-1.0 pipewiresrc gtk4paintablesink         # qml6glsink for the Qt app; ximagesrc on X11
cat /sys/class/video4linux/*/name                     # shows Linksy Camera
ls /etc/udev/rules.d/70-linksy-usb.rules /usr/lib/udev/rules.d/70-linksy-usb.rules
sudo firewall-cmd --list-all                          # 47200/udp in ports, mdns in services
sudo ufw status                                       # or: 47200/udp and 5353/udp allowed
notify-send Linksy test                               # a notification appears
pkexec true                                           # a password window appears
systemctl --user is-enabled linksyd.service           # enabled
```
