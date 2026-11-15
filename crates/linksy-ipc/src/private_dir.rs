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
    fallback_dir_in(&std::env::temp_dir())
}

/// `linksy-<uid>` inside `base`.
pub fn fallback_dir_in(base: &Path) -> PathBuf {
    base.join(fallback_name(current_uid()))
}

fn fallback_name(uid: u32) -> String {
    format!("linksy-{uid}")
}

/// Is `dir` a fallback folder (named for this user), wherever its base is?
fn is_fallback_dir(dir: &Path) -> bool {
    dir.file_name().is_some_and(|n| *n == *fallback_name(current_uid()))
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
    verify_private_dir_for(dir, current_uid())
}

/// [`verify_private_dir`] for an expected owner `uid`.
fn verify_private_dir_for(dir: &Path, uid: u32) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // symlink_metadata: a symlink is reported as one, never followed
    let meta = std::fs::symlink_metadata(dir)?;
    let refuse = |why: &str| Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: {why}; refusing to put Linksy's sockets there (set XDG_RUNTIME_DIR, or remove it)", dir.display())));
    if !meta.file_type().is_dir() {
        return refuse("not a plain folder (a symlink or a file is in the way)");
    }
    if meta.uid() != uid {
        return refuse("owned by another user");
    }
    if meta.permissions().mode() & 0o777 != 0o700 {
        return refuse("not private (mode must be 0700)");
    }
    Ok(())
}

/// Make the folder of a path about to be bound safe: a fallback folder (`linksy-<uid>`) is
/// created and verified; the folder of any other path (`$XDG_RUNTIME_DIR`, `--socket`) is the caller's choice.
pub fn prepare_socket_dir(socket: &Path) -> io::Result<()> {
    match socket.parent() {
        Some(dir) if is_fallback_dir(dir) => ensure_private_dir(dir),
        _ => Ok(()),
    }
}

/// Check, before connecting, that a socket in the fallback folder isn't in someone else's reach.
/// A folder that doesn't exist yet is left for the connect to fail on.
pub fn check_socket_dir(socket: &Path) -> io::Result<()> {
    match socket.parent() {
        Some(dir) if is_fallback_dir(dir) => match verify_private_dir(dir) {
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

    fn mode(dir: &Path, mode: u32) {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// A 0777 fallback folder planted under a scratch base, as another user could in /tmp.
    pub fn planted(base: &Path) -> PathBuf {
        let dir = fallback_dir_in(base);
        std::fs::create_dir(&dir).unwrap();
        mode(&dir, 0o777);
        dir
    }

    #[test]
    fn a_missing_folder_is_created_private() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("linksy-x");
        ensure_private_dir(&dir).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        verify_private_dir(&dir).unwrap();
    }

    #[test]
    fn our_own_private_folder_is_accepted_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("linksy-x");
        ensure_private_dir(&dir).unwrap();
        ensure_private_dir(&dir).unwrap();
    }

    #[test]
    fn a_symlink_in_the_way_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
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
        let tmp = tempfile::tempdir().unwrap();
        for bad in [0o755, 0o750, 0o770, 0o777, 0o701] {
            let dir = tmp.path().join(format!("linksy-{bad:o}"));
            std::fs::create_dir(&dir).unwrap();
            mode(&dir, bad);
            assert!(ensure_private_dir(&dir).is_err(), "{bad:o} accepted");
        }
    }

    #[test]
    fn a_plain_file_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("linksy-x");
        std::fs::write(&path, b"").unwrap();
        assert!(ensure_private_dir(&path).is_err());
    }

    #[test]
    fn a_folder_of_another_owner_is_refused() {
        // An otherwise valid 0700 folder of ours: only the expected owner differs
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("linksy-x");
        ensure_private_dir(&dir).unwrap();
        verify_private_dir_for(&dir, current_uid()).unwrap();
        let err = verify_private_dir_for(&dir, current_uid().wrapping_add(1)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains("another user"), "{err}");
    }

    #[test]
    fn only_the_fallback_folder_is_checked() {
        let tmp = tempfile::tempdir().unwrap();
        // Not ours to judge: any other folder is the caller's choice
        let open = tmp.path().join("open");
        std::fs::create_dir(&open).unwrap();
        mode(&open, 0o777);
        prepare_socket_dir(&open.join("a.sock")).unwrap();
        check_socket_dir(&open.join("a.sock")).unwrap();
    }

    #[test]
    fn the_fallback_folder_is_named_for_the_user() {
        assert!(fallback_dir().ends_with(format!("linksy-{}", current_uid())));
        let base = Path::new("/scratch");
        assert_eq!(fallback_dir_in(base), base.join(format!("linksy-{}", current_uid())));
    }

    #[test]
    fn a_planted_fallback_folder_stops_bind_and_connect() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = planted(tmp.path());
        let socket = dir.join("linksyd.sock");
        assert!(prepare_socket_dir(&socket).is_err());
        assert!(check_socket_dir(&socket).is_err());
        mode(&dir, 0o700);
        prepare_socket_dir(&socket).unwrap();
        check_socket_dir(&socket).unwrap();
    }

    #[test]
    fn a_missing_fallback_folder_is_left_to_the_connect() {
        let tmp = tempfile::tempdir().unwrap();
        check_socket_dir(&fallback_dir_in(tmp.path()).join("linksyd.sock")).unwrap();
        assert!(!fallback_dir_in(tmp.path()).exists());
    }
}
