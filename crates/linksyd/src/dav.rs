//! A phone in the Files sidebar: a small WebDAV server on this computer, one per phone,
//! that turns what the file manager asks into the phone's file requests (see
//! `linksy_core::fs`). GVfs mounts it (`dav://127.0.0.1:PORT/TOKEN/`), so the phone shows
//! up in Files and every file dialog, and (outside a Flatpak) a bookmark names it after
//! the phone.
//!
//! Only this machine can connect (loopback), and only with the phone's random token in
//! the path. Nothing is cached: every listing and read goes to the phone.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use linksy_core::fs::{FsAnswer, MAX_CHUNK};
use linksy_core::{Error, Fingerprint, FsEntry, FsRequest, Node};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

/// Each read from the phone, and each write to it.
const CHUNK: u64 = 1 << 20;

/// The running bridges, once the service has started them.
pub static BRIDGES: std::sync::OnceLock<Bridges> = std::sync::OnceLock::new();

#[derive(Clone)]
pub struct Bridges {
    path: PathBuf,
    node: Node,
    /// Phones that have been browsed, with their address here, kept across restarts so
    /// the bookmark stays valid.
    known: Arc<Mutex<HashMap<Fingerprint, Place>>>,
    running: Arc<Mutex<HashMap<Fingerprint, u16>>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Place {
    port: u16,
    token: String,
}

impl Bridges {
    /// Load the phones browsed before and serve them again.
    pub async fn start(path: PathBuf, node: Node) -> Self {
        let known: HashMap<Fingerprint, Place> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let bridges = Self { path, node, known: Arc::new(Mutex::new(known.clone())), running: Arc::default() };
        for id in known.keys() {
            if let Err(e) = bridges.serve(*id).await {
                warn!("can't serve a phone's files: {e}");
            }
        }
        bridges
    }

    /// Show a phone's files in the file manager: serve it, mount it, bookmark it, open it.
    pub async fn browse(&self, phone: Fingerprint, name: &str) -> Result<String, String> {
        // First make sure the phone shares its files at all, for a clear message.
        match self.node.fs(&phone, FsRequest::Stat { path: "/".into() }, &[]).await {
            Ok(_) => {}
            Err(Error::Invalid(m)) if m.contains("doesn't share") => {
                return Err(format!("{name} doesn't share its files yet. Turn it on in Linksy on {name}."));
            }
            Err(e) => return Err(e.to_string()),
        }
        // Label it with the folder the phone shares, when it says ("Pixel 9 (Documents)").
        let label = match self.node.fs(&phone, FsRequest::Info, &[]).await {
            Ok(FsAnswer::Info { name: folder }) if !folder.trim().is_empty() => format!("{name} ({})", folder.trim()),
            _ => name.to_string(),
        };
        let uri = self.serve(phone).await.map_err(|e| e.to_string())?;
        let mounted = tokio::process::Command::new("gio").args(["mount", &uri]).output().await;
        if let Ok(out) = &mounted
            && !out.status.success()
            && !String::from_utf8_lossy(&out.stderr).contains("already mounted")
        {
            debug!("gio mount: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        bookmark(&uri, &label);
        // Tests check the bridge without a window opening on someone's screen.
        if std::env::var_os("LINKSY_FILES_NO_OPEN").is_none() {
            show_folder(&uri).await;
        }
        Ok(uri)
    }

    /// Serve a phone (once), at its remembered address when possible.
    async fn serve(&self, phone: Fingerprint) -> std::io::Result<String> {
        let place = self.known.lock().unwrap().get(&phone).cloned();
        if let Some(port) = self.running.lock().unwrap().get(&phone).copied() {
            let token = place.map(|p| p.token).unwrap_or_default();
            return Ok(uri(port, &token));
        }
        let token = place.as_ref().map(|p| p.token.clone()).unwrap_or_else(|| linksy_core::Uuid::new_v4().simple().to_string());
        let listener = match place.as_ref().map(|p| p.port) {
            Some(port) => match TcpListener::bind(("127.0.0.1", port)).await {
                Ok(l) => l,
                Err(_) => TcpListener::bind(("127.0.0.1", 0)).await?,
            },
            None => TcpListener::bind(("127.0.0.1", 0)).await?,
        };
        let port = listener.local_addr()?.port();
        self.known.lock().unwrap().insert(phone, Place { port, token: token.clone() });
        self.save();
        self.running.lock().unwrap().insert(phone, port);
        info!("serving a phone's files on 127.0.0.1:{port}");
        let (node, prefix) = (self.node.clone(), format!("/{token}"));
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (node, prefix) = (node.clone(), prefix.clone());
                tokio::spawn(async move {
                    if let Err(e) = connection(stream, node, phone, prefix).await {
                        debug!("file manager connection: {e}");
                    }
                });
            }
        });
        Ok(uri(port, &token))
    }

