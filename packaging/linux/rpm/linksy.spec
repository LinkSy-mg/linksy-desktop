# Packs the binaries scripts/package-linux.sh already built: no build step here.
# One spec, three packages, as Transmission splits its apps: linksy-common (the service and
# tools), linksy (the GTK app) and linksy-qt (the Qt app for KDE Plasma), each app needing
# the common one. Built twice: Fedora's dependency names by default (release 1), openSUSE's
# with --define "linksy_suse 1" (release 1.suse).
%global debug_package %{nil}
%global source_date_epoch_from_changelog 0

Name:           linksy
Version:        %{version}
Summary:        Your devices, connected
License:        AGPL-3.0-or-later
URL:            https://github.com/LinkSy-mg/linksy-releases
ExclusiveArch:  x86_64
%if 0%{?linksy_suse}
Release:        1.suse
%else
Release:        1
%endif
Requires:       linksy-common = %{version}-%{release}
%if 0%{?linksy_suse}
# gtk4paintablesink lives in the Rust plugins bundle on openSUSE
Requires:       gstreamer-plugins-rs
%else
Requires:       gstreamer1-plugin-gtk4
%endif

%description
Linksy connects your computer and phone directly over your own network, with no
account, cloud or server: send files and your clipboard, see and control another
device's screen, use a tablet as a second display, or a phone as a webcam.

This is the GTK app, for GNOME and most desktops; linksy-qt is the one for KDE Plasma.

%package common
Summary:        Your devices, connected (service and tools)
# Before 0.8.4, one linksy package held everything
Conflicts:      linksy < 0.8.4
# Comes with a window: the GTK app, or the Qt app when that's the one asked for. Only
# recommended, so installers that take one file at a time can take this one first.
Recommends:     (linksy = %{version}-%{release} or linksy-qt = %{version}-%{release})
%if 0%{?linksy_suse}
Requires:       gstreamer-plugins-good
Requires:       gstreamer-plugin-pipewire
Requires:       xdg-desktop-portal
Recommends:     gstreamer-plugin-openh264
Recommends:     gstreamer-plugins-libav
Recommends:     gstreamer-plugins-bad
Recommends:     wl-clipboard
# The camera driver (v4l2loopback-kmp-<flavor>) is left to Finish Setup: it picks the
# running kernel's flavor, where a fixed Recommends could pull in another kernel.
%else
Requires:       gstreamer1-plugins-good
Requires:       pipewire-gstreamer
Requires:       xdg-desktop-portal
Recommends:     gstreamer1-plugin-openh264
Recommends:     gstreamer1-plugins-bad-free
Recommends:     wl-clipboard
# The phone's camera in every app (RPM Fusion); Linksy's Finish Setup offers it too
Recommends:     v4l2loopback
# Send with Linksy in Files' right-click menu
Recommends:     (nautilus-python if nautilus)
%endif

%description common
What both Linksy apps share: the background service that finds, pairs and talks to your
devices, the screen and camera helper, the linksy command and the setup helpers.
It comes with linksy (GNOME and others) or linksy-qt (KDE Plasma), the window.

%package qt
Summary:        Your devices, connected (KDE Plasma app)
Requires:       linksy-common = %{version}-%{release}
%if 0%{?linksy_suse}
Requires:       libQt6Quick6
Requires:       qt6-declarative-imports
Requires:       kf6-kirigami-imports
Requires:       kirigami-addons6
Requires:       kf6-qqc2-desktop-style
Requires:       gstreamer-plugins-good-qtqml6
%else
Requires:       qt6-qtbase
Requires:       qt6-qtdeclarative
Requires:       kf6-kirigami
Requires:       kf6-kirigami-addons
Requires:       kf6-qqc2-desktop-style
Requires:       gstreamer1-plugins-good-qt6
%endif

%description qt
Linksy connects your computer and phone directly over your own network, with no
account, cloud or server: send files and your clipboard, see and control another
device's screen, use a tablet as a second display, or a phone as a webcam.

This is the Qt and Kirigami app, made for KDE Plasma 6.

%install
cp -a %{root}/. %{buildroot}/

%post common
# Start the background service with every login (it starts now too, when Linksy opens).
if [ $1 -eq 1 ]; then systemctl --global enable linksyd.service >/dev/null 2>&1 || :; fi
# Phones by USB cable: apply the udev rule to what's plugged in now.
udevadm control --reload-rules >/dev/null 2>&1 || :
udevadm trigger --subsystem-match=usb --action=change >/dev/null 2>&1 || :
# Let nearby devices find this computer (firewalld or ufw, when one is on).
if [ $1 -eq 1 ]; then /usr/libexec/linksy/linksy-system-setup firewall >/dev/null 2>&1 || :; fi

%preun common
if [ $1 -eq 0 ]; then
  systemctl --global disable linksyd.service >/dev/null 2>&1 || :
  # The camera device Finish Setup made, if it did
  systemctl disable linksy-camera.service >/dev/null 2>&1 || :
  rm -f /etc/systemd/system/linksy-camera.service /etc/modules-load.d/linksy-camera.conf /etc/modprobe.d/linksy-camera.conf
fi

%files
/usr/bin/linksy-app

%files common
%license /usr/share/licenses/linksy/LICENSE
%license /usr/share/licenses/linksy/THIRD-PARTY-NOTICES
/usr/bin/linksyd
/usr/bin/linksy
/usr/bin/linksy-open
/usr/bin/linksy-screen
/usr/libexec/linksy/linksy-camera-setup
/usr/libexec/linksy/linksy-usb-setup
/usr/libexec/linksy/linksy-system-setup
/usr/lib/firewalld/services/linksy.xml
/usr/lib/udev/rules.d/70-linksy-usb.rules
/usr/lib/systemd/user/linksyd.service
%config(noreplace) /etc/xdg/autostart/linksyd.desktop
/usr/share/applications/mg.diary.linksy.Linksy.desktop
/usr/share/metainfo/mg.diary.linksy.Linksy.metainfo.xml
/usr/share/kio/servicemenus/linksy-send.desktop
/usr/share/Thunar/sendto/linksy.desktop
/usr/share/nautilus-python/extensions/linksy.py
/usr/share/icons/hicolor/scalable/apps/mg.diary.linksy.Linksy.svg
/usr/share/icons/hicolor/symbolic/apps/mg.diary.linksy.Linksy-symbolic.svg
/usr/share/icons/hicolor/symbolic/apps/mg.diary.linksy.Linksy-tray-symbolic.svg
/usr/share/icons/hicolor/symbolic/apps/mg.diary.linksy.Linksy-tray-idle-symbolic.svg

%files qt
/usr/bin/linksy-qt
