# Desktop support

Linksy is built and tested on GNOME and KDE Plasma. This page covers how each feature
behaves on other desktops (Hyprland, Sway, niri, X11 desktops, distributions without
systemd), and the plan to make them all work.

Beyond GNOME and KDE, this comes from the code and from what each desktop's portals are
known to support. It hasn't been tested on those desktops yet; portal support moves fast,
so check on a real machine before relying on it.

## Works everywhere

These don't depend on the desktop: pairing, file transfer, the phone's notifications,
battery and Ring, media controls (MPRIS), the terminal, the USB cable, the webcam and
microphone (v4l2loopback and PipeWire), and opening links.

## Today

*Linksy 0.9 adds: a second display on Hyprland and Sway (a headless output it makes and
removes), the installed screen locker for Lock When Away, an installed polkit agent started
when none runs, a notification daemon suggested where none runs, the line that starts
Linksy at login on tiling compositors, an XDG autostart entry for desktops without
systemd's session, X11 clipboard watching, and GNOME's portal suggested on niri. The
tables below describe 0.8; the plan's remaining steps are screen control through the
virtual input protocols, Linksy's own notifications, and the in-app file browser.*

### Hyprland, Sway, niri and other tiling compositors

| Feature | Status | Why |
|---|---|---|
| Notifications with Reply and actions | Needs a notification daemon | None ships by default. mako, dunst and swaync work; support for reply and actions varies. |
| Finish Setup (password prompt) | Fails without a polkit agent | `pkexec` needs an agent, and Hyprland and Sway start none. `hyprpolkitagent` or `polkit-gnome` must be running. |
| Clipboard sync | Works | `wl-paste --watch` needs the data-control protocol, which wlroots compositors and Hyprland have. |
| Watching the computer's screen | Works with the right portal | `xdg-desktop-portal-hyprland` on Hyprland, `-wlr` on Sway, `-gnome` on niri. |
| Screen control, phone as touchpad | Likely missing | Needs the RemoteDesktop portal, which the Hyprland and wlr portals don't implement. |
| Sidecar (phone as a second display) | Likely missing | Asks the portal for a virtual monitor, which only GNOME and KDE offer. |
| Lock When Away | Only with a configured locker | Falls back to `loginctl lock-session`, which locks only if `hypridle` or `swayidle` reacts to it. |
| Phone in the Files sidebar | Depends on the file manager | Works with gvfs-based ones (Nautilus, Thunar). |

### X11 desktops (XFCE, Cinnamon, MATE, i3)

- The clipboard works both ways with `xclip` or `xsel`: copies on the computer reach the
  phone when the clipboard changes (XFixes).
- Watching and controlling the screen work without a portal (0.10): Linksy reads the
  primary monitor with `ximagesrc` and drives it with XTest. Sidecar needs Wayland: X11
  can't add a monitor.

### Distributions without systemd (Void, Artix, Gentoo with OpenRC)

Desktops start the service at login from its XDG autostart entry; on a tiling compositor,
Finish Setup gives the line for its config. Linksy Camera comes back at boot through
`modules-load.d` instead of a systemd service (0.9.13), and Restart Now uses `loginctl`.
Lock When Away needs `elogind`.

## Plan

Each gap gets a fallback Linksy handles itself, rather than a package the user has to find.

| Gap | Fallback | Covers | Effort |
|---|---|---|---|
| No polkit agent | Linksy's own password dialog, handed to `pkttyagent` (ships with polkit, wherever `pkexec` is). The password only goes to polkit's helper. | everywhere | small |
| No notification daemon | When nobody owns `org.freedesktop.Notifications`, Linksy shows its own popups with Reply and actions, as a layer-shell overlay (gtk4-layer-shell). | Hyprland, Sway, niri | medium |
| No ScreenCast portal | Capture the X display with GStreamer's `ximagesrc` (gst-plugins-good, already a dependency). Done in 0.10. | X11 | small |
| Screen control without RemoteDesktop | Input through the compositor's virtual pointer and keyboard protocols (`zwlr_virtual_pointer_v1`, `zwp_virtual_keyboard_v1`), as wayvnc does. XTest on X11, which needs no permission (done in 0.10). | Hyprland, Sway, X11 | medium |
| Sidecar without a virtual monitor | A real headless output, created for the session and removed after: `hyprctl output create headless`, `swaymsg create_output`. | Hyprland, Sway | medium |
| Lock When Away does nothing | Run the installed locker: `hyprlock`, `swaylock`, `xflock4`, `cinnamon-screensaver-command -l`, `light-locker-command -l`, `i3lock`, `xdg-screensaver lock`. | everywhere | small |
| No clipboard watching on X11 | Selection-change events through XFixes. | X11 | small |
| No autostart without systemd | An XDG autostart entry, which most desktops read. On Hyprland, a button that adds `exec-once` to the user's config, with their OK. | Void, Artix, Hyprland | small |
| File manager without gvfs | The phone's files in a browser window inside Linksy; the core already browses them. | everywhere | medium |
| niri portal name | Suggest `xdg-desktop-portal-gnome`. | niri | trivial |

Finish Setup also learns the tiling compositors: it checks for a polkit agent, a
notification daemon and the screen-sharing portal, and says exactly what's missing. Features
a desktop can't do are hidden, the way features a phone can't do already are.

### Not planned

- **`uinput` for screen control.** It works everywhere, but lets any of the user's programs
  type as a keyboard. The compositor protocols and XTest are scoped to the session instead.

### What stays limited

- **Clipboard watching on GNOME.** GNOME doesn't let apps watch the clipboard; only a GNOME
  Shell extension could.
- **Sidecar on niri and X11.** niri has no virtual outputs; X11 needs the evdi kernel module
  or a dummy display.
- **Screen control on niri.** Depends on niri's support for the virtual input protocols; to
  check.

## Order

1. Polkit password dialog, the lockers, the niri portal name, X11 clipboard watching.
   These fix things that fail outright.
2. Autostart without systemd, Linksy's own notifications.
3. Screen control and Sidecar on Hyprland and Sway.
4. The in-app file browser.

They ship as the 0.9.x releases, one step each (see ROADMAP.md).

## Testing

Sway and Hyprland both run in a window inside an existing session, so every step can be
tried in a nested compositor during development. Each release still gets one run on a real
Hyprland install before it ships.
