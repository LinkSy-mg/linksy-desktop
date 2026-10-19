//! File transfer over one QUIC bidirectional stream.
//!
//! Sender: Offer → (Accept{offsets} | Decline) → for each file: raw bytes from offset,
//! then FileDone{blake3 of the whole file} → receiver answers Complete or Failed.
//!
//! The receiver writes to `<name>.linksy-part` and renames only after the hash matches,
//! so a half-received file is never mistaken for a complete one. An existing `.linksy-part`
//! is resumed from its current length; the whole-file hash catches a stale partial.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{Error, Result, protocol};
use crate::protocol::{FileEntry, TransferMsg, read_msg, write_msg};

const CHUNK: usize = 256 * 1024;
const PART_SUFFIX: &str = ".linksy-part";
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

pub struct OutgoingFile {
    pub entry: FileEntry,
    pub source: PathBuf,
}

/// Expand files and folders into a flat list. Folders keep their name as the top-level
/// component. Symlinks are skipped. Blocking; run on a blocking thread.
pub fn collect(paths: &[PathBuf]) -> io::Result<Vec<OutgoingFile>> {
    let mut out = Vec::new();
    for p in paths {
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported file name: {}", p.display())))?
            .to_string();
        walk(p, name, &mut out)?;
    }
    Ok(out)
}

fn walk(path: &Path, rel: String, out: &mut Vec<OutgoingFile>) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_file() {
        out.push(OutgoingFile { entry: FileEntry { path: rel, size: meta.len() }, source: path.to_path_buf() });
    } else if meta.is_dir() {
        let mut children: Vec<_> = std::fs::read_dir(path)?.collect::<io::Result<_>>()?;
        children.sort_by_key(|e| e.file_name());
        for child in children {
            if let Some(name) = child.file_name().to_str() {
                walk(&child.path(), format!("{rel}/{name}"), out)?;
            }
        }
    }
    Ok(())
}

/// Throttled progress reporter: `bytes done` out of the transfer total.
pub(crate) struct Progress<'a> {
    done: u64,
    last: Instant,
    report: &'a mut (dyn FnMut(u64) + Send),
}

impl<'a> Progress<'a> {
    pub(crate) fn new(start: u64, report: &'a mut (dyn FnMut(u64) + Send)) -> Self {
        report(start);
        Self { done: start, last: Instant::now(), report }
    }

    fn add(&mut self, n: u64) {
        self.done += n;
        if self.last.elapsed() >= PROGRESS_EVERY {
            self.last = Instant::now();
            (self.report)(self.done);
        }
    }

    fn flush(&mut self) {
        (self.report)(self.done);
    }
}

pub(crate) async fn send_files(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    files: &[OutgoingFile],
    offsets: &[u64],
    progress: &mut Progress<'_>,
) -> Result<()> {
    if offsets.len() != files.len() {
        return Err(protocol("receiver answered with the wrong number of offsets"));
    }
    let mut buf = vec![0u8; CHUNK];
    for (f, &offset) in files.iter().zip(offsets) {
        if offset > f.entry.size {
            return Err(protocol("resume offset beyond end of file"));
        }
        let mut file = File::open(&f.source).await?;
        let mut hasher = blake3::Hasher::new();
        // Hash the part the receiver already has without sending it.
        let mut skip = offset;
        while skip > 0 {
            let n = file.read(&mut buf[..(skip as usize).min(CHUNK)]).await?;
            if n == 0 {
                return Err(Error::Io(io::ErrorKind::UnexpectedEof.into()));
            }
            hasher.update(&buf[..n]);
            skip -= n as u64;
        }
        let mut left = f.entry.size - offset;
        while left > 0 {
            let n = file.read(&mut buf[..(left as usize).min(CHUNK)]).await?;
            if n == 0 {
                return Err(protocol(format!("{} shrank while sending", f.entry.path)));
            }
            hasher.update(&buf[..n]);
            send.write_all(&buf[..n]).await?;
            left -= n as u64;
            progress.add(n as u64);
        }
        write_msg(send, &TransferMsg::FileDone { blake3: hasher.finalize().to_hex().to_string() }).await?;
    }
    send.finish()?;
    progress.flush();
    match read_msg(recv).await? {
        TransferMsg::Complete => Ok(()),
        TransferMsg::Failed { reason } => Err(Error::Integrity(reason)),
        other => Err(protocol(format!("unexpected {other:?}"))),
    }
}

pub struct IncomingFile {
    pub entry: FileEntry,
    pub part: PathBuf,
    pub dest: PathBuf,
    pub offset: u64,
}

