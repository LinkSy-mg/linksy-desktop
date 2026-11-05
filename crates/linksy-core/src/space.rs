//! How much room there is where received files go.

use std::path::Path;

/// Bytes free for this user in the file system `path` is in (or would be created in: the
/// nearest folder that exists), or `None` when it can't be told.
pub fn free(path: &Path) -> Option<u64> {
    let mut at = path;
    while !at.exists() {
        at = at.parent()?;
    }
    free_at(at)
}

/// "812 MB", "2.1 GB": for a message, in the decimal units phones show.
pub fn show(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let (mut value, mut unit) = (bytes as f64, 0);
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} bytes"),
        _ if value < 10.0 => format!("{value:.1} {}", UNITS[unit]),
        _ => format!("{value:.0} {}", UNITS[unit]),
    }
}

#[cfg(unix)]
fn free_at(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: `c` is a valid C string and `stat` is a statvfs to fill in
    if unsafe { libc::statvfs(c.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statvfs succeeded, so it is filled in
    let stat = unsafe { stat.assume_init() };
    // 32-bit targets (armeabi-v7a) have these as u32
    #[allow(clippy::unnecessary_cast)]
    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

#[cfg(not(unix))]
fn free_at(_: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_phones_show_them() {
        assert_eq!(show(0), "0 bytes");
        assert_eq!(show(999), "999 bytes");
        assert_eq!(show(1_500), "1.5 KB");
        assert_eq!(show(812_000_000), "812 MB");
        assert_eq!(show(2_100_000_000), "2.1 GB");
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_does_not_exist_yet_counts_for_the_one_it_will_be_in() {
        let dir = tempfile::tempdir().unwrap();
        let there = free(dir.path()).unwrap();
        assert!(there > 0);
        let later = free(&dir.path().join("not/yet/made")).unwrap();
        // The same file system, give or take what else is writing to it
        assert!(there.abs_diff(later) < 1 << 30);
    }
}