    fn save(&self) {
        let known = self.known.lock().unwrap().clone();
        if let Ok(json) = serde_json::to_vec_pretty(&known) {
            let _ = linksy_core::identity::write_private(&self.path, &json);
        }
    }
}

fn uri(port: u16, token: &str) -> String {
    format!("dav://127.0.0.1:{port}/{token}/")
}

/// Add the phone to the Files sidebar under its own name (GTK's bookmarks file, the
/// computer's from the Flatpak).
fn bookmark(uri: &str, name: &str) {
    let path = linksy_host::config_dir().join("gtk-3.0").join("bookmarks");
    let text = linksy_host::read(&path).unwrap_or_default();
    if text.lines().any(|l| l.split(' ').next() == Some(uri)) {
        return;
    }
    let mut text = text;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("{uri} {name}\n"));
    if let Err(e) = linksy_host::write(&path, text.as_bytes()) {
        debug!("bookmark: {e}");
    }
}

/// Open a location in the file manager.
async fn show_folder(uri: &str) {
    let shown = async {
        let bus = zbus::Connection::session().await?;
        bus.call_method(Some("org.freedesktop.FileManager1"), "/org/freedesktop/FileManager1", Some("org.freedesktop.FileManager1"), "ShowFolders", &(vec![uri], ""))
            .await?;
        Ok::<_, zbus::Error>(())
    };
    if let Err(e) = shown.await {
        debug!("ShowFolders: {e}; trying gio open");
        let _ = tokio::process::Command::new("gio").args(["open", uri]).status().await;
    }
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
}

