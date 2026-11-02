// The folder a phone shares, kept by its page for the Files tab: the folder shown and what's
// in it, and opening its files through the service's bridge
import QtQuick

QtObject {
    id: files
    required property var app
    property string phone: ""
    // "/" first, always ending in "/"
    property string path: "/"
    // Folders first, as the service sorts them; no hidden ones
    property var entries: []
    // What shows instead of the list ("Loading…", an empty folder, an error), "" for the list
    property string status: ""
    // Where apps open the phone's files (dav://…/), once asked
    property string place: ""

    function capitalize(text) {
        return text.charAt(0).toUpperCase() + text.slice(1);
    }

    function reset(id) {
        phone = id;
        path = "/";
        entries = [];
        status = "";
        place = "";
    }

    // List a folder, the one shown when none is given
    function list(to) {
        const id = phone, at = to ?? path;
        if (!id) return;
        path = at;
        if (entries.length === 0) status = app.t("Loading…");
        app.request("fs_list", { phone: id, path: at }, (ok, found) => {
            if (id !== files.phone || at !== files.path) return;
            if (!ok) {
                files.entries = [];
                files.status = files.capitalize(found);
                return;
            }
            files.entries = found.filter(e => !e.name.startsWith("."));
            files.status = files.entries.length ? "" : files.app.t("This folder is empty.");
        });
    }

    // A file, or a folder in the file manager
    function launch(target) {
        if (place) return Qt.openUrlExternally(webdav(place, target));
        const id = phone;
        app.request("fs_place", { phone: id }, (ok, where) => {
            if (!ok) return files.app.toast(files.capitalize(where));
            if (id === files.phone) files.place = where;
            Qt.openUrlExternally(files.webdav(where, target));
        });
    }

    // The top folder also goes in the file manager's sidebar
    function openInFiles() {
        if (path !== "/") return launch(path);
        app.request("browse_files", { phone: phone }, (ok, err) => { if (!ok) files.app.toast(files.capitalize(err)) });
    }

    // KDE opens dav:// addresses as webdav://, each name in the path escaped
    function webdav(where, target) {
        return where.replace(/^dav:/, "webdav:").replace(/\/+$/, "") + target.split("/").map(encodeURIComponent).join("/");
    }
}
