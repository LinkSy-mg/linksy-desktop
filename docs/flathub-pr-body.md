### App submission

- App: Linksy (`mg.diary.linksy.Linksy`), https://linksy.diary.mg
- Source: https://github.com/LinkSy-mg/linksy-desktop (AGPL-3.0-or-later), built from the release tag with the commit pinned
- Builds offline from `cargo-sources.json`; x86_64 only (`flathub.json`), which is what upstream CI tests

Linksy connects a computer and a phone directly over the local network: files, clipboard, screen sharing, a phone as webcam/touchpad, with no account or cloud.

### Permissions

Each is commented in the manifest. The less usual ones:

- `--device=all`: a phone's camera is used as a webcam through V4L2, which has no portal yet
- `--filesystem=xdg-run/pipewire-0`: publishes the phone's camera as a PipeWire camera
- `--filesystem=xdg-download`, `xdg-pictures`: received files and imported phone photos
- `--talk-name=org.mpris.MediaPlayer2.*` / `--own-name=org.mpris.MediaPlayer2.Linksy.*`: media controls both ways
- `--system-talk-name=org.freedesktop.NetworkManager`: joining a phone's Wi-Fi Direct link for big transfers

### Checklist

- [x] `appstreamcli validate --pedantic` passes on the metainfo
- [x] Flatpak built in `ghcr.io/flathub-infra/flatpak-github-actions:gnome-51` by upstream CI on every release
- [x] The app ID's domain is `linksy.diary.mg` (verification file: see docs/flathub.md upstream)
- [x] Licenses installed to `/app/share/licenses`