/// One file-manager connection: requests one after another until it closes.
async fn connection(stream: TcpStream, node: Node, phone: Fingerprint, prefix: String) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    loop {
        let Some(req) = head(&mut read).await? else { return Ok(()) };
        let close = req.headers.get("connection").is_some_and(|v| v.eq_ignore_ascii_case("close"));
        let path = match req.path.split('?').next().map(percent_decode).and_then(|p| p.strip_prefix(&prefix).map(str::to_string)) {
            Some(p) if p.is_empty() || p.starts_with('/') => clean(&p),
            _ => {
                skip_body(&mut read, &req).await?;
                respond(&mut write, 404, "Not Found", &[], b"").await?;
                continue;
            }
        };
        let phone_fs = Fs { node: &node, phone };
        match req.method.as_str() {
            "OPTIONS" => {
                skip_body(&mut read, &req).await?;
                respond(&mut write, 200, "OK", &[("DAV", "1"), ("Allow", "OPTIONS, PROPFIND, GET, HEAD, PUT, DELETE, MKCOL, MOVE")], b"").await?;
            }
            "PROPFIND" => {
                skip_body(&mut read, &req).await?;
                let depth_one = req.headers.get("depth").is_none_or(|d| d != "0");
                match propfind(&phone_fs, &prefix, &path, depth_one).await {
                    Ok(xml) => respond(&mut write, 207, "Multi-Status", &[("Content-Type", "application/xml; charset=utf-8")], xml.as_bytes()).await?,
                    Err(e) => error(&mut write, e).await?,
                }
            }
            "GET" | "HEAD" => {
                skip_body(&mut read, &req).await?;
                get(&mut write, &phone_fs, &path, req.method == "HEAD", req.headers.get("range").map(String::as_str)).await?;
            }
            "PUT" => match put(&mut read, &phone_fs, &path, &req).await {
                Ok(()) => respond(&mut write, 201, "Created", &[], b"").await?,
                Err(e) => error(&mut write, e).await?,
            },
            "MKCOL" => {
                skip_body(&mut read, &req).await?;
                match phone_fs.ask(FsRequest::Mkdir { path: path.clone() }, &[]).await {
                    Ok(_) => respond(&mut write, 201, "Created", &[], b"").await?,
                    Err(e) => error(&mut write, e).await?,
                }
            }
            "DELETE" => {
                skip_body(&mut read, &req).await?;
                match phone_fs.ask(FsRequest::Delete { path: path.clone() }, &[]).await {
                    Ok(_) => respond(&mut write, 204, "No Content", &[], b"").await?,
                    Err(e) => error(&mut write, e).await?,
                }
            }
            "MOVE" => {
                skip_body(&mut read, &req).await?;
                let to = req.headers.get("destination").and_then(|d| destination(d, &prefix));
                match to {
                    Some(to) => match phone_fs.ask(FsRequest::Rename { path: path.clone(), to }, &[]).await {
                        Ok(_) => respond(&mut write, 201, "Created", &[], b"").await?,
                        Err(e) => error(&mut write, e).await?,
                    },
                    None => respond(&mut write, 400, "Bad Request", &[], b"").await?,
                }
            }
            _ => {
                skip_body(&mut read, &req).await?;
                respond(&mut write, 405, "Method Not Allowed", &[("Allow", "OPTIONS, PROPFIND, GET, HEAD, PUT, DELETE, MKCOL, MOVE")], b"").await?;
            }
        }
        if close {
            return Ok(());
        }
    }
}

struct Fs<'a> {
    node: &'a Node,
    phone: Fingerprint,
}

impl Fs<'_> {
    async fn ask(&self, request: FsRequest, body: &[u8]) -> linksy_core::Result<FsAnswer> {
        self.node.fs(&self.phone, request, body).await
    }

    async fn stat(&self, path: &str) -> linksy_core::Result<FsEntry> {
        match self.ask(FsRequest::Stat { path: path.into() }, &[]).await? {
            FsAnswer::Entry(e) => Ok(e),
            _ => Err(Error::Protocol("unexpected answer".into())),
        }
    }
}

/// The request line and headers; `None` when the connection closed.
async fn head(read: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> std::io::Result<Option<Request>> {
    let mut line = String::new();
    if read.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Err(std::io::Error::other("bad request line"));
    };
    let (method, path) = (method.to_string(), path.to_string());
    let mut headers = HashMap::new();
    let mut size = 0;
    loop {
        line.clear();
        let n = read.read_line(&mut line).await?;
        size += n;
        if n == 0 || size > 64 * 1024 {
            return Err(std::io::Error::other("bad headers"));
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    Ok(Some(Request { method, path, headers }))
}

/// Read (and drop) a request body nobody needs.
async fn skip_body(read: &mut BufReader<tokio::net::tcp::OwnedReadHalf>, req: &Request) -> std::io::Result<()> {
    let mut body = Body::new(req);
    let mut buf = vec![0u8; 64 * 1024];
    while body.read(read, &mut buf).await? > 0 {}
    Ok(())
}

/// A request body, sized by Content-Length or sent in chunks.
struct Body {
    chunked: bool,
    left: u64,
    done: bool,
}

impl Body {
    fn new(req: &Request) -> Self {
        let chunked = req.headers.get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked"));
        let left = req.headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        Self { chunked, left, done: !chunked && left == 0 }
    }

    /// Up to `buf.len()` bytes; 0 at the end.
    async fn read(&mut self, read: &mut BufReader<tokio::net::tcp::OwnedReadHalf>, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done {
            return Ok(0);
        }
        if self.chunked && self.left == 0 {
            let mut line = String::new();
            read.read_line(&mut line).await?;
            if line.trim().is_empty() {
                line.clear();
                read.read_line(&mut line).await?;
            }
            let size = u64::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16).map_err(std::io::Error::other)?;
            if size == 0 {
                // Trailers, then the blank line.
                loop {
                    line.clear();
                    if read.read_line(&mut line).await? == 0 || line.trim().is_empty() {
                        break;
                    }
                }
                self.done = true;
                return Ok(0);
            }
            self.left = size;
        }
        let want = (buf.len() as u64).min(self.left) as usize;
        let n = read.read(&mut buf[..want]).await?;
        if n == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        self.left -= n as u64;
        if !self.chunked && self.left == 0 {
            self.done = true;
        }
        Ok(n)
    }
}

async fn respond(write: &mut tokio::net::tcp::OwnedWriteHalf, code: u16, reason: &str, headers: &[(&str, &str)], body: &[u8]) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    write.write_all(head.as_bytes()).await?;
    write.write_all(body).await
}

