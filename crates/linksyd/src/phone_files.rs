//! Copies between this computer and a phone's files (docs/protocol.md §4.9), in the
//! background: out of the phone into the download folder (in a folder named after the phone,
//! like imported photos), or into one of the phone's folders. Nothing is overwritten on
//! either side: a name that's taken becomes "name (1).ext". The apps follow each copy as
//! [`FsProgress`]; finished ones go in the history.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use linksy_core::fs::FsAnswer;
use linksy_core::{DeviceView, Error, Fingerprint, FsEntry, FsRequest, Node, Uuid};
use linksy_ipc::{FsProgress, Shared};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use crate::history::History;

pub static FILES: OnceLock<PhoneFiles> = OnceLock::new();

/// Each read from the phone, and each write to it.
const CHUNK: u64 = 1 << 20;
/// Reads asked for at once, so the phone's answers overlap the round trips.
const AHEAD: usize = 4;
/// Progress at most this often per copy, and once at the end.
const REPORT_EVERY: Duration = Duration::from_millis(200);
const PART_SUFFIX: &str = ".linksy-part";

#[derive(Clone)]
pub struct PhoneFiles {
    node: Node,
    history: History,
    running: Arc<Mutex<HashMap<Uuid, watch::Sender<bool>>>>,
    /// Phone paths uploads are writing (lowercase: phones' storage ignores case), so two at
    /// once into one folder pick different names.
    claimed: Arc<Mutex<HashSet<(Fingerprint, String)>>>,
    progress: broadcast::Sender<FsProgress>,
}

enum Job {
    Copy(Vec<String>),
    Upload(Vec<PathBuf>),
}

