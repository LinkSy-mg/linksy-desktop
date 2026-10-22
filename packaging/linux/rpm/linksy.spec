# Packs the binaries scripts/package-linux.sh already built: no build step here.
%global debug_package %{nil}
%global source_date_epoch_from_changelog 0

Name:           linksy
Version:        %{version}
Release:        1
Summary:        Your devices, connected
License:        AGPL-3.0-or-later
URL:            https://github.com/LinkSy-mg/linksy-releases
ExclusiveArch:  x86_64
Requires:       gstreamer1-plugins-good
Requires:       gstreamer1-plugin-gtk4
Requires:       pipewire-gstreamer
Requires:       xdg-desktop-portal
Recommends:     gstreamer1-plugin-openh264
Recommends:     gstreamer1-plugins-bad-free
Recommends:     wl-clipboard
# The phone's camera in every app (RPM Fusion); Linksy's Finish Setup offers it too
Recommends:     v4l2loopback

%description
Linksy connects your computer and phone directly over your own network, with no
account, cloud or server: send files and your clipboard, see and control another
device's screen, use a tablet as a second display, or a phone as a webcam.

%install
cp -a %{root}/. %{buildroot}/

%post
# Start the background service with every login (it starts now too, when Linksy opens).
if [ $1 -eq 1 ]; then systemctl --global enable linksyd.service >/dev/null 2>&1 || :; fi
# Phones by USB cable: apply the udev rule to what's plugged in now.
udevadm control --reload-rules >/dev/null 2>&1 || :
udevadm trigger --subsystem-match=usb --action=change >/dev/null 2>&1 || :
# Let nearby devices find this computer (firewalld or ufw, when one is on).
if [ $1 -eq 1 ]; then /usr/libexec/linksy/linksy-system-setup firewall >/dev/null 2>&1 || :; fi

%preun
if [ $1 -eq 0 ]; then
  systemctl --global disable linksyd.service >/dev/null 2>&1 || :
  # The camera device Finish Setup made, if it did
  systemctl disable linksy-camera.service >/dev/null 2>&1 || :
  rm -f /etc/systemd/system/linksy-camera.service
fi

%files
%license /usr/share/licenses/linksy/LICENSE
%license /usr/share/licenses/linksy/THIRD-PARTY-NOTICES
/usr/bin/linksyd
/usr/bin/linksy
/usr/bin/linksy-app
/usr/bin/linksy-screen
/usr/libexec/linksy/linksy-camera-setup
/usr/libexec/linksy/linksy-usb-setup
/usr/libexec/linksy/linksy-system-setup
/usr/lib/firewalld/services/linksy.xml
/usr/lib/udev/rules.d/70-linksy-usb.rules
/usr/lib/systemd/user/linksyd.service
/usr/share/applications/mg.diary.linksy.Linksy.desktop
/usr/share/metainfo/mg.diary.linksy.Linksy.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/mg.diary.linksy.Linksy.svg
/usr/share/icons/hicolor/symbolic/apps/mg.diary.linksy.Linksy-symbolic.svg
