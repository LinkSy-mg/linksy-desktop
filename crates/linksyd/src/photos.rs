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
        linksy_core::identity::write_private(&self.path, &json).map_err(|e| e.to_string())?;
        apply(node, &prefs);
        *self.prefs.lock().unwrap() = prefs;
        Ok(())
    }
}

fn apply(node: &Node, prefs: &PhotoPrefs) {
    node.set_import_dir(prefs.import.then(|| prefs.folder.clone()));
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn the_saved_choice_is_owner_only_even_over_a_looser_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = linksy_core::Config::new(dir.path().join("n"), dir.path().join("dl"), "photos");
        c.discovery = false;
        let node = Node::start(c).await.unwrap();
        let path = dir.path().join("photos.json");
        std::fs::write(&path, b"{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let photos = Photos::load(path.clone(), &node);
        photos.set_prefs(&node, PhotoPrefs { import: false, folder: dir.path().join("pics") }).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
