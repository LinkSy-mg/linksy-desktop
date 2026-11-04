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
    // The phone's photos changed while another tab showed
    property bool stale: false
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

    // The Gallery tab shows: the phone's albums and newest photos the first time, brought
    // up to date when they changed meanwhile
    function show(id) {
        if (phone === id && items.count > 0) {
            if (stale) {
                stale = false;
                loadAlbums();
                refresh();
            }
            return;
        }
        phone = id;
        stale = false;
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
        // A refresh may have put some of these in already
        const shown = new Set();
        for (let i = 0; i < items.count; i++) shown.add(items.get(i).itemId);
        const fresh = got.items.filter(i => !shown.has(i.id));
        if (fresh.length) items.append(fresh.map(i => row(i)));
        status = items.count ? "" : app.t("No photos here yet.");
        pageLanded();
    }

    // The phone's photos changed: the newest again, as many as shown (a phone gives 500 at
    // most), and the grid brought in line; the scroll position and the selection stay
    function refresh() {
        if (!phone) return;
        const at = generation;
        const limit = Math.min(500, Math.max(120, items.count));
        app.request("gallery_items", { phone: phone, album: album || null, before: null, limit: limit }, (ok, got) => {
            if (ok && at === gallery.generation) gallery.reconcile(got.items, got.more);
        });
    }

    // Shown ones within the page's span but not in it were deleted; older ones stay, as
    // does anything when the page holds all. Ties at the span's oldest millisecond stay:
    // the phone's order within one isn't known here.
    function reconcile(list, pageMore) {
        const oldest = pageMore && list.length ? list[list.length - 1].taken_ms : null;
        const ids = new Set(list.map(i => i.id));
        let changed = false;
        for (let i = items.count - 1; i >= 0; i--) {
            const shown = items.get(i);
            if ((oldest === null || shown.taken_ms > oldest) && !ids.has(shown.itemId)) {
                if (shown.selected) selectedCount--;
                items.remove(i);
                changed = true;
            }
        }
        // The page's order is the phone's: each new one right after the last one shown
        // before it in the page
        let at = 0;
        for (const i of list) {
            let found = -1;
            for (let j = at; j < items.count; j++) {
                if (items.get(j).itemId === i.id) {
                    found = j;
                    break;
                }
            }
            if (found >= 0) {
                at = found + 1;
                continue;
            }
            items.insert(at, row(i));
            at++;
            changed = true;
        }
        if (changed) anchor = -1;
        if (!pageMore) more = false;
        status = items.count ? "" : app.t("No photos here yet.");
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
