// A list for a ListView that changes row by row (insert, move, remove by key) instead of all at
// once, so the view keeps its place. Delegates get `key` (and any extra roles) and read
// their object from `rows`.
import QtQuick

QtObject {
    id: keyed
    // The objects by key
    property var rows: ({})
    property ListModel model: ListModel {}

    // `list` in order; `keyOf(x, i)` a unique string; `rolesOf(x)` scalar roles for sections
    function sync(list, keyOf, rolesOf) {
        const next = {};
        const wanted = [];
        for (let i = 0; i < list.length; i++) {
            const row = Object.assign({ key: String(keyOf(list[i], i)) }, rolesOf ? rolesOf(list[i]) : {});
            next[row.key] = list[i];
            wanted.push(row);
        }
        const present = {};
        let kept = 0;
        for (let i = 0; i < model.count; i++) {
            const key = model.get(i).key;
            if (key in next) {
                present[key] = true;
                kept++;
            }
        }
        // Rows going away keep their object until the next time, for delegates still closing
        const gone = {};
        // Nothing in common (another folder, another phone): all at once
        if (kept === 0) {
            model.clear();
            rows = Object.assign(gone, rows, next);
            if (wanted.length) model.append(wanted);
            return;
        }
        for (let i = model.count - 1; i >= 0; i--) {
            const key = model.get(i).key;
            if (present[key]) continue;
            gone[key] = rows[key];
            model.remove(i);
        }
        rows = Object.assign(gone, next);
        for (let i = 0; i < wanted.length; i++) {
            const row = wanted[i];
            let at = -1;
            for (let j = i; j < model.count && present[row.key]; j++) {
                if (model.get(j).key === row.key) {
                    at = j;
                    break;
                }
            }
            if (at < 0) {
                model.insert(i, row);
                continue;
            }
            if (at !== i) model.move(at, i, 1);
            const old = model.get(i);
            for (const role in row) {
                if (old[role] !== row[role]) {
                    model.set(i, row);
                    break;
                }
            }
        }
    }
}
