Linksy for Linux
================

Share files, clipboard text, pings, screens and cameras between your own devices,
directly over your local network. No account, cloud or server.

Install (no root needed, everything goes in your home folder):

    ./install.sh

Then open Linksy from your apps. Remove it with ./uninstall.sh (your device identity and
paired devices are kept unless you add --purge).

Requirements
  - x86_64, glibc 2.39 or newer (Fedora 41+, Ubuntu 24.10+, Debian 13+)
  - GTK 4.14+ and libadwaita 1.6+
  - For screens and cameras: GStreamer 1.22+ with pipewire-gstreamer, the GTK 4 sink
    (gstreamer1-plugin-gtk4) and an H.264 codec (openh264, x264 or libav)

Firewall: allow UDP 47200 (Linksy) and mDNS (UDP 5353). On Fedora:
    sudo firewall-cmd --permanent --add-port=47200/udp --add-service=mdns
    sudo firewall-cmd --reload

More: https://github.com/LinkSy-mg/linksy-releases