async fn error(write: &mut tokio::net::tcp::OwnedWriteHalf, e: Error) -> std::io::Result<()> {
    let (code, reason) = match e {
        Error::NotFound(_) => (404, "Not Found"),
        Error::Invalid(_) => (403, "Forbidden"),
        _ => (503, "Service Unavailable"),
    };
    debug!("phone file request failed: {e}");
    respond(write, code, reason, &[("Content-Type", "text/plain; charset=utf-8")], e.to_string().as_bytes()).await
}

async fn propfind(fs: &Fs<'_>, prefix: &str, path: &str, depth_one: bool) -> linksy_core::Result<String> {
    let me = fs.stat(path).await?;
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<d:multistatus xmlns:d=\"DAV:\">\n");
    xml.push_str(&response_xml(prefix, path, &me));
    if me.dir && depth_one {
        let FsAnswer::Entries(entries) = fs.ask(FsRequest::List { path: path.into() }, &[]).await? else {
            return Err(Error::Protocol("unexpected answer".into()));
        };
        for e in entries.iter().filter(|e| !e.name.is_empty() && !e.name.contains('/')) {
            let child = if path == "/" { format!("/{}", e.name) } else { format!("{path}/{}", e.name) };
            xml.push_str(&response_xml(prefix, &child, e));
        }
    }
    xml.push_str("</d:multistatus>\n");
    Ok(xml)
}

fn response_xml(prefix: &str, path: &str, e: &FsEntry) -> String {
    let mut href = format!("{prefix}{}", percent_encode(path));
    if e.dir && !href.ends_with('/') {
        href.push('/');
    }
    let name = path.rsplit('/').next().unwrap_or("");
    let kind = if e.dir { "<d:resourcetype><d:collection/></d:resourcetype>".to_string() } else { format!("<d:resourcetype/><d:getcontentlength>{}</d:getcontentlength><d:getcontenttype>{}</d:getcontenttype>", e.size, mime(name)) };
    let modified = if e.modified_ms > 0 { format!("<d:getlastmodified>{}</d:getlastmodified>", http_date(e.modified_ms / 1000)) } else { String::new() };
    format!(
        "<d:response><d:href>{}</d:href><d:propstat><d:prop><d:displayname>{}</d:displayname>{kind}{modified}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\n",
        xml_escape(&href),
        xml_escape(name)
    )
}

