// The folder a phone shares, kept by its page for the Files tab: the folder shown and what's
// in it, what's done to it (new folders, renames, deletes) and the copies to and from it,
// which run in the service and say how they go
import QtQuick

QtObject {
    id: files
    required property var app
    property string phone: ""
    // "/" first, always ending in "/"
    property string path: "/"
    // No hidden ones
    property var entries: []
    // Why the folder couldn't be read, "" when it could
    property string error: ""
    // Only a folder that takes a while says it's loading
    property bool waiting: false
    property bool loading: false
    property int loads: 0
    // What to pick once the folder is read again (a new folder, a renamed one)
    property string pick: ""
    // Where apps open the phone's files (dav://…/), once asked
    property string place: ""
    // Name, date or size; folders first whichever the order
    property string sortBy: "name"
    property bool descending: false
    // Copies going on, any phone's: op, phone, name, done, total, upload
    property ListModel ops: ListModel {}
    property Timer slow: Timer {
        interval: 300
        onTriggered: if (files.loading) files.waiting = true
    }

    // The folder was read; `moved`: it's another one than before
    signal listed(bool moved)

    function capitalize(text) {
        return text.charAt(0).toUpperCase() + text.slice(1);
    }

    function fileUrl(local) {
        return "file://" + local.split("/").map(encodeURIComponent).join("/");
    }

    function reset(id) {
        phone = id;
        path = "/";
        entries = [];
        error = "";
        pick = "";
        place = "";
        loading = false;
        waiting = false;
    }

    // Read a folder, the one shown when none is given, picking `name` once it's there
    function list(to, name) {
        const id = phone, at = to ?? path, moved = at !== path;
        if (!id) return;
        path = at;
        pick = name ?? "";
        const load = ++loads;
        loading = true;
        if (entries.length === 0) waiting = true;
        else if (moved) slow.restart();
        app.request("fs_list", { phone: id, path: at }, (ok, found) => {
            if (load !== files.loads || id !== files.phone) return;
            files.loading = false;
            files.waiting = false;
            files.slow.stop();
            files.error = ok ? "" : files.capitalize(found);
            files.entries = ok ? found.filter(e => !e.name.startsWith(".")) : [];
            files.listed(moved);
        });
    }

    function reload(name) {
        list(path, name);
    }

    // Into the download folder, in a folder named after the phone; how it goes comes later
    function copy(names) {
        const id = phone, dir = path;
        app.request("fs_copy", { phone: id, paths: names.map(n => dir + n) }, (ok, err) => { if (!ok) files.app.toastError(err, id) });
    }

    // Files and folders from here into the folder shown
    function upload(paths) {
        const id = phone;
        app.request("fs_upload", { phone: id, dir: path, paths: paths }, (ok, err) => { if (!ok) files.app.toastError(err, id) });
    }

    function mkdir(name) {
        const id = phone;
        app.request("fs_mkdir", { phone: id, path: path + name }, (ok, err) => {
            if (!ok) files.app.toastError(err, id);
            else if (id === files.phone) files.reload(name);
        });
    }

    function rename(old, to) {
        const id = phone;
        app.request("fs_rename", { phone: id, path: path + old, to: to }, (ok, err) => {
            if (!ok) files.app.toastError(err, id);
            else if (id === files.phone) files.reload(to);
        });
    }

    function remove(names) {
        const id = phone, dir = path;
        app.request("fs_delete", { phone: id, paths: names.map(n => dir + n) }, (ok, err) => {
            if (!ok) files.app.toastError(err, id);
            if (id === files.phone) files.reload();
        });
    }

    function cancel(op) {
        app.request("fs_cancel", { op: op });
    }

    // A copy to or from a phone went on, or ended
    function progress(p) {
        let at = -1;
        for (let i = 0; i < ops.count && at < 0; i++) {
            if (ops.get(i).op === p.op) at = i;
        }
        if (p.finished) {
            if (at >= 0) ops.remove(at);
            finished(p);
            return;
        }
        const row = { op: p.op, phone: p.phone, name: p.name, done: p.done, total: p.total, upload: typeof p.dir === "string" };
        if (at < 0) ops.append(row);
        else ops.set(at, row);
    }

    // Where it went, or why it didn't; an upload's folder shows what came
    function finished(p) {
        const upload = typeof p.dir === "string";
        if (p.error === "cancelled") {
            // Stopped on purpose: nothing to say
        } else if (p.error && upload) {
            app.toast(app.t("Couldn't upload {name}: {error}", { name: p.name, error: p.error }));
        } else if (p.error) {
            app.toast(app.t("Couldn't copy {name}: {error}", { name: p.name, error: p.error }));
        } else if (!upload && p.saved?.length) {
            // Saved in <download folder>/<phone>/
            const first = p.saved[0], parts = first.split("/");
            app.toastAction(app.t("Copied to {folder}", { folder: parts[parts.length - 3] ?? "" }), app.t("Show in Folder"),
                            () => Qt.openUrlExternally(files.fileUrl(first.slice(0, first.lastIndexOf("/")))));
        }
        if (upload && p.phone === phone && p.dir.replace(/\/+$/, "") === path.replace(/\/+$/, "")) reload();
    }

    // A file, or a folder in the file manager, through the service's bridge
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
