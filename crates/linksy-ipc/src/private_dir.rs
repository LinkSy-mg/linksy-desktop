//! The private folder sockets fall back to when `$XDG_RUNTIME_DIR` is unset.
//!
//! A shared `/tmp` lets another local user pre-create the socket's path (a folder, a symlink,
//! a socket of their own), so the service or an app would bind or connect inside their reach.
//! The fallback is a per-user folder that is created 0700 and checked (ours, not a symlink,
//! 0700) before any socket is bound or connected in it.

use std::io;
use std::path::{Path, PathBuf};

/// `linksy-<uid>` in the temp folder: where sockets go without `$XDG_RUNTIME_DIR`.
pub fn fallback_dir() -> PathBuf {
    std::env::temp_dir().join(format!("linksy-{}", current_uid()))
}

fn current_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail
    unsafe { libc::geteuid() }
}

/// Create `dir` with mode 0700 if it's missing, then check it with [`verify_private_dir`].
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    verify_private_dir(dir)
}

/// Refuse unless `dir` is a real folder (not a symlink), owned by this user, with mode 0700.
pub fn verify_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // symlink_metadata: a symlink is reported as one, never followed
    let meta = std::fs::symlink_metadata(dir)?;
    let refuse = |why: &str| Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: {why}; refusing to put Linksy's sockets there (set XDG_RUNTIME_DIR, or remove it)", dir.display())));
    if !meta.file_type().is_dir() {
        return refuse("not a plain folder (a symlink or a file is in the way)");
    }
    if meta.uid() != current_uid() {
        return refuse("owned by another user");
    }
    if meta.permissions().mode() & 0o777 != 0o700 {
        return refuse("not private (mode must be 0700)");
    }
    Ok(())
}

/// Make the folder of a socket about to be bound safe: the fallback folder is created and
/// verified; the folder of any other path (`$XDG_RUNTIME_DIR`, `--socket`) is the caller's choice.
pub fn prepare_socket_dir(socket: &Path) -> io::Result<()> {
    match socket.parent() {
        Some(dir) if dir == fallback_dir() => ensure_private_dir(dir),
        _ => Ok(()),
    }
}

/// Check, before connecting, that a socket in the fallback folder isn't in someone else's reach.
/// A folder that doesn't exist yet is left for the connect to fail on.
pub fn check_socket_dir(socket: &Path) -> io::Result<()> {
    match socket.parent() {
        Some(dir) if dir == fallback_dir() => match verify_private_dir(dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    /// A scratch folder, made while no test has TMPDIR pointed elsewhere.
    fn scratch() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        (tempfile::tempdir().unwrap(), lock)
    }

    fn mode(dir: &Path, mode: u32) {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn a_missing_folder_is_created_private() {
        let (tmp, _lock) = scratch();
        let dir = tmp.path().join("linksy-x");
        ensure_private_dir(&dir).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        verify_private_dir(&dir).unwrap();
    }

    #[test]
    fn our_own_private_folder_is_accepted_again() {
        let (tmp, _lock) = scratch();
        let dir = tmp.path().join("linksy-x");
        ensure_private_dir(&dir).unwrap();
        ensure_private_dir(&dir).unwrap();
    }

    #[test]
    fn a_symlink_in_the_way_is_refused() {
        let (tmp, _lock) = scratch();
        let target = tmp.path().join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        mode(&target, 0o700);
        let dir = tmp.path().join("linksy-x");
        symlink(&target, &dir).unwrap();
        assert!(ensure_private_dir(&dir).is_err());
        assert!(verify_private_dir(&dir).is_err());
    }

    #[test]
    fn a_folder_others_can_enter_is_refused() {
        let (tmp, _lock) = scratch();
        for bad in [0o755, 0o750, 0o770, 0o777, 0o701] {
            let dir = tmp.path().join(format!("linksy-{bad:o}"));
            std::fs::create_dir(&dir).unwrap();
            mode(&dir, bad);
            assert!(ensure_private_dir(&dir).is_err(), "{bad:o} accepted");
        }
    }

    #[test]
    fn a_plain_file_is_refused() {
        let (tmp, _lock) = scratch();
        let path = tmp.path().join("linksy-x");
        std::fs::write(&path, b"").unwrap();
        assert!(ensure_private_dir(&path).is_err());
    }

    #[test]
    fn a_folder_of_another_owner_is_refused() {
        // We can't chown without root; a folder owned by root stands in for another user's
        let root_dir = Path::new("/proc");
        if current_uid() != 0 {
            assert!(verify_private_dir(root_dir).is_err());
        }
    }

    #[test]
    fn only_the_fallback_folder_is_checked() {
        let (tmp, _lock) = scratch();
        // Not ours to judge: any other folder is the caller's choice
        let open = tmp.path().join("open");
        std::fs::create_dir(&open).unwrap();
        mode(&open, 0o777);
        prepare_socket_dir(&open.join("a.sock")).unwrap();
        check_socket_dir(&open.join("a.sock")).unwrap();
    }

    #[test]
    fn the_fallback_folder_is_named_for_the_user() {
        let dir = fallback_dir();
        assert!(dir.ends_with(format!("linksy-{}", current_uid())));
    }

    #[test]
    fn a_planted_fallback_folder_stops_bind_and_connect() {
        let (tmp, _lock) = scratch();
        let _guard = TempDirEnv::set(tmp.path());
        let dir = fallback_dir();
        std::fs::create_dir(&dir).unwrap();
        mode(&dir, 0o777);
        let socket = dir.join("linksyd.sock");
        assert!(prepare_socket_dir(&socket).is_err());
        assert!(check_socket_dir(&socket).is_err());
        mode(&dir, 0o700);
        prepare_socket_dir(&socket).unwrap();
        check_socket_dir(&socket).unwrap();
    }

    #[test]
    fn a_missing_fallback_folder_is_left_to_the_connect() {
        let (tmp, _lock) = scratch();
        let _guard = TempDirEnv::set(tmp.path());
        check_socket_dir(&fallback_dir().join("linksyd.sock")).unwrap();
        assert!(!fallback_dir().exists());
    }

    /// Points `temp_dir()` (TMPDIR) at a folder for one test; the caller holds the `scratch` lock.
    struct TempDirEnv {
        old: Option<std::ffi::OsString>,
    }
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    impl TempDirEnv {
        fn set(to: &Path) -> Self {
            let old = std::env::var_os("TMPDIR");
            // SAFETY: serialized by the ENV lock the caller holds; no other test in this crate reads TMPDIR meanwhile
            unsafe { std::env::set_var("TMPDIR", to) };
            Self { old }
        }
    }
    impl Drop for TempDirEnv {
        fn drop(&mut self) {
            // SAFETY: the caller's ENV lock outlives this guard (declared first, dropped last)
            unsafe {
                match &self.old {
                    Some(v) => std::env::set_var("TMPDIR", v),
                    None => std::env::remove_var("TMPDIR"),
                }
            }
        }
    }
}
