# Handover

One keyboard and mouse for two computers. Plug them into the first, press a hotkey, and
keep working on the second; press it again to come back. Nothing is unplugged.

## How it works

- **The hotkey is the only trigger** (both Ctrl keys by default, settable). The pointer
  never switches computers by touching a screen edge.
- **On the computer with the real keyboard and mouse,** the hotkey takes them over (an
  exclusive grab on their `/dev/input` devices): nothing is typed or clicked there while
  they are on the other computer. Taking over waits until every key and button is up, so
  none stays pressed.
- **On the other computer,** the events appear on a virtual keyboard and mouse
  (`/dev/uinput`) that exist only while the sharing lasts. They send key positions, so that
  computer's own layout applies, as if the keyboard had been plugged in there. It works
  on every desktop and at the lock screen.
- **Giving back** lifts every key still down on the other computer, then returns the
  devices once everything is up.

## Safeguards

- The grab ends when the program ends or dies: the kernel gives the devices back by itself.
- The link dropping, or the other computer going away, gives the devices back at once
  (`Router::abort`).
- Only between paired computers, with an Always Allow per pair, and never from away from
  home unless Allow Everything From Away is on for that computer.
- Both computers say when it is on.

## Permission

Reading `/dev/input` and writing `/dev/uinput` need more than a normal user has (here both
are root only), and a program that reads every key could log them. So the part that touches
the devices is this small program, `linksy-handover`, and not the service. It is meant to be
started only while the feature is on, through pkexec with Linksy's own password prompt, and
to stop when it is turned off.

## Built so far (branch `continuous-keys`)

- `crates/linksy-handover`: events, the hotkey, the router (the rules above, with tests),
  reading and grabbing real devices, and the virtual device. The ioctl numbers and struct
  sizes are checked against the kernel headers.
- `linksy-handover devices` lists what can be shared.
- `linksy-handover loopback` tests both halves on one computer.

Not built yet: carrying the events between computers (a session over the paired link),
starting the helper through pkexec, the setting and hotkey choice and the switch in both
apps, Finish Setup, the packages, and its name.

## Trying the loopback test

On the computer with the keyboard and mouse, from a terminal:

```
cargo build -p linksy-handover
sudo target/debug/linksy-handover devices
sudo target/debug/linksy-handover loopback --seconds 60
```

Press both Ctrl keys, release them: it says "now on the virtual keyboard and mouse", and
typing and moving the mouse still work, now through the virtual device. Press both Ctrl
again: "back on the real ones". It ends by itself after the seconds given, and Ctrl+C ends it
too. At the end it prints how many events went through the virtual device.
