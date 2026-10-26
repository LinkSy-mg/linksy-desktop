//! Light at rest (desktop#26): every few minutes the service gives back memory it isn't
//! using. The allocator's free pages go back to the system, and so does the program's own
//! code: the kernel maps all 16 MB of it in at start, though little runs while idle. What
//! runs again comes straight back from the page cache.

use std::path::Path;
use std::time::Duration;

pub async fn run() {
    tokio::time::sleep(Duration::from_secs(30)).await;
    loop {
        release();
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}

fn release() {
    // SAFETY: only hands the allocator's free memory back
    #[cfg(target_env = "gnu")]
    unsafe {
        libc::malloc_trim(0);
    }
    let (Ok(exe), Ok(smaps)) = (std::fs::read_link("/proc/self/exe"), std::fs::read_to_string("/proc/self/smaps")) else { return };
    for (start, end) in untouched(&smaps, &exe) {
        // SAFETY: these pages hold only the file's own bytes (see `untouched`), so the next
        // read maps the same bytes again
        unsafe {
            libc::madvise(start as *mut libc::c_void, end - start, libc::MADV_DONTNEED);
        }
    }
}

/// The program's own mappings that were never written: not writable, and no page of them
/// copied (relocated data is, and must stay).
fn untouched(smaps: &str, exe: &Path) -> Vec<(usize, usize)> {
    let exe = exe.to_string_lossy();
    // Each mapping: its range, whether it's the program's and read-only, its copied kB
    let mut maps: Vec<((usize, usize), bool, u64)> = Vec::new();
    for line in smaps.lines() {
        let mut fields = line.split_whitespace();
        let first = fields.next().unwrap_or_default();
        if let Some(range) = first.split_once('-').and_then(|(s, e)| Some((usize::from_str_radix(s, 16).ok()?, usize::from_str_radix(e, 16).ok()?))) {
            let perms = fields.next().unwrap_or_default();
            let path = fields.skip(3).collect::<Vec<_>>().join(" ");
            maps.push((range, path == exe && perms.as_bytes().get(1) == Some(&b'-') && perms.ends_with('p'), 0));
        } else if matches!(first, "Anonymous:" | "Swap:")
            && let Some(last) = maps.last_mut()
        {
            last.2 += fields.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(1);
        }
    }
    maps.into_iter().filter(|&(_, ours, copied)| ours && copied == 0).map(|(range, ..)| range).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMAPS: &str = "\
5600000000-5600100000 r--p 00000000 00:2e 123 /usr/bin/linksyd
Rss:                1024 kB
Anonymous:             0 kB
Swap:                  0 kB
5600100000-5601100000 r-xp 00100000 00:2e 123 /usr/bin/linksyd
Rss:               16000 kB
Anonymous:             0 kB
Swap:                  0 kB
5601100000-5601200000 r--p 01100000 00:2e 123 /usr/bin/linksyd
Rss:                 600 kB
Anonymous:           600 kB
Swap:                  0 kB
5601200000-5601210000 rw-p 01200000 00:2e 123 /usr/bin/linksyd
Anonymous:            64 kB
Swap:                  0 kB
7f0000000000-7f0000100000 r-xp 00000000 00:2e 456 /usr/lib64/libc.so.6
Anonymous:             0 kB
Swap:                  0 kB
";

    #[test]
    fn only_the_programs_unwritten_pages() {
        let found = untouched(SMAPS, Path::new("/usr/bin/linksyd"));
        assert_eq!(found, vec![(0x5600000000, 0x5600100000), (0x5600100000, 0x5601100000)]);
    }

    #[test]
    fn swapped_copies_stay() {
        let smaps = SMAPS.replacen("Swap:                  0 kB", "Swap:                  4 kB", 1);
        assert_eq!(untouched(&smaps, Path::new("/usr/bin/linksyd")), vec![(0x5600100000, 0x5601100000)]);
    }

    #[test]
    fn a_replaced_program_too() {
        let smaps = SMAPS.replace("/usr/bin/linksyd", "/usr/bin/linksyd (deleted)");
        assert_eq!(untouched(&smaps, Path::new("/usr/bin/linksyd (deleted)")).len(), 2);
    }
}