impl PhoneFiles {
    pub fn new(node: Node, history: History) -> Self {
        let (progress, _) = broadcast::channel(256);
        Self { node, history, running: Arc::default(), claimed: Arc::default(), progress }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<FsProgress> {
        self.progress.subscribe()
    }

    /// Copy files and folders from the phone into the download folder; answers at once.
    pub fn copy(&self, phone: Fingerprint, paths: Vec<String>) -> Result<Uuid, String> {
        let paths = paths.iter().map(|p| phone_path(p)).collect::<Result<Vec<_>, _>>()?;
        if paths.is_empty() || paths.iter().any(|p| p == "/") {
            return Err("pick files or folders in the phone's storage".into());
        }
        Ok(self.start(phone, None, Job::Copy(paths)))
    }

    /// Copy files and folders from here into the phone's folder `dir`; answers at once.
    pub fn upload(&self, phone: Fingerprint, dir: &str, paths: Vec<PathBuf>) -> Result<Uuid, String> {
        let dir = phone_path(dir)?;
        if paths.is_empty() {
            return Err("pick files or folders to upload".into());
        }
        Ok(self.start(phone, Some(dir), Job::Upload(paths)))
    }

    /// Stop a copy; answers whether it was running.
    pub fn cancel(&self, op: Uuid) -> bool {
        self.running.lock().unwrap().get(&op).map(|stop| stop.send_replace(true)).is_some()
    }

    pub async fn mkdir(&self, phone: Fingerprint, path: &str) -> Result<(), String> {
        let path = phone_path(path)?;
        let name = split(&path).1;
        check_name(name)?;
        if self.exists(phone, &path).await {
            return Err(format!("\"{name}\" is already there"));
        }
        self.ask(phone, FsRequest::Mkdir { path }).await.map(drop)
    }

    /// Rename an item in its folder; answers its new path.
    pub async fn rename(&self, phone: Fingerprint, path: &str, to: &str) -> Result<String, String> {
        let path = phone_path(path)?;
        if path == "/" {
            return Err("the phone's storage can't be renamed".into());
        }
        check_name(to)?;
        let (dir, name) = split(&path);
        let target = join(dir, to);
        if target == path {
            return Ok(target);
        }
        // Only the case changes: the same file, to a phone's storage
        if name.to_lowercase() != to.to_lowercase() && self.exists(phone, &target).await {
            return Err(format!("\"{to}\" is already there"));
        }
        self.ask(phone, FsRequest::Rename { path, to: target.clone() }).await?;
        Ok(target)
    }

    /// Delete files and folders; the phone refusing one doesn't keep the others.
    pub async fn delete(&self, phone: Fingerprint, paths: Vec<String>) -> Result<(), String> {
        let paths = paths.iter().map(|p| phone_path(p)).collect::<Result<Vec<_>, _>>()?;
        if paths.iter().any(|p| p == "/") {
            return Err("the phone's storage itself can't be deleted".into());
        }
        let mut refused = None;
        for path in paths {
            match self.node.fs(&phone, FsRequest::Delete { path }, &[]).await {
                Ok(_) | Err(Error::NotFound(_)) => {}
                Err(Error::Invalid(m)) => {
                    refused.get_or_insert(m);
                }
                Err(e) => return Err(self.reason(phone, e)),
            }
        }
        refused.map_or(Ok(()), Err)
    }

    fn start(&self, phone: Fingerprint, dir: Option<String>, job: Job) -> Uuid {
        let id = Uuid::new_v4();
        let (stop_tx, stop) = watch::channel(false);
        self.running.lock().unwrap().insert(id, stop_tx);
        let name = match &job {
            Job::Copy(paths) => split(&paths[0]).1.to_string(),
            Job::Upload(paths) => paths[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        let mut op = Op { files: self.clone(), id, phone, dir, stop, name, done: 0, total: 0, saved: Vec::new(), copied: Vec::new(), landed: Vec::new(), writing: None, claims: Vec::new(), reported: None };
        tokio::spawn(async move {
            op.report(true);
            let result = match job {
                Job::Copy(paths) => op.copy(paths).await,
                Job::Upload(paths) => op.upload(paths).await,
            };
            let error = match result {
                Ok(()) => {
                    op.record();
                    None
                }
                Err(fail) => {
                    op.clean_up().await;
                    Some(op.files.failure(phone, fail))
                }
            };
            op.release();
            let _ = op.files.progress.send(op.progress(true, error));
        });
        id
    }

    async fn ask(&self, phone: Fingerprint, request: FsRequest) -> Result<FsAnswer, String> {
        self.node.fs(&phone, request, &[]).await.map_err(|e| self.reason(phone, e))
    }

    async fn exists(&self, phone: Fingerprint, path: &str) -> bool {
        matches!(self.node.fs(&phone, FsRequest::Stat { path: path.into() }, &[]).await, Ok(FsAnswer::Entry(_)))
    }

    fn device(&self, phone: &Fingerprint) -> Option<DeviceView> {
        self.node.devices().into_iter().find(|d| d.device.id == *phone)
    }

    /// What went wrong with the phone, for the person: its own words when it refused.
    fn reason(&self, phone: Fingerprint, e: Error) -> String {
        let name = self.device(&phone).map(|d| (d.device.name, d.connected));
        match (e, name) {
            (Error::NotFound(m) | Error::Invalid(m), _) => m,
            (Error::Unreachable | Error::Connect(_), Some((name, _))) => format!("{name} isn't reachable right now"),
            (Error::Connection(_) | Error::Read(_) | Error::Write(_) | Error::Closed(_), Some((name, _))) | (_, Some((name, false))) => format!("{name} disconnected"),
            (e, _) => e.to_string(),
        }
    }

    fn failure(&self, phone: Fingerprint, fail: Fail) -> String {
        match fail {
            Fail::Stopped => "cancelled".into(),
            Fail::Phone(e) => self.reason(phone, e),
            Fail::Local(path, e) => {
                let text = e.to_string();
                format!("{}: {}", path.display(), text.split(" (os error").next().unwrap_or(&text))
            }
            Fail::Other(m) => m,
        }
    }
}

enum Fail {
    Stopped,
    Phone(Error),
    Local(PathBuf, std::io::Error),
    Other(String),
}

/// One copy as it runs.
struct Op {
    files: PhoneFiles,
    id: Uuid,
    phone: Fingerprint,
    /// Uploads: the phone's folder they go into.
    dir: Option<String>,
    stop: watch::Receiver<bool>,
    name: String,
    done: u64,
    total: u64,
    /// Each item asked for, where it went (copies).
    saved: Vec<PathBuf>,
    /// Each file done, as the history lists it, and its size; and where it went (copies).
    copied: Vec<(String, u64)>,
    landed: Vec<PathBuf>,
    /// The phone file an upload is writing, to delete if it stops halfway.
    writing: Option<String>,
    claims: Vec<String>,
    reported: Option<Instant>,
}

/// Something on the phone to copy, and where it goes in the copy.
struct Item {
    path: String,
    rel: PathBuf,
    entry: FsEntry,
}

/// Something here to upload; `rel` is its path in the upload, with `/`.
struct Local {
    path: PathBuf,
    rel: String,
    dir: bool,
    size: u64,
}

impl Op {
    async fn copy(&mut self, paths: Vec<String>) -> Result<(), Fail> {
        let mut plan = Vec::with_capacity(paths.len());
        for path in paths {
            let entry = self.stat(&path).await?;
            let mut inside = Vec::new();
            if entry.dir {
                self.walk(&path, &mut inside).await?;
            } else {
                self.total += entry.size;
            }
            plan.push((path, entry, inside));
        }
        let phone_name = self.files.device(&self.phone).map(|d| d.device.name).unwrap_or_default();
        let base = self.files.node.settings().download_dir.join(linksy_core::folder_name(&phone_name));
        tokio::fs::create_dir_all(&base).await.map_err(|e| Fail::Local(base.clone(), e))?;
        self.report(true);
        for (path, entry, inside) in plan {
            let name = split(&path).1;
            if !entry.dir {
                let saved = self.fetch(&path, &entry, &base.join(name), None).await?;
                self.saved.push(saved);
                continue;
            }
            let top = new_folder(&base, name).await?;
            self.saved.push(top.clone());
            let label = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for item in inside {
                let dest = top.join(&item.rel);
                if item.entry.dir {
                    tokio::fs::create_dir_all(&dest).await.map_err(|e| Fail::Local(dest, e))?;
                } else {
                    let label = format!("{label}/{}", item.rel.to_string_lossy());
                    self.fetch(&item.path, &item.entry, &dest, Some(label)).await?;
                }
            }
        }
        Ok(())
    }

    /// Everything in a phone folder, each folder before what's in it.
    async fn walk(&mut self, path: &str, out: &mut Vec<Item>) -> Result<(), Fail> {
        let mut folders = vec![(path.to_string(), PathBuf::new())];
        while let Some((path, rel)) = folders.pop() {
            for entry in self.list(&path).await? {
                // A name that would land outside the copy
                if check_name(&entry.name).is_err() {
                    continue;
                }
                let (path, rel) = (join(&path, &entry.name), rel.join(&entry.name));
                if entry.dir {
                    folders.push((path.clone(), rel.clone()));
                } else {
                    self.total += entry.size;
                }
                out.push(Item { path, rel, entry });
            }
        }
        Ok(())
    }

    /// One file from the phone to `want` (or the first free name like it), through a part
    /// file renamed once it's all there. `label`: its path in the copy, for the history.
    async fn fetch(&mut self, path: &str, entry: &FsEntry, want: &Path, label: Option<String>) -> Result<PathBuf, Fail> {
        let (dest, mut part, mut file) = claim(want).await?;
        self.name = split(path).1.to_string();
        let size = entry.size;
        let mut ahead = Ahead::default();
        let (mut asked, mut at) = (0, 0);
        while at < size {
            while ahead.0.len() < AHEAD && asked < size {
                let (node, phone, path, length) = (self.files.node.clone(), self.phone, path.to_string(), CHUNK.min(size - asked));
                let offset = asked;
                ahead.0.push_back(tokio::spawn(async move { node.fs(&phone, FsRequest::Read { path, offset, length }, &[]).await }));
                asked += length;
            }
            let want = CHUNK.min(size - at);
            let next = ahead.0.front_mut().expect("a read asked for");
            let answer = tokio::select! {
                r = next => r.map_err(|e| Fail::Other(e.to_string()))?.map_err(Fail::Phone)?,
                _ = stopped(&mut self.stop) => return Err(Fail::Stopped),
            };
            ahead.0.pop_front();
            let data = match answer {
                FsAnswer::Data(d) if d.len() as u64 == want => d,
                FsAnswer::Data(_) => return Err(Fail::Other(format!("{} changed on the phone while it was copied", entry.name))),
                _ => return Err(Fail::Other("the phone answered something else".into())),
            };
            file.write_all(&data).await.map_err(|e| Fail::Local(dest.clone(), e))?;
            at += want;
            self.done += want;
            self.report(false);
        }
        file.flush().await.map_err(|e| Fail::Local(dest.clone(), e))?;
        file.sync_data().await.map_err(|e| Fail::Local(dest.clone(), e))?;
        if entry.modified_ms > 0 {
            let _ = file.into_std().await.set_modified(UNIX_EPOCH + Duration::from_millis(entry.modified_ms));
        }
        // Never over a file that turned up meanwhile
        let dest = match tokio::fs::try_exists(&dest).await {
            Ok(false) => dest,
            _ => claim(want).await?.0,
        };
        let from = part.0.take().expect("the part file");
        if let Err(e) = tokio::fs::rename(&from, &dest).await {
            part.0 = Some(from);
            return Err(Fail::Local(dest, e));
        }
        let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.copied.push((label.unwrap_or(name), size));
        self.landed.push(dest.clone());
        Ok(dest)
    }

    async fn upload(&mut self, paths: Vec<PathBuf>) -> Result<(), Fail> {
        let dir = self.dir.clone().unwrap_or_else(|| "/".into());
        let plan = tokio::task::spawn_blocking(move || local_plan(&paths)).await.map_err(|e| Fail::Other(e.to_string()))??;
        self.total = plan.iter().flat_map(|(top, inside)| std::iter::once(top).chain(inside)).map(|l| l.size).sum();
        // What's in the folder already, as the phone compares names
        let mut taken: HashSet<String> = self.list(&dir).await?.into_iter().map(|e| e.name.to_lowercase()).collect();
        self.report(true);
        let mut buf = vec![0u8; CHUNK as usize];
        for (top, inside) in plan {
            let name = self.free_name(&dir, &top.rel, top.dir, &mut taken);
            let target = join(&dir, &name);
            if top.dir {
                self.ask(FsRequest::Mkdir { path: target.clone() }, &[]).await?;
            } else {
                self.put(&top.path, &target, &mut buf).await?;
                self.copied.push((name.clone(), top.size));
            }
            for item in inside {
                let path = format!("{target}/{}", item.rel);
                if item.dir {
                    self.ask(FsRequest::Mkdir { path }, &[]).await?;
                } else {
                    self.put(&item.path, &path, &mut buf).await?;
                    self.copied.push((format!("{name}/{}", item.rel), item.size));
                }
            }
        }
        Ok(())
    }

    /// `name`, or the first "name (1).ext"… neither in the folder nor being uploaded to it.
    fn free_name(&mut self, dir: &str, name: &str, folder: bool, taken: &mut HashSet<String>) -> String {
        for i in 0.. {
            let candidate = numbered(name, i, folder);
            let path = join(dir, &candidate).to_lowercase();
            if !taken.insert(candidate.to_lowercase()) || !self.files.claimed.lock().unwrap().insert((self.phone, path.clone())) {
                continue;
            }
            self.claims.push(path);
            return candidate;
        }
        unreachable!()
    }

    /// One file from here to the phone's `path`, a chunk at a time (the first truncates).
    async fn put(&mut self, local: &Path, path: &str, buf: &mut [u8]) -> Result<(), Fail> {
        let mut file = tokio::fs::File::open(local).await.map_err(|e| Fail::Local(local.into(), e))?;
        self.name = split(path).1.to_string();
        self.writing = Some(path.to_string());
        let mut at = 0u64;
        loop {
            let n = fill(&mut file, buf).await.map_err(|e| Fail::Local(local.into(), e))?;
            if n == 0 && at > 0 {
                break;
            }
            self.ask(FsRequest::Write { path: path.into(), offset: at, length: n as u64, truncate: at == 0 }, &buf[..n]).await?;
            at += n as u64;
            self.done += n as u64;
            self.report(false);
            if n < buf.len() {
                break;
            }
        }
        self.writing = None;
        Ok(())
    }

    async fn ask(&mut self, request: FsRequest, body: &[u8]) -> Result<FsAnswer, Fail> {
        tokio::select! {
            r = self.files.node.fs(&self.phone, request, body) => r.map_err(Fail::Phone),
            _ = stopped(&mut self.stop) => Err(Fail::Stopped),
        }
    }

    async fn stat(&mut self, path: &str) -> Result<FsEntry, Fail> {
        match self.ask(FsRequest::Stat { path: path.into() }, &[]).await? {
            FsAnswer::Entry(e) => Ok(e),
            _ => Err(Fail::Other("the phone answered something else".into())),
        }
    }

    async fn list(&mut self, path: &str) -> Result<Vec<FsEntry>, Fail> {
        match self.ask(FsRequest::List { path: path.into() }, &[]).await? {
            FsAnswer::Entries(e) => Ok(e),
            _ => Err(Fail::Other("the phone answered something else".into())),
        }
    }

    /// An upload stopped halfway: the phone keeps no half file. (A copy's part file went
    /// with it already.)
    async fn clean_up(&mut self) {
        if let Some(path) = self.writing.take() {
            let delete = self.files.node.fs(&self.phone, FsRequest::Delete { path }, &[]);
            let _ = tokio::time::timeout(Duration::from_secs(10), delete).await;
        }
    }

    fn release(&mut self) {
        self.files.running.lock().unwrap().remove(&self.id);
        let mut claimed = self.files.claimed.lock().unwrap();
        for path in self.claims.drain(..) {
            claimed.remove(&(self.phone, path));
        }
    }

    /// A finished copy goes in the history: received, or sent for an upload.
    fn record(&mut self) {
        let Some(device) = self.files.device(&self.phone).map(|d| d.device) else { return };
        if self.copied.is_empty() {
            return;
        }
        let (files, sizes): (Vec<String>, Vec<u64>) = std::mem::take(&mut self.copied).into_iter().unzip();
        let total = sizes.iter().sum();
        let saved = std::mem::take(&mut self.landed);
        let what = Shared::Files { files, sizes, total, saved, import: false, more: 0 };
        match self.dir {
            None => self.files.history.record_incoming(&device, what),
            Some(_) => self.files.history.record_sent(&device, what),
        }
    }

    fn report(&mut self, now: bool) {
        if !now && self.reported.is_some_and(|t| t.elapsed() < REPORT_EVERY) {
            return;
        }
        self.reported = Some(Instant::now());
        let _ = self.files.progress.send(self.progress(false, None));
    }

    fn progress(&self, finished: bool, error: Option<String>) -> FsProgress {
        FsProgress {
            op: self.id,
            phone: self.phone,
            name: self.name.clone(),
            done: self.done,
            total: self.total.max(self.done),
            finished,
            error,
            saved: if finished { self.saved.clone() } else { Vec::new() },
            dir: self.dir.clone(),
        }
    }
}

async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

/// A part file that goes unless it was finished.
struct Part(Option<PathBuf>);

impl Drop for Part {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Reads asked for ahead; any still going when a copy stops are dropped with it.
#[derive(Default)]
struct Ahead(VecDeque<JoinHandle<linksy_core::Result<FsAnswer>>>);

impl Drop for Ahead {
    fn drop(&mut self) {
        for read in &self.0 {
            read.abort();
        }
    }
}

/// `want`, or the first of "name (1).ext", "name (2).ext"… free here, with its part file
/// made (so a copy running beside this one picks another).
async fn claim(want: &Path) -> Result<(PathBuf, Part, tokio::fs::File), Fail> {
    let name = want.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for i in 0.. {
        let file_name = numbered(&name, i, false);
        let (dest, part) = (want.with_file_name(&file_name), want.with_file_name(format!("{file_name}{PART_SUFFIX}")));
        if tokio::fs::try_exists(&dest).await.map_err(|e| Fail::Local(dest.clone(), e))? {
            continue;
        }
        match tokio::fs::OpenOptions::new().write(true).create_new(true).open(&part).await {
            Ok(file) => return Ok((dest, Part(Some(part)), file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Fail::Local(dest, e)),
        }
    }
    unreachable!()
}

/// `dir/name`, or the first "name (1)"… free, made empty.
async fn new_folder(dir: &Path, name: &str) -> Result<PathBuf, Fail> {
    for i in 0.. {
        let path = dir.join(numbered(name, i, true));
        match tokio::fs::create_dir(&path).await {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Fail::Local(path, e)),
        }
    }
    unreachable!()
}

/// "photo.jpg", then "photo (1).jpg", "photo (2).jpg"… ("Photos (1)" for a folder).
fn numbered(name: &str, i: u32, folder: bool) -> String {
    if i == 0 {
        return name.into();
    }
    match name.rsplit_once('.').filter(|(stem, _)| !folder && !stem.is_empty()) {
        Some((stem, ext)) => format!("{stem} ({i}).{ext}"),
        None => format!("{name} ({i})"),
    }
}

/// What an upload sends: each path (a link chosen itself is followed) and, in folders,
/// everything in them, each folder before what's in it. Links inside folders are skipped,
/// as when sending files.
fn local_plan(paths: &[PathBuf]) -> Result<Vec<(Local, Vec<Local>)>, Fail> {
    let mut plan = Vec::with_capacity(paths.len());
    for path in paths {
        let meta = std::fs::metadata(path).map_err(|e| Fail::Local(path.clone(), e))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !(meta.is_dir() || meta.is_file()) || check_name(&name).is_err() {
            return Err(Fail::Other(format!("{} isn't a file or folder that can be copied", path.display())));
        }
        let mut inside = Vec::new();
        let mut folders = if meta.is_dir() { vec![(path.clone(), String::new())] } else { Vec::new() };
        while let Some((dir, rel)) = folders.pop() {
            for entry in std::fs::read_dir(&dir).map_err(|e| Fail::Local(dir.clone(), e))? {
                let entry = entry.map_err(|e| Fail::Local(dir.clone(), e))?;
                let Ok(kind) = entry.file_type() else { continue };
                let name = entry.file_name().to_string_lossy().into_owned();
                let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
                if kind.is_dir() {
                    folders.push((entry.path(), rel.clone()));
                    inside.push(Local { path: entry.path(), rel, dir: true, size: 0 });
                } else if kind.is_file() {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    inside.push(Local { path: entry.path(), rel, dir: false, size });
                }
            }
        }
        let size = if meta.is_dir() { 0 } else { meta.len() };
        plan.push((Local { path: path.clone(), rel: name, dir: meta.is_dir(), size }, inside));
    }
    Ok(plan)
}

/// Read until `buf` is full or the file ends.
async fn fill(file: &mut tokio::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match file.read(&mut buf[n..]).await? {
            0 => break,
            read => n += read,
        }
    }
    Ok(n)
}

/// A path in what the phone shares: "/" first, no "." or "..", no empty parts.
fn phone_path(path: &str) -> Result<String, String> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" => {}
            "." | ".." => return Err(format!("\"{path}\" isn't in the phone's storage")),
            p if p.contains('\0') => return Err(format!("\"{path}\" isn't in the phone's storage")),
            p => parts.push(p),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

fn check_name(name: &str) -> Result<(), String> {
    match name {
        n if n.trim().is_empty() => Err("a name is needed".into()),
        "." | ".." => Err(format!("\"{name}\" can't be a name")),
        n if n.contains('/') || n.contains('\0') => Err("a name can't have \"/\" in it".into()),
        _ => Ok(()),
    }
}

/// "/a/b" → ("/a", "b"); "/a" → ("/", "a").
fn split(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some(("", name)) => ("/", name),
        Some(parts) => parts,
        None => ("/", path),
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use linksy_core::fs::FsCall;
    use linksy_core::{Config, Event};
    use std::net::{IpAddr, Ipv4Addr};

    /// A phone called Pixel sharing `storage`, paired with this computer.
    struct Setup {
        dir: tempfile::TempDir,
        phone: Node,
        phone_id: Fingerprint,
        storage: PathBuf,
        out: PathBuf,
        files: PhoneFiles,
        history: History,
    }

    async fn node(dir: &Path, name: &str) -> Node {
        let mut c = Config::new(dir.join("data"), dir.join("downloads"), name);
        c.bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
        c.discovery = false;
        let node = Node::start(c).await.unwrap();
        let (mut events, n) = (node.subscribe(), node.clone());
        tokio::spawn(async move {
            while let Ok(e) = events.recv().await {
                if let Event::PairingRequest { request, .. } = e {
                    n.respond(request, true);
                }
            }
        });
        node
    }

    impl Setup {
        /// `slow`: the phone waits that long before each answer.
        async fn new(slow: Duration) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let phone = node(&dir.path().join("phone"), "Pixel").await;
            let pc = node(&dir.path().join("pc"), "Desk").await;
            let mut connected = phone.subscribe();
            pc.pair(phone.local_addr().unwrap()).await.unwrap();
            tokio::time::timeout(Duration::from_secs(20), async {
                while !matches!(connected.recv().await, Ok(Event::Connected { .. })) {}
            })
            .await
            .unwrap();
            let storage = dir.path().join("storage");
            std::fs::create_dir_all(&storage).unwrap();
            serve(&phone, storage.clone(), slow);
            let history = History::load(dir.path().join("history.json"));
            let out = dir.path().join("pc/downloads/Pixel");
            Self { phone_id: phone.info().id, phone, storage, out, files: PhoneFiles::new(pc, history.clone()), history, dir }
        }

        async fn copy(&self, paths: &[&str]) -> FsProgress {
            let mut progress = self.files.subscribe();
            let op = self.files.copy(self.phone_id, paths.iter().map(|p| p.to_string()).collect()).unwrap();
            finished(&mut progress, op).await
        }

        async fn upload(&self, dir: &str, paths: &[PathBuf]) -> FsProgress {
            let mut progress = self.files.subscribe();
            let op = self.files.upload(self.phone_id, dir, paths.to_vec()).unwrap();
            finished(&mut progress, op).await
        }

        /// Start a copy (or upload), and stop it once some of it went.
        async fn stop_halfway(&self, start: impl FnOnce(&PhoneFiles) -> Uuid, then: impl AsyncFnOnce()) -> FsProgress {
            let mut progress = self.files.subscribe();
            let op = start(&self.files);
            tokio::time::timeout(Duration::from_secs(20), async {
                while !matches!(progress.recv().await, Ok(p) if p.op == op && p.done > 0) {}
            })
            .await
            .unwrap();
            then().await;
            finished(&mut progress, op).await
        }
    }

    async fn finished(progress: &mut broadcast::Receiver<FsProgress>, op: Uuid) -> FsProgress {
        all_finished(progress, &[op]).await.remove(0)
    }

    /// The last word of each of `ops`, whichever ends first.
    async fn all_finished(progress: &mut broadcast::Receiver<FsProgress>, ops: &[Uuid]) -> Vec<FsProgress> {
        let mut done = Vec::new();
        tokio::time::timeout(Duration::from_secs(30), async {
            while done.len() < ops.len() {
                match progress.recv().await {
                    Ok(p) if ops.contains(&p.op) && p.finished => done.push(p),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(e) => panic!("{e}"),
                }
            }
        })
        .await
        .unwrap();
        done
    }

    /// Serve `root` as a phone does, each request on its own. Anything in /private is refused.
    fn serve(node: &Node, root: PathBuf, slow: Duration) {
        let mut calls = node.incoming_fs().unwrap();
        tokio::spawn(async move {
            while let Some(call) = calls.recv().await {
                let root = root.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(slow).await;
                    answer(call, &root).await;
                });
            }
        });
    }

