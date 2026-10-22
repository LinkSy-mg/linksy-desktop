# Contributing

Thanks for helping. Linksy is the Linux app; the Android app is in
[linksy-mobile](https://github.com/LinkSy-mg/linksy-mobile).

## Before you start

- **Bugs:** open an issue with the bug form; versions, desktop and phone matter.
- **Features:** check the [roadmap](ROADMAP.md) and its
  [board](https://github.com/orgs/LinkSy-mg/projects/1). For anything bigger than a small
  fix, say on the issue that you're on it, so two people don't build the same thing.
- **First time?** Issues labelled
  [good first issue](https://github.com/LinkSy-mg/linksy-desktop/labels/good%20first%20issue)
  are small and self-contained.

## Build and test

The [README](README.md#build) has the packages to install. In short:

```sh
cargo build                                   # service, CLI, core
cargo build -p linksy-app -p linksy-screen    # the app and the screen helper (GTK, GStreamer)
cargo test
cargo clippy --all-targets -- -D warnings
```

CI runs the same on every pull request, on Fedora and openSUSE.

To try your build: `scripts/install-local.sh` installs it to `~/.local` and turns on
the service.

## How the code is laid out

- `crates/linksy-core`: the protocol, pairing, transfers; shared with the phone (Rust).
- `crates/linksyd`: the service that runs all the time.
- `crates/linksy-gtk`: the app (`linksy-app`), a window on the service.
- `crates/linksy-screen`: screen, camera and microphone pipelines (GStreamer).
- `crates/linksy-ipc`: how the app talks to the service.

## Style

- **Don't reformat code you didn't change.** No `cargo fmt` over the tree; keep diffs to
  what the change needs.
- **Comments say why**, not what the next line does.
- **Text people see** goes through `t!("...")`. After adding or changing any, run
  `scripts/i18n.py sync`. Translations are welcome: `scripts/i18n.py todo LANG` lists
  what's missing.
- **The protocol:** anything the phone has to know goes in
  [docs/protocol.md](docs/protocol.md), with a dated line at the end.

## Commits and pull requests

- Small commits, each one doing one thing, in
  [Conventional Commits](https://www.conventionalcommits.org/) style with the part of
  Linksy as the scope: `fix(screen): ...`, `feat(app): ...`, `feat(daemon): ...`,
  `docs(protocol): ...`.
- Say in the pull request what a user will notice. The changelog is written at release
  time from that.
- AI tools are fine; you're responsible for what you send, and for having run it.

## License

Linksy is under the [GNU AGPL v3.0 or later](LICENSE). By contributing, you agree that
your contribution is under the same license.
