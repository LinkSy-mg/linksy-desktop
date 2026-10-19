# Linksy

Your devices, connected. Share files, notifications and more between nearby devices,
directly over the local network, with no account, cloud or server.

## Layout

| Path | What it is |
|---|---|
| `crates/linksy-core` | Device identity, discovery (mDNS), pairing, QUIC transport, file transfer. No UI. |
| `crates/linksyd` | The always-on background service. Keeps paired devices connected, receives files, shows desktop notifications. |
| `crates/linksy-ipc` | The local socket protocol between the service and its front ends. |
| `crates/linksy-cli` | `linksy`, the command-line front end. |
| `crates/linksy-gtk` | `linksy-app`, the GTK4/libadwaita app for Linux. |
| `brand/` | Logo and brand assets. |
| `data/` | Desktop entry, icons, systemd user unit. |

Front ends come and go; the service does the work. With no window open, Linksy is one
small process (about 6 MB on disk, 7 MB of memory, no CPU when idle).

## How it works

- **Identity.** Each device makes a self-signed certificate on first run. Its SHA-256
  fingerprint is the device id. There is no central authority.
- **Discovery.** Devices advertise `_linksy._udp.local.` over mDNS. Paired devices are
  also dialed directly at their last known address, so reconnecting doesn't wait for
  discovery.
- **Pairing.** Both screens show a 6-digit code derived from the encrypted session, and
  both users confirm it. A device in the middle would see a different code.
- **Transport.** QUIC (TLS 1.3, mutual authentication). There is one connection per paired
  device, and each transfer or message gets its own stream, so a large file never blocks a
  notification.
- **Transfers.** Files are streamed from disk with a BLAKE3 check per file. Interrupted
  transfers resume, and a file only appears once its checksum matches.
- **Consent.** Pairing and incoming files always ask first, in the app or through a desktop
  notification with Accept/Decline buttons.

## Build

Rust stable (edition 2024). The service, CLI and core build anywhere:

```sh
cargo build --release      # linksyd + linksy
cargo test                 # unit + end-to-end tests
```

The GTK app needs GTK ≥ 4.14 and libadwaita ≥ 1.6 headers:

```sh
sudo dnf install gtk4-devel libadwaita-devel        # Fedora
sudo apt install libgtk-4-dev libadwaita-1-dev      # Debian/Ubuntu (24.10+)
cargo build --release -p linksy-app
```

Without root, a toolbox works: `toolbox create linksy-dev`, then
`toolbox run -c linksy-dev sudo dnf install -y gtk4-devel libadwaita-devel gcc`, then
build with `toolbox run -c linksy-dev cargo build -p linksy-app`.

## Run

```sh
scripts/install-local.sh   # installs to ~/.local and enables the user service
linksy-app                 # the app
linksy devices             # or the CLI
linksy pair "Other Laptop"
linksy send "Other Laptop" ~/Pictures/photo.jpg
```

Or run without installing: `cargo run -p linksyd` in one terminal, then
`cargo run -p linksy -- devices`.

Firewall: allow UDP 47200 (Linksy) and UDP 5353 (mDNS). On Fedora:
`sudo firewall-cmd --permanent --add-port=47200/udp --add-service=mdns && sudo firewall-cmd --reload`.

## Roadmap

- [x] Discovery, pairing, persistent identities, reconnect
- [x] File and folder transfer with progress, integrity check, resume
- [x] Device-to-device notifications
- [x] Background service, desktop notifications, CLI
- [x] GTK4/libadwaita app
- [ ] Clipboard sync
- [ ] Tray icon and "Send with Linksy" in the file manager
- [ ] macOS (AppKit) and Windows (WinUI) apps
- [ ] Screen mirroring (optional module)

## Development aids

- `scripts/ui-scene.sh <dir>` starts two local services ("This Laptop" and "Studio Desktop")
  and pairs them. Point the app or CLI at one with `LINKSY_SOCKET=<dir>/a.sock`.
- `scripts/ui-shot.sh <socket> <out.png> [delay-ms] [light|dark] [files…]` renders the app
  on a headless Wayland compositor (weston, inside the toolbox) and saves a screenshot.
  Nothing appears on screen.

Screenshots are in `docs/screenshots/`.
