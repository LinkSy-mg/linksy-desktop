# Handover

One keyboard and mouse for two computers. Plug them into the first, press a hotkey, and
keep working on the second; press it again to come back. Nothing is unplugged. Linux
computers only (§4.27 of the protocol).

## How it works

- **The hotkey is the only trigger** (both Ctrl keys by default, `--hotkey` changes it). The
  pointer never switches computers by touching a screen edge.
- **On the computer with the real keyboard and mouse** (the sender), `linksy-handover send`
  watches their `/dev/input` devices. The hotkey takes them over (an exclusive grab): nothing
  is typed or clicked there while they are on the other computer. Taking over waits until
  every key and button is up, so none stays pressed. Touchpads and touchscreens stay put.
- **On the other computer** (the receiver), `linksy-handover receive` makes a virtual
  keyboard and mouse (`/dev/uinput`) that exist only while the session lasts. It gets key
  positions, so that computer's own layout applies, as if the keyboard had been plugged in
  there. It works on every desktop and at the lock screen.
- **Giving back** lifts every key still down on the other computer, then returns the
  devices once everything is up.
- **The service carries the bytes** between the program and the paired link, in a session of
  the new kind `handover`; the other computer's user is asked like for a screen (Always
  Allow is the same switch), and the person is told, on both computers, when the devices go
  over and come back.

## Safeguards

- The grab ends when the program ends or dies: the kernel gives the devices back by itself.
- The link dropping, or the other computer ending the session, gives the devices back at once.
- Only between paired computers; never from away from home unless Allow Everything From Away
  is on for that computer.
- The receiver announces `handover` only while it can make the virtual devices, and clamps
  and filters what arrives.
- Only the program the service started, or one of the person's own (root, or the same user),
  may connect to a session's socket.

## Permission

Reading `/dev/input` and writing `/dev/uinput` need more than a normal user has, and a
program that reads every key could log them, so the part that touches the devices is a
program of its own, `linksy-handover`, not the service.

- **Receiving:** a udev rule (`70-linksy-handover.rules`) lets the person at the screen
  open `/dev/uinput`, which also lets any program they run make virtual keyboards. So no
  package installs it: it is Finish Setup's "Keyboard and Mouse From Another Computer"
  (`linksy-system-setup handover`), on its own button, outside Set Up Everything, the count
  and the first-run offer. The program and the setup script are in the packages; the rule is
  only written when the person asks, from `linksy-handover-setup`.
- **Sending:** only when the person starts it. If they cannot read the devices, the root
  helper starts the program for them (`linksy-system-setup handover-send:SOCKET`) after the
  password prompt Finish Setup uses, and only for a socket that belongs to them.
- Exit status 77 from the program means "no permission".

## Trying it

Between two computers: install this build on both, run the one-time setup on the receiving
one (Finish Setup, or `sudo /usr/libexec/linksy/linksy-handover-setup`), pair them, then on
the sending one:

```
linksy handover <the other computer>
```

The other computer says yes; press both Ctrl keys.

On one computer alone, `linksy-handover loopback --seconds 60` (with sudo) hands the devices to
a virtual keyboard and mouse here and back. `linksy-handover send --demo` and
`receive --print` try the link without any device; the test is the one in
`crates/linksy-handover/src/link.rs` and the daemon's own.

## Not done

- Choosing the hotkey in the apps (only `linksy handover --hotkey`).
- The History page calls it "touchpad and keyboard".
- In the Flatpak the receiving computer cannot make the devices from the sandbox, so it does not
  announce the feature there.
