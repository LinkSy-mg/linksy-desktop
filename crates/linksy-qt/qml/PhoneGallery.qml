// A phone's photos and videos, kept by its page for the Gallery tab (docs/protocol.md §4.18):
// newest first, a page at a time; each thumbnail asked for once, by the square showing it
import QtQuick

QtObject {
    id: gallery
    required property var app
    property string phone: ""
    property var albums: []
    // The album shown, "" for all of them
    property string album: ""
    property bool more: false
    property bool loading: false
    // Bumped when what's shown starts over, so late answers are dropped
    property int generation: 0
    // What shows instead of the grid ("Loading…", none yet, an error), "" for the grid
    property string status: ""
    property int selectedCount: 0
    // Where a range starts (Shift-click)
    property int anchor: -1
    property ListModel items: ListModel {}
    // Thumbnails on disk by item id, and the ones asked for
    property var thumbs: ({})
    property var asked: ({})
    readonly property int total: albums.reduce((n, a) => n + a.count, 0)

    signal thumbReady(string id, string path)
    signal pageLanded()

    function capitalize(text) {
        return text.charAt(0).toUpperCase() + text.slice(1);
    }

    function row(i) {
        return { itemId: i.id, name: i.name, video: i.video ?? false, taken_ms: i.taken_ms, duration_ms: i.duration_ms ?? -1, selected: false };
    }

    // The Gallery tab shows: the phone's albums and newest photos the first time
    function show(id) {
        if (phone === id && items.count > 0) return;
        phone = id;
        album = "";
        albums = [];
        thumbs = {};
        asked = {};
        loadAlbums();
        page(true);
    }

    function loadAlbums() {
        const id = phone;
        app.request("gallery_albums", { phone: id }, (ok, list) => {
            if (ok && id === gallery.phone) gallery.albums = list;
        });
    }

    // The first page (`fresh`), or the next one when there's more and none is coming
    function page(fresh) {
        if (!phone) return;
        let before = null;
        if (fresh) {
            generation++;
            items.clear();
            selectedCount = 0;
            anchor = -1;
            more = false;
            status = app.t("Loading…");
        } else {
            if (loading || !more || items.count === 0) return;
            const last = items.get(items.count - 1);
            before = { taken_ms: last.taken_ms, id: last.itemId };
        }
        loading = true;
        const at = generation;
        app.request("gallery_items", { phone: phone, album: album || null, before: before, limit: 120 }, (ok, got) => {
            if (at !== gallery.generation) return;
            gallery.loading = false;
            if (ok) gallery.landed(got);
            else if (gallery.items.count === 0) gallery.status = gallery.capitalize(got);
            else gallery.app.toast(gallery.capitalize(got));
        });
    }

    function landed(got) {
        more = got.more;
        if (got.items.length) items.append(got.items.map(i => row(i)));
        status = items.count ? "" : app.t("No photos here yet.");
        pageLanded();
    }

    // The phone's photos changed: the ones taken since the newest shown, in front
    function newer() {
        if (!phone) return;
        const at = generation;
        app.request("gallery_items", { phone: phone, album: album || null, before: null, limit: 120 }, (ok, got) => {
            if (ok && at === gallery.generation) gallery.putNewer(got.items);
        });
    }

    function putNewer(list) {
        const first = items.count ? items.get(0) : null;
        const fresh = list.filter(i => !first || i.taken_ms > first.taken_ms || (i.taken_ms === first.taken_ms && i.id > first.itemId));
        if (!fresh.length) return;
        items.insert(0, fresh.map(i => row(i)));
        if (anchor >= 0) anchor += fresh.length;
        status = "";
    }

    // Where item `id`'s thumbnail is; "" until it's here, and asked for then
    function thumb(id) {
        if (id in thumbs) return thumbs[id];
        if (!(id in asked)) {
            asked[id] = true;
            fetch(id);
        }
        return "";
    }

    function fetch(id) {
        const of = phone;
        app.request("gallery_thumb", { phone: of, item: id }, (ok, path) => {
            if (of !== gallery.phone) return;
            if (!ok) {
                delete gallery.asked[id];
                return;
            }
            gallery.thumbs[id] = path;
            gallery.thumbReady(id, path);
        });
    }

    function toggle(index) {
        const on = !items.get(index).selected;
        items.setProperty(index, "selected", on);
        selectedCount += on ? 1 : -1;
        anchor = index;
    }

    function selectOnly(index) {
        clearSelection();
        toggle(index);
    }

    function selectRange(index) {
        if (anchor < 0) return toggle(index);
        for (let i = Math.min(anchor, index); i <= Math.max(anchor, index); i++) {
            if (items.get(i).selected) continue;
            items.setProperty(i, "selected", true);
            selectedCount++;
        }
    }

    function clearSelection() {
        for (let i = 0; i < items.count && selectedCount > 0; i++) {
            if (!items.get(i).selected) continue;
            items.setProperty(i, "selected", false);
            selectedCount--;
        }
        selectedCount = 0;
        anchor = -1;
    }

    function selectedIds() {
        const ids = [];
        for (let i = 0; i < items.count; i++) {
            if (items.get(i).selected) ids.push(items.get(i).itemId);
        }
        return ids;
    }

    // Have the phone send these into the photos folder; `open`: open it once it's here
    function save(ids, open) {
        if (!ids.length) return;
        const n = ids.length;
        app.request("gallery_save", { phone: phone, ids: ids }, (ok, transfer) => {
            if (!ok) gallery.app.toast(gallery.capitalize(transfer));
            else if (open) gallery.app.openWhenDone(transfer);
            else gallery.app.toast(gallery.app.t("Saving {what} to your photos folder", { what: gallery.app.plural(n, "item") }));
        });
    }
}
