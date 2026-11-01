# Bundled icons

Eleven symbolic icons the app asks for by name that not every desktop's icon theme has
(KDE's Breeze has none of them, and GTK then draws its broken-image placeholder). They are
compiled into the binary as part of hicolor (`icons.gresource.xml`, `build.rs`) and added to
the icon theme's resource path at startup, so whichever theme is in use, the app draws these
from here rather than depending on what is installed.

The files are from GNOME's [adwaita-icon-theme](https://gitlab.gnome.org/GNOME/adwaita-icon-theme)
(version 51), `symbolic/` tree, unmodified, and are dual-licensed
`CC-BY-SA-3.0 OR LGPL-3.0-only`: Linksy uses them under the LGPL, like the rest of the
icon theme. To add one, drop the file in `scalable/status/` and add it to
`icons.gresource.xml` — the test in `src/main.rs` fails if the list and the directory
disagree.
