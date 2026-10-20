# Packs the binaries scripts/package-linux.sh already built: no build step here.
%global debug_package %{nil}
%global source_date_epoch_from_changelog 0

Name:           linksy
Version:        %{version}
Release:        1
Summary:        Your devices, connected
License:        GPL-3.0-or-later
URL:            https://github.com/LinkSy-mg/linksy-releases
ExclusiveArch:  x86_64
Requires:       gstreamer1-plugins-good
Requires:       pipewire-gstreamer
Requires:       xdg-desktop-portal
Recommends:     gstreamer1-plugin-openh264
Recommends:     gstreamer1-plugins-bad-free
Recommends:     wl-clipboard

%description
Linksy connects your computer and phone directly over your own network, with no
account, cloud or server: send files and your clipboard, see and control another
device's screen, use a tablet as a second display, or a phone as a webcam.

%install
cp -a %{root}/. %{buildroot}/

%post
# Start the background service with every login (it starts now too, when Linksy opens).
if [ $1 -eq 1 ]; then systemctl --global enable linksyd.service >/dev/null 2>&1 || :; fi

%preun
if [ $1 -eq 0 ]; then systemctl --global disable linksyd.service >/dev/null 2>&1 || :; fi

%files
%license /usr/share/licenses/linksy/LICENSE
/usr/bin/linksyd
/usr/bin/linksy
/usr/bin/linksy-app
/usr/bin/linksy-screen
/usr/lib/systemd/user/linksyd.service
/usr/share/applications/app.linksy.Linksy.desktop
/usr/share/metainfo/app.linksy.Linksy.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/app.linksy.Linksy.svg
/usr/share/icons/hicolor/symbolic/apps/app.linksy.Linksy-symbolic.svg
