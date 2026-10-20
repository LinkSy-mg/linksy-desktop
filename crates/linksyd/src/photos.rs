//! Photo auto-import: where phones' imported photos go, and whether they're taken.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use linksy_core::Node;
use linksy_ipc::PhotoPrefs;

#[derive(Clone)]
pub struct Photos {
    path: PathBuf,
    prefs: Arc<Mutex<PhotoPrefs>>,
}

impl Photos {
    /// Load the choice (on, into Pictures, unless changed) and apply it to the node.
    pub fn load(path: PathBuf, node: &Node) -> Self {
        let prefs = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| PhotoPrefs { import: true, folder: crate::system::pictures_dir() });
        apply(node, &prefs);
        Self { path, prefs: Arc::new(Mutex::new(prefs)) }
    }

    pub fn prefs(&self) -> PhotoPrefs {
        self.prefs.lock().unwrap().clone()
    }

    pub fn set_prefs(&self, node: &Node, prefs: PhotoPrefs) -> Result<(), String> {
        if !prefs.folder.is_absolute() {
            return Err("the photo folder must be a full path".into());
        }
        let json = serde_json::to_vec_pretty(&prefs).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())?;
        apply(node, &prefs);
        *self.prefs.lock().unwrap() = prefs;
        Ok(())
    }
}

fn apply(node: &Node, prefs: &PhotoPrefs) {
    node.set_import_dir(prefs.import.then(|| prefs.folder.clone()));
}