/// Decide where each offered file goes and how much of it is already here.
pub async fn plan(dir: &Path, files: &[FileEntry]) -> Result<Vec<IncomingFile>> {
    let mut out = Vec::with_capacity(files.len());
    for entry in files {
        let rel = sanitize(&entry.path).ok_or_else(|| protocol(format!("refusing unsafe path {:?}", entry.path)))?;
        let mut dest = dir.join(&rel);
        if tokio::fs::try_exists(&dest).await? {
            dest = unique(&dest).await?;
        }
        let part = part_path(&dest);
        let offset = match tokio::fs::metadata(&part).await {
            Ok(m) if m.len() <= entry.size => m.len(),
            _ => 0,
        };
        out.push(IncomingFile { entry: entry.clone(), part, dest, offset });
    }
    Ok(out)
}

pub(crate) async fn receive_files(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    plan: &[IncomingFile],
    progress: &mut Progress<'_>,
) -> Result<Vec<PathBuf>> {
    let mut buf = vec![0u8; CHUNK];
    let mut saved = Vec::with_capacity(plan.len());
    for f in plan {
        if let Some(parent) = f.part.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut hasher = blake3::Hasher::new();
        let mut file = OpenOptions::new().create(true).read(true).append(true).open(&f.part).await?;
        if f.offset > 0 {
            let mut left = f.offset;
            while left > 0 {
                let n = file.read(&mut buf[..(left as usize).min(CHUNK)]).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                left -= n as u64;
            }
        }
        let mut left = f.entry.size - f.offset;
        while left > 0 {
            let want = (left as usize).min(CHUNK);
            let n = recv
                .read(&mut buf[..want])
                .await
                .map_err(|e| Error::Protocol(e.to_string()))?
                .ok_or_else(|| protocol("sender closed the stream early"))?;
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).await?;
            left -= n as u64;
            progress.add(n as u64);
        }
        file.flush().await?;
        file.sync_data().await?;
        drop(file);

        let expected = match read_msg(recv).await? {
            TransferMsg::FileDone { blake3 } => blake3,
            other => return Err(protocol(format!("unexpected {other:?}"))),
        };
        if hasher.finalize().to_hex().as_str() != expected {
            let _ = tokio::fs::remove_file(&f.part).await;
            let reason = format!("{} did not match its checksum", f.entry.path);
            write_msg(send, &TransferMsg::Failed { reason: reason.clone() }).await?;
            let _ = send.finish();
            return Err(Error::Integrity(reason));
        }
        tokio::fs::rename(&f.part, &f.dest).await?;
        saved.push(f.dest.clone());
    }
    progress.flush();
    write_msg(send, &TransferMsg::Complete).await?;
    send.finish()?;
    Ok(saved)
}

/// Accept only plain relative paths: no root, no `..`, no drive letters or backslashes.
fn sanitize(path: &str) -> Option<PathBuf> {
    if path.is_empty() || path.contains('\\') || path.contains(':') || path.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in Path::new(path).components() {
        match c {
            Component::Normal(s) => out.push(s),
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(PART_SUFFIX);
    dest.with_file_name(name)
}

/// "photo.jpg" → "photo (1).jpg", "photo (2).jpg", …
// If you ever reach "photo (47).jpg", we should talk about your Downloads folder.
async fn unique(dest: &Path) -> Result<PathBuf> {
    let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = dest.extension().and_then(|s| s.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    for i in 1.. {
        let candidate = dest.with_file_name(format!("{stem} ({i}){ext}"));
        if !tokio::fs::try_exists(&candidate).await? {
            return Ok(candidate);
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_escapes() {
        assert_eq!(sanitize("a/b.txt"), Some(PathBuf::from("a/b.txt")));
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "./a", "C:\\x", "a\\b", "c:/x"] {
            assert_eq!(sanitize(bad), None, "{bad}");
        }
    }

    #[tokio::test]
    async fn plan_picks_unique_name_and_resume_offset() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"old").unwrap();
        std::fs::write(dir.path().join("b.bin.linksy-part"), b"12345").unwrap();
        let files = vec![
            FileEntry { path: "a.txt".into(), size: 3 },
            FileEntry { path: "b.bin".into(), size: 10 },
        ];
        let p = plan(dir.path(), &files).await.unwrap();
        assert_eq!(p[0].dest, dir.path().join("a (1).txt"));
        assert_eq!(p[0].offset, 0);
        assert_eq!(p[1].dest, dir.path().join("b.bin"));
        assert_eq!(p[1].offset, 5);
    }

    #[test]
    fn collect_walks_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Photos");
        std::fs::create_dir_all(root.join("2026")).unwrap();
        std::fs::write(root.join("a.jpg"), b"aa").unwrap();
        std::fs::write(root.join("2026/b.jpg"), b"bbb").unwrap();
        let files = collect(&[root]).unwrap();
        let entries: Vec<_> = files.iter().map(|f| (f.entry.path.as_str(), f.entry.size)).collect();
        assert_eq!(entries, vec![("Photos/2026/b.jpg", 3), ("Photos/a.jpg", 2)]);
    }
}