    async fn answer(mut call: FsCall, root: &Path) {
        use std::io::{Read, Seek, Write};
        enum Reply {
            Entries(Vec<FsEntry>),
            Entry(FsEntry),
            Data(Vec<u8>),
            Ok,
        }
        let body = call.body().await.unwrap_or_default();
        let at = |p: &str| root.join(p.trim_start_matches('/'));
        let entry = |p: &Path| {
            let m = std::fs::metadata(p).ok()?;
            Some(FsEntry { name: p.file_name()?.to_string_lossy().into_owned(), dir: m.is_dir(), size: m.len(), modified_ms: 1_700_000_000_000 })
        };
        let path = match &call.request {
            FsRequest::List { path } | FsRequest::Stat { path } | FsRequest::Read { path, .. } | FsRequest::Write { path, .. } => path.clone(),
            FsRequest::Mkdir { path } | FsRequest::Delete { path } | FsRequest::Rename { path, .. } => path.clone(),
            FsRequest::Info => String::new(),
        };
        if path.starts_with("/private") {
            let _ = call.error("Permission denied", false).await;
            return;
        }
        let done = (|| -> std::io::Result<Reply> {
            Ok(match call.request.clone() {
                FsRequest::List { path } => Reply::Entries(std::fs::read_dir(at(&path))?.flatten().filter_map(|e| entry(&e.path())).collect()),
                FsRequest::Stat { path } => Reply::Entry(entry(&at(&path)).ok_or(std::io::Error::new(ErrorKind::NotFound, "No such file"))?),
                FsRequest::Read { path, offset, length } => {
                    let mut f = std::fs::File::open(at(&path))?;
                    f.seek(std::io::SeekFrom::Start(offset))?;
                    let mut data = Vec::new();
                    f.take(length).read_to_end(&mut data)?;
                    Reply::Data(data)
                }
                FsRequest::Write { path, offset, truncate, .. } => {
                    let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(truncate).open(at(&path))?;
                    f.seek(std::io::SeekFrom::Start(offset))?;
                    f.write_all(&body)?;
                    Reply::Ok
                }
                FsRequest::Mkdir { path } => std::fs::create_dir(at(&path)).map(|_| Reply::Ok)?,
                FsRequest::Delete { path } => {
                    let p = at(&path);
                    if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }?;
                    Reply::Ok
                }
                FsRequest::Rename { path, to } => std::fs::rename(at(&path), at(&to)).map(|_| Reply::Ok)?,
                FsRequest::Info => Reply::Ok,
            })
        })();
        let _ = match done {
            Ok(Reply::Entries(e)) => call.entries(e).await,
            Ok(Reply::Entry(e)) => call.entry(e).await,
            Ok(Reply::Data(d)) => call.data(&d).await,
            Ok(Reply::Ok) => call.ok().await,
            Err(e) => call.error(&e.to_string(), e.kind() == ErrorKind::NotFound).await,
        };
    }

    fn bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    /// Every file under `dir`, relative to it.
    fn tree(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut folders = vec![dir.to_path_buf()];
        while let Some(d) = folders.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let rel = e.path().strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                if e.path().is_dir() {
                    folders.push(e.path());
                    out.push(format!("{rel}/"));
                } else {
                    out.push(rel);
                }
            }
        }
        out.sort();
        out
    }

    #[tokio::test]
    async fn copies_files_and_folders_out_never_over_anything() {
        let t = Setup::new(Duration::ZERO).await;
        let s = &t.storage;
        std::fs::create_dir_all(s.join("DCIM/Été 2026/empty")).unwrap();
        std::fs::write(s.join("DCIM/Été 2026/a b.jpg"), b"jpeg").unwrap();
        std::fs::write(s.join("DCIM/zero.txt"), b"").unwrap();
        let big = bytes(3 * CHUNK as usize + 12_345);
        std::fs::write(s.join("big.bin"), &big).unwrap();

        let p = t.copy(&["/DCIM", "/big.bin"]).await;
        assert_eq!(p.error, None);
        assert_eq!(p.saved, [t.out.join("DCIM"), t.out.join("big.bin")]);
        assert_eq!((p.done, p.total), (big.len() as u64 + 4, big.len() as u64 + 4));
        assert_eq!(std::fs::read(t.out.join("big.bin")).unwrap(), big);
        assert_eq!(std::fs::read(t.out.join("DCIM/Été 2026/a b.jpg")).unwrap(), b"jpeg");
        assert_eq!(tree(&t.out.join("DCIM")), ["zero.txt", "Été 2026/", "Été 2026/a b.jpg", "Été 2026/empty/"]);
        // The phone's dates come along
        let modified = std::fs::metadata(t.out.join("big.bin")).unwrap().modified().unwrap();
        assert_eq!(modified, UNIX_EPOCH + Duration::from_millis(1_700_000_000_000));

        let p = t.copy(&["/DCIM", "/big.bin"]).await;
        assert_eq!(p.saved, [t.out.join("DCIM (1)"), t.out.join("big (1).bin")]);
        assert_eq!(std::fs::read(t.out.join("big.bin")).unwrap(), big);
        assert!(!tree(&t.out).iter().any(|f| f.ends_with(PART_SUFFIX)));

        // In the history, received from the phone
        let history = t.history.list(None);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].direction, linksy_core::Direction::Incoming);
        let Shared::Files { files, saved, .. } = &history[0].what else { panic!("{:?}", history[0].what) };
        assert!(files.contains(&"DCIM (1)/Été 2026/a b.jpg".to_string()) && files.contains(&"big (1).bin".to_string()), "{files:?}");
        assert!(saved.contains(&t.out.join("DCIM (1)/zero.txt")));
    }

    #[tokio::test]
    async fn uploads_files_and_folders_in_never_over_anything() {
        let t = Setup::new(Duration::ZERO).await;
        let d = t.storage.join("Download");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Report.PDF"), b"old").unwrap();
        let here = t.dir.path().join("here");
        std::fs::create_dir_all(here.join("Folder/empty")).unwrap();
        let big = bytes(2 * CHUNK as usize);
        std::fs::write(here.join("Folder/x y.bin"), &big).unwrap();
        std::fs::write(here.join("report.pdf"), b"new").unwrap();
        std::fs::write(here.join("zero"), b"").unwrap();
        let paths = [here.join("report.pdf"), here.join("Folder"), here.join("zero")];

        let p = t.upload("/Download", &paths).await;
        assert_eq!(p.error, None);
        assert_eq!(p.dir.as_deref(), Some("/Download"));
        assert_eq!((p.done, p.total), (big.len() as u64 + 3, big.len() as u64 + 3));
        // The phone ignores case: report.pdf would have been Report.PDF
        assert_eq!(std::fs::read(d.join("Report.PDF")).unwrap(), b"old");
        assert_eq!(std::fs::read(d.join("report (1).pdf")).unwrap(), b"new");
        assert_eq!(std::fs::read(d.join("Folder/x y.bin")).unwrap(), big);
        assert_eq!(tree(&d), ["Folder/", "Folder/empty/", "Folder/x y.bin", "Report.PDF", "report (1).pdf", "zero"]);

        let p = t.upload("/Download", &paths[1..]).await;
        assert_eq!(p.error, None);
        assert_eq!(std::fs::read(d.join("Folder (1)/x y.bin")).unwrap(), big);
        assert!(d.join("zero (1)").exists());
        let history = t.history.list(None);
        assert_eq!(history[0].direction, linksy_core::Direction::Outgoing);
        assert_eq!(history[0].what.file_count(), 2);

        // Nowhere to go, nothing to send
        assert!(t.upload("/Nope", &paths[..1]).await.error.is_some());
        assert!(t.upload("/Download", &[here.join("missing")]).await.error.unwrap().contains("missing"));
        assert!(t.files.upload(t.phone_id, "/../x", paths.to_vec()).is_err());
        assert!(t.files.upload(t.phone_id, "/", Vec::new()).is_err());
    }

    #[tokio::test]
    async fn makes_renames_and_deletes() {
        let t = Setup::new(Duration::ZERO).await;
        let (f, id, s) = (&t.files, t.phone_id, &t.storage);
        f.mkdir(id, "/New").await.unwrap();
        assert!(s.join("New").is_dir());
        assert!(f.mkdir(id, "/New").await.unwrap_err().contains("already"));
        for bad in ["/", "/a/../b", "/ "] {
            assert!(f.mkdir(id, bad).await.is_err(), "{bad}");
        }

        assert_eq!(f.rename(id, "/New", "Old").await.unwrap(), "/Old");
        assert!(s.join("Old").is_dir() && !s.join("New").exists());
        for bad in ["a/b", "", " ", ".", ".."] {
            assert!(f.rename(id, "/Old", bad).await.is_err(), "{bad:?}");
        }
        assert!(f.rename(id, "/", "x").await.is_err());
        std::fs::write(s.join("x.txt"), b"x").unwrap();
        assert!(f.rename(id, "/Old", "x.txt").await.unwrap_err().contains("already"));
        assert_eq!(f.rename(id, "/x.txt", "X.txt").await.unwrap(), "/X.txt");
        assert!(s.join("X.txt").exists());

        std::fs::write(s.join("Old/inside.txt"), b"x").unwrap();
        f.delete(id, vec!["/Old".into(), "/X.txt".into(), "/already gone".into()]).await.unwrap();
        assert_eq!(tree(s), Vec::<String>::new());
        assert!(f.delete(id, vec!["/".into()]).await.is_err());
        std::fs::create_dir_all(s.join("private")).unwrap();
        assert_eq!(f.delete(id, vec!["/private".into()]).await.unwrap_err(), "Permission denied");
    }

    #[tokio::test]
    async fn stopped_copies_leave_nothing_half_done() {
        let t = Setup::new(Duration::from_millis(40)).await;
        std::fs::write(t.storage.join("big.bin"), bytes(12 * CHUNK as usize)).unwrap();
        let phone = t.phone_id;
        let op = std::cell::Cell::new(Uuid::nil());
        let p = t
            .stop_halfway(
                |f| {
                    op.set(f.copy(phone, vec!["/big.bin".into()]).unwrap());
                    op.get()
                },
                async || assert!(t.files.cancel(op.get())),
            )
            .await;
        assert_eq!(p.error.as_deref(), Some("cancelled"));
        assert_eq!(tree(&t.out), Vec::<String>::new());
        assert!(!t.files.cancel(op.get()), "not running anymore");

        // An upload stopped halfway leaves no half file on the phone
        std::fs::create_dir_all(t.storage.join("up")).unwrap();
        let here = t.dir.path().join("big.bin");
        std::fs::write(&here, bytes(12 * CHUNK as usize)).unwrap();
        let p = t
            .stop_halfway(
                |f| {
                    op.set(f.upload(phone, "/up", vec![here.clone()]).unwrap());
                    op.get()
                },
                async || assert!(t.files.cancel(op.get())),
            )
            .await;
        assert_eq!(p.error.as_deref(), Some("cancelled"));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(tree(&t.storage.join("up")), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_phone_going_away_mid_copy_fails_cleanly() {
        let t = Setup::new(Duration::from_millis(40)).await;
        std::fs::write(t.storage.join("big.bin"), bytes(12 * CHUNK as usize)).unwrap();
        let phone = t.phone_id;
        let p = t.stop_halfway(|f| f.copy(phone, vec!["/big.bin".into()]).unwrap(), async || t.phone.shutdown().await).await;
        let error = p.error.unwrap();
        assert!(error.contains("Pixel"), "{error}");
        assert_eq!(tree(&t.out), Vec::<String>::new());
        assert!(t.history.list(None).is_empty());
    }

    #[tokio::test]
    async fn refusals_and_two_copies_at_once() {
        let t = Setup::new(Duration::ZERO).await;
        std::fs::create_dir_all(t.storage.join("private")).unwrap();
        std::fs::write(t.storage.join("private/x"), b"x").unwrap();
        assert_eq!(t.copy(&["/private/x"]).await.error.as_deref(), Some("Permission denied"));
        assert_eq!(t.copy(&["/nothing here"]).await.error.as_deref(), Some("No such file"));
        for bad in [&[][..], &["/"], &["/../etc/passwd"]] {
            assert!(t.files.copy(t.phone_id, bad.iter().map(|p| p.to_string()).collect()).is_err(), "{bad:?}");
        }

        std::fs::write(t.storage.join("a.txt"), bytes(CHUNK as usize + 1)).unwrap();
        let mut progress = t.files.subscribe();
        let ops = [(); 2].map(|_| t.files.copy(t.phone_id, vec!["/a.txt".into()]).unwrap());
        let mut saved = Vec::new();
        for p in all_finished(&mut progress, &ops).await {
            assert_eq!(p.error, None);
            saved.extend(p.saved);
        }
        saved.sort();
        assert_eq!(saved, [t.out.join("a (1).txt"), t.out.join("a.txt")]);
        assert_eq!(std::fs::read(t.out.join("a (1).txt")).unwrap(), bytes(CHUNK as usize + 1));
    }

    #[test]
    fn names_and_paths() {
        assert_eq!(numbered("photo.jpg", 2, false), "photo (2).jpg");
        assert_eq!(numbered("Photos.2026", 1, true), "Photos.2026 (1)");
        assert_eq!(numbered(".hidden", 1, false), ".hidden (1)");
        assert_eq!(phone_path("DCIM//Camera/").unwrap(), "/DCIM/Camera");
        assert!(phone_path("/a/./b").is_err());
        assert_eq!(split("/a"), ("/", "a"));
        assert_eq!(split("/a/b c"), ("/a", "b c"));
        assert_eq!(join("/", "a"), "/a");
    }
}
