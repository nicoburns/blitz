//! File download support.
//!
//! When a navigation response carries a `Content-Disposition: attachment`
//! header the browser saves the body to the user's Downloads directory instead
//! of trying to render it. This module handles detecting that header, deriving
//! a sensible filename, and writing the bytes to disk.

use std::path::{Path, PathBuf};

use blitz_net::ResponseHead;
use blitz_traits::net::{HeaderMap, Url};
use dioxus_native::prelude::*;

/// Returns `true` if the response headers request that the body be handled as a
/// downloadable attachment rather than displayed inline.
pub fn is_attachment(headers: &HeaderMap) -> bool {
    disposition_str(headers)
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("attachment")
        })
        .unwrap_or(false)
}

/// Pick a filename for a downloaded response. Prefers the `Content-Disposition`
/// filename (RFC 5987 `filename*` first, then plain `filename`), then the URL's
/// last path segment, and finally a generic fallback. The result is always a
/// bare filename with any path components stripped.
pub fn download_filename(headers: &HeaderMap, url: &Url) -> String {
    let from_header = disposition_str(headers).and_then(filename_from_disposition);
    let name = from_header
        .or_else(|| filename_from_url(url))
        .unwrap_or_default();

    let sanitized = sanitize_filename(&name);
    if sanitized.is_empty() {
        "download".to_string()
    } else {
        sanitized
    }
}

/// Write `bytes` to the user's Downloads directory under `filename`, avoiding
/// collisions by appending ` (n)` before the extension. Returns the path that
/// was actually written.
pub fn save_to_downloads(filename: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = downloads_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = unique_path(&dir, filename);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// State of a single download in the current session.
#[derive(Clone, PartialEq)]
pub enum DownloadStatus {
    InProgress,
    Completed { path: PathBuf },
    Failed { error: String },
}

/// A download initiated during the current browser session.
#[derive(Clone, PartialEq)]
pub struct Download {
    pub id: u64,
    pub filename: String,
    pub url: Url,
    pub status: DownloadStatus,
}

/// Session-wide registry of downloads. Shared via context so the toolbar can
/// show in-progress state and list results across all tabs. Only lives for the
/// current process; nothing is persisted.
#[derive(Clone, Copy)]
pub struct Downloads {
    items: Signal<Vec<Download>>,
    counter: Signal<u64>,
}

impl Downloads {
    pub fn new() -> Self {
        Self {
            items: Signal::new(Vec::new()),
            counter: Signal::new(0),
        }
    }

    /// Register a new in-progress download and return its id.
    pub fn start(&self, filename: String, url: Url) -> u64 {
        let mut counter = self.counter;
        let id = *counter.read() + 1;
        counter.set(id);

        let mut items = self.items;
        items.write().push(Download {
            id,
            filename,
            url,
            status: DownloadStatus::InProgress,
        });
        id
    }

    pub fn complete(&self, id: u64, path: PathBuf) {
        self.set_status(id, DownloadStatus::Completed { path });
    }

    pub fn fail(&self, id: u64, error: String) {
        self.set_status(id, DownloadStatus::Failed { error });
    }

    fn set_status(&self, id: u64, status: DownloadStatus) {
        let mut items = self.items;
        let mut guard = items.write();
        if let Some(item) = guard.iter_mut().find(|d| d.id == id) {
            item.status = status;
        }
    }

    /// Reactive handle to the list of downloads (oldest first).
    pub fn items(&self) -> Signal<Vec<Download>> {
        self.items
    }

    /// True once at least one download has been started this session.
    pub fn has_any(&self) -> bool {
        !self.items.read().is_empty()
    }

    /// True while at least one download is still in progress.
    pub fn has_active(&self) -> bool {
        self.items
            .read()
            .iter()
            .any(|d| matches!(d.status, DownloadStatus::InProgress))
    }
}

impl Default for Downloads {
    fn default() -> Self {
        Self::new()
    }
}

/// Read the body of a detected download and save it to disk, updating the
/// shared [`Downloads`] registry with the outcome. Intended to be spawned so it
/// outlives the navigation that triggered it.
pub async fn run_download(downloads: Downloads, id: u64, head: ResponseHead, filename: String) {
    let bytes = match head.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!("Failed to download {}: {}", filename, err);
            downloads.fail(id, err.to_string());
            return;
        }
    };

    let save_name = filename.clone();
    let saved = tokio::task::spawn_blocking(move || save_to_downloads(&save_name, &bytes))
        .await
        .map_err(|e| e.to_string())
        .and_then(|res| res.map_err(|e| e.to_string()));

    match saved {
        Ok(path) => {
            tracing::info!("Saved download {} to {}", filename, path.display());
            downloads.complete(id, path);
        }
        Err(err) => {
            tracing::error!("Failed to save download {}: {}", filename, err);
            downloads.fail(id, err);
        }
    }
}

/// Open a downloaded file with the OS default application.
pub fn open_file(path: &Path) {
    if let Err(err) = platform_open(path, false) {
        tracing::error!("Failed to open {}: {}", path.display(), err);
    }
}

/// Reveal a downloaded file in the OS file manager, selecting it where the
/// platform supports it.
pub fn reveal_in_folder(path: &Path) {
    if let Err(err) = platform_open(path, true) {
        tracing::error!("Failed to reveal {}: {}", path.display(), err);
    }
}

