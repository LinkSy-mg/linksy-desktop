fn main() {
    // The icons in icons/: a few names KDE's Breeze and other themes don't have.
    glib_build_tools::compile_resources(&["icons"], "icons/icons.gresource.xml", "icons.gresource");
}
