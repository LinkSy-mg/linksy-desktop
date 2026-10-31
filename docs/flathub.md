# Putting Linksy on Flathub

Everything Flathub needs is already in the repo: the manifest
(`packaging/flatpak/mg.diary.linksy.Linksy.yml`), the offline Cargo sources, the metainfo
(valid under `appstreamcli validate --pedantic`), the desktop file, the icons, the screenshots
and the licenses. CI already builds the Flatpak in Flathub's own image on every release.

Two things are left, and only the project owner can do them.

## 1. Prove that Linksy owns `mg.diary.linksy`

Flathub shows a "verified" badge only for an app ID whose domain serves a file naming it.
Serve this at `https://linksy.diary.mg/.well-known/org.flathub.VerifiedApps.txt` (the ID
`mg.diary.linksy.Linksy` is the reverse of `linksy.diary.mg`):

```
mg.diary.linksy.Linksy
```

Without it the app is still accepted, just unverified. Flathub's login check can be done after
the listing exists.

## 2. Submit

```sh
git fetch --tags
scripts/flathub-submit.sh --pr        # for the version in Cargo.toml
```

With no `--pr` it only builds the submission into `target/flathub/vX.Y.Z` so you can look at
it. With `--pr` it forks `flathub/flathub`, pushes the manifest, `cargo-sources.json` and
`flathub.json` to a branch, and opens the pull request against `new-pr` with the text in
`flathub-pr-body.md`. It needs `gh` logged in as the account that will own the submission.

Then answer the reviewers on that PR. When it's merged Flathub creates
`flathub/mg.diary.linksy.Linksy`, builds, and publishes; accept the repository invitation.

## Later releases

`scripts/publish-release.sh` still does the release. After Flathub lists the app, update it by
copying the new `target/flathub/vX.Y.Z` files (from `scripts/flathub-submit.sh vX.Y.Z`) into
a pull request on `flathub/mg.diary.linksy.Linksy`. When `Cargo.lock` changes, regenerate
`cargo-sources.json` first (see `scripts/package-flatpak.sh`).

## Why the manifest asks for what it does

Reviewers question broad permissions; the manifest comments say what each is for. In short:
`--share=network` and `--device=all` (the LAN protocol; a phone using this computer's camera
as a webcam, since V4L2 has no portal), `xdg-download`/`xdg-pictures` (received files and
imported photos), `xdg-run/pipewire-0` (publishing a phone's camera), and the session and
system bus names for notifications, media controls, the Files sidebar, the tray, locking the
screen and NetworkManager Wi-Fi Direct.

Only x86_64 is built (`flathub.json`), matching what CI tests. Remove that file's
restriction when an aarch64 build is tested.