#[cfg(target_os = "macos")]
fn platform_open(path: &Path, reveal: bool) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new("open");
    if reveal {
        cmd.arg("-R");
    }
    cmd.arg(path).spawn().map(|_| ())
}

#[cfg(target_os = "windows")]
fn platform_open(path: &Path, reveal: bool) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new("explorer");
    if reveal {
        cmd.arg(format!("/select,{}", path.display()));
    } else {
        cmd.arg(path);
    }
    cmd.spawn().map(|_| ())
}

// Other unix targets (Linux, and mobile as a best-effort no-op fallback).
#[cfg(all(unix, not(target_os = "macos")))]
fn platform_open(path: &Path, reveal: bool) -> std::io::Result<()> {
    // xdg-open cannot select a file, so revealing opens the containing folder.
    let target = if reveal {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    std::process::Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map(|_| ())
}

fn disposition_str(headers: &HeaderMap) -> Option<&str> {
    headers.get("content-disposition")?.to_str().ok()
}

fn filename_from_disposition(value: &str) -> Option<String> {
    let mut plain: Option<String> = None;
    let mut extended: Option<String> = None;

    for part in value.split(';').skip(1) {
        let part = part.trim();
        // `filename*` must be checked before `filename` since it shares the prefix.
        if let Some(rest) = part.strip_prefix("filename*") {
            if let Some(raw) = rest.split_once('=').map(|(_, v)| v.trim()) {
                extended = decode_ext_value(raw);
            }
        } else if let Some(rest) = part.strip_prefix("filename") {
            if let Some(raw) = rest.split_once('=').map(|(_, v)| v.trim()) {
                plain = Some(unquote(raw));
            }
        }
    }

    extended.or(plain).filter(|s| !s.is_empty())
}

/// Decode an RFC 5987 ext-value of the form `charset'lang'percent-encoded`.
fn decode_ext_value(value: &str) -> Option<String> {
    let mut parts = value.splitn(3, '\'');
    let _charset = parts.next()?;
    let _lang = parts.next()?;
    let encoded = parts.next()?;
    Some(percent_decode(encoded))
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        // Undo backslash escaping used within quoted-strings.
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut escaped = false;
        for c in inner.chars() {
            if escaped {
                out.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else {
                out.push(c);
            }
        }
        out
    } else {
        trimmed.to_string()
    }
}

fn filename_from_url(url: &Url) -> Option<String> {
    url.path_segments()?
        .rfind(|s| !s.is_empty())
        .map(percent_decode)
}

fn sanitize_filename(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    base.chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\'))
        .collect::<String>()
        .trim()
        .to_string()
}

fn downloads_dir() -> std::io::Result<PathBuf> {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "could not locate a Downloads directory",
            )
        })
}

fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let ext = path.extension().and_then(|s| s.to_str());

    for n in 1u32.. {
        let name = match ext {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    // The range above is effectively unbounded; a path is always found.
    unreachable!("failed to find a free download filename")
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blitz_traits::net::HeaderMap;

    fn headers_with(disposition: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-disposition",
            disposition.parse().expect("valid header value"),
        );
        headers
    }

    fn url(s: &str) -> Url {
        Url::parse(s).expect("valid url")
    }

    #[test]
    fn detects_attachment() {
        assert!(is_attachment(&headers_with("attachment")));
        assert!(is_attachment(&headers_with(
            "attachment; filename=\"report.pdf\""
        )));
        assert!(is_attachment(&headers_with("ATTACHMENT")));
    }

    #[test]
    fn inline_is_not_attachment() {
        assert!(!is_attachment(&headers_with("inline")));
        assert!(!is_attachment(&headers_with("inline; filename=\"a.txt\"")));
        assert!(!is_attachment(&HeaderMap::new()));
    }

    #[test]
    fn plain_filename() {
        let name = download_filename(
            &headers_with("attachment; filename=\"my report.pdf\""),
            &url("https://example.com/x"),
        );
        assert_eq!(name, "my report.pdf");
    }

    #[test]
    fn unquoted_filename() {
        let name = download_filename(
            &headers_with("attachment; filename=report.csv"),
            &url("https://example.com/x"),
        );
        assert_eq!(name, "report.csv");
    }

    #[test]
    fn rfc5987_filename_is_preferred() {
        let name = download_filename(
            &headers_with(
                "attachment; filename=\"fallback.txt\"; filename*=UTF-8''na%C3%AFve%20file.txt",
            ),
            &url("https://example.com/x"),
        );
        assert_eq!(name, "naïve file.txt");
    }

    #[test]
    fn falls_back_to_url_segment() {
        let name = download_filename(
            &headers_with("attachment"),
            &url("https://example.com/files/archive%20final.zip"),
        );
        assert_eq!(name, "archive final.zip");
    }

    #[test]
    fn strips_path_traversal() {
        let name = download_filename(
            &headers_with("attachment; filename=\"../../etc/passwd\""),
            &url("https://example.com/x"),
        );
        assert_eq!(name, "passwd");
    }

    #[test]
    fn generic_fallback_when_nothing_usable() {
        let name = download_filename(&headers_with("attachment"), &url("https://example.com/"));
        assert_eq!(name, "download");
    }

    #[test]
    fn unique_path_appends_counter() {
        let dir = std::env::temp_dir().join(format!("blitz-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("a.txt"));
        let _ = std::fs::remove_file(dir.join("a (1).txt"));

        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (1).txt"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