async fn get(write: &mut tokio::net::tcp::OwnedWriteHalf, fs: &Fs<'_>, path: &str, head_only: bool, range: Option<&str>) -> std::io::Result<()> {
    let entry = match fs.stat(path).await {
        Ok(e) if !e.dir => e,
        Ok(_) => return respond(write, 405, "Method Not Allowed", &[], b"").await,
        Err(e) => return error(write, e).await,
    };
    let size = entry.size;
    let (start, end) = match range.and_then(|r| parse_range(r, size)) {
        Some(r) => r,
        None if range.is_some() && size > 0 => {
            return respond(write, 416, "Range Not Satisfiable", &[("Content-Range", &format!("bytes */{size}"))], b"").await;
        }
        None => (0, size),
    };
    let length = end - start;
    let mut head = if range.is_some() && size > 0 {
        format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{size}\r\n", end.saturating_sub(1))
    } else {
        "HTTP/1.1 200 OK\r\n".to_string()
    };
    head.push_str(&format!("Content-Length: {length}\r\nContent-Type: {}\r\nAccept-Ranges: bytes\r\n\r\n", mime(path)));
    write.write_all(head.as_bytes()).await?;
    if head_only {
        return Ok(());
    }
    let mut at = start;
    while at < end {
        let want = (end - at).min(CHUNK.min(MAX_CHUNK));
        match fs.ask(FsRequest::Read { path: path.into(), offset: at, length: want }, &[]).await {
            Ok(FsAnswer::Data(d)) if !d.is_empty() => {
                write.write_all(&d).await?;
                at += d.len() as u64;
            }
            // The phone stopped mid-file: cut the connection so the reader notices.
            _ => return Err(std::io::Error::other("the phone stopped sending the file")),
        }
    }
    Ok(())
}

async fn put(read: &mut BufReader<tokio::net::tcp::OwnedReadHalf>, fs: &Fs<'_>, path: &str, req: &Request) -> linksy_core::Result<()> {
    let mut body = Body::new(req);
    let mut chunk = vec![0u8; CHUNK as usize];
    let mut at = 0u64;
    let mut first = true;
    loop {
        // Fill a whole chunk before sending, so writes stay few and large.
        let mut filled = 0;
        while filled < chunk.len() {
            let n = body.read(read, &mut chunk[filled..]).await?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 && !first {
            return Ok(());
        }
        fs.ask(FsRequest::Write { path: path.into(), offset: at, length: filled as u64, truncate: first }, &chunk[..filled]).await?;
        at += filled as u64;
        first = false;
        if filled < chunk.len() {
            return Ok(());
        }
    }
}

/// `bytes=a-b`, `bytes=a-`, or `bytes=-n` → half-open [start, end).
fn parse_range(range: &str, size: u64) -> Option<(u64, u64)> {
    let spec = range.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (size.saturating_sub(n), size)
        }
        (a, "") => (a.parse().ok()?, size),
        (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.saturating_add(1).min(size)),
    };
    (start < end && start < size).then_some((start, end))
}

/// The phone path a MOVE's Destination header points to.
fn destination(dest: &str, prefix: &str) -> Option<String> {
    let path = match dest.find("://") {
        Some(i) => &dest[dest[i + 3..].find('/').map(|j| i + 3 + j)?..],
        None => dest,
    };
    let path = percent_decode(path);
    path.strip_prefix(prefix).map(clean)
}

/// "/a/./b//c/" → "/a/b/c"; never above the root.
fn clean(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            p => out.push(p),
        }
    }
    format!("/{}", out.join("/"))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn mime(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "zip" => "application/zip",
        "apk" => "application/vnd.android.package-archive",
        _ => "application/octet-stream",
    }
}

/// RFC 1123 date for HTTP ("Tue, 20 Oct 2026 10:30:00 GMT").
fn http_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    // Trust the numbers. Do not try to understand the numbers.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][(days.rem_euclid(7)) as usize];
    let month_name = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][(month - 1) as usize];
    format!("{weekday}, {day:02} {month_name} {year} {:02}:{:02}:{:02} GMT", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_paths_and_dates() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 100)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 1000)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 1000)));
        assert_eq!(parse_range("bytes=2000-", 1000), None);
        assert_eq!(clean("/a/./b//../c/"), "/a/c");
        assert_eq!(clean("/../../etc"), "/etc");
        assert_eq!(destination("http://127.0.0.1:5/tok/DCIM/b%20c.jpg", "/tok").as_deref(), Some("/DCIM/b c.jpg"));
        assert_eq!(percent_encode("/My Photos/é.jpg"), "/My%20Photos/%C3%A9.jpg");
        assert_eq!(http_date(1_792_492_200), "Tue, 20 Oct 2026 10:30:00 GMT");
    }
}
