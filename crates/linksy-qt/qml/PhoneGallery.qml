// A phone's photos and videos, kept by its page for the Gallery tab (docs/protocol.md §4.18):
// newest first and under a heading for each day, a page at a time; each thumbnail asked
// for once, by the square showing it
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
    // The selected items' ids, and a counter that changes with it for squares to follow
    property var chosen: ({})
    property int selectionRev: 0
    // Where a range starts (Shift-click)
    property int anchor: -1
    property ListModel items: ListModel {}
    // What the list shows: the same items cut into rows of `columns` under a heading for
    // each day, as {kind: "day", label} or {kind: "photos", start, size}
    property ListModel rows: ListModel {}
    property var laid: []
    property int columns: 4
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
        return { itemId: i.id, name: i.name, video: i.video ?? false, taken_ms: i.taken_ms, duration_ms: i.duration_ms ?? -1 };
    }

    // Cut the items into rows again, touching only the rows from the first that changed
    function layout() {
        const next = [];
        let day = null, open = null;
        for (let i = 0; i < items.count; i++) {
            const label = app.sectionLabel(items.get(i).taken_ms);
            if (label !== day) {
                next.push({ kind: "day", label: label, start: 0, size: 0, key: "d" + label });
                day = label;
                open = null;
            }
            if (open === null || open.size >= columns) {
                open = { kind: "photos", label: "", start: i, size: 0, key: "p" + i };
                next.push(open);
            }
            open.size++;
            open.key += "," + items.get(i).itemId;
        }
        let keep = 0;
        while (keep < laid.length && keep < next.length && laid[keep].key === next[keep].key) keep++;
        if (keep < rows.count) rows.remove(keep, rows.count - keep);
        for (let i = keep; i < next.length; i++) rows.append(next[i]);
        laid = next;
    }

    function setColumns(n) {
        if (n === columns) return;
        columns = n;
        layout();
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
            layout();
            clearSelection();
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
        if (fresh.length) {
            items.append(fresh.map(i => row(i)));
            layout();
        }
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
                if (chosen[shown.itemId]) {
                    delete chosen[shown.itemId];
                    selectedCount--;
                    selectionRev++;
                }
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
        if (changed) {
            anchor = -1;
            layout();
        }
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
        const id = items.get(index).itemId;
        if (chosen[id]) {
            delete chosen[id];
            selectedCount--;
        } else {
            chosen[id] = true;
            selectedCount++;
        }
        selectionRev++;
        anchor = index;
    }

    function selectOnly(index) {
        clearSelection();
        toggle(index);
    }

    function selectRange(index) {
        if (anchor < 0) return toggle(index);
        for (let i = Math.min(anchor, index); i <= Math.max(anchor, index); i++) {
            const id = items.get(i).itemId;
            if (chosen[id]) continue;
            chosen[id] = true;
            selectedCount++;
        }
        selectionRev++;
    }

    function clearSelection() {
        chosen = ({});
        selectedCount = 0;
        selectionRev++;
        anchor = -1;
    }

    function selectedIds() {
        const ids = [];
        for (let i = 0; i < items.count; i++) {
            if (chosen[items.get(i).itemId]) ids.push(items.get(i).itemId);
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
