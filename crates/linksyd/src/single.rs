//! One service per user. A lock on a file beside the IPC socket, held for as long as the
//! service runs, decides it: two services started together (a login that runs two autostart
//! entries) both find no socket yet, and only one gets the lock.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;

/// Another service holds the lock.
#[derive(Debug, PartialEq)]
pub struct Taken;

/// The lock for the service that owns `socket`: keep the file until the service ends. `None`
/// when no lock could be made at all (a runtime folder that can't be written to): the service
/// then starts as it always did, with only the socket to tell it from another.
pub fn take(socket: &Path) -> Result<Option<File>, Taken> {
    let path = socket.with_extension("lock");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(file) = File::options().create(true).truncate(false).write(true).open(&path) else { return Ok(None) };
    // SAFETY: the descriptor is open for as long as `file` is
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(file));
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EWOULDBLOCK) => Err(Taken),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_service_does_not_get_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("linksyd.sock");
        let first = take(&socket).unwrap();
        assert!(first.is_some());
        assert_eq!(take(&socket).unwrap_err(), Taken);
        drop(first);
        assert!(take(&socket).unwrap().is_some());
    }

    #[test]
    fn services_on_other_sockets_do_not_meet() {
        let dir = tempfile::tempdir().unwrap();
        let a = take(&dir.path().join("a.sock")).unwrap();
        let b = take(&dir.path().join("b.sock")).unwrap();
        assert!(a.is_some() && b.is_some());
    }

    #[test]
    fn a_folder_that_cannot_be_written_to_does_not_stop_the_service() {
        assert!(take(Path::new("/proc/linksy/linksyd.sock")).unwrap().is_none());
    }
}
