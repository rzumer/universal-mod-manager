//! Network access. UMM identifies itself only as `UMM/<version>` and sends nothing else
//! about the user, except headers a service asks for (Nexus Mods' API key).

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct NetError {
    pub url: String,
    /// The HTTP status, when the server answered with an error.
    pub status: Option<u16>,
    /// What went wrong; for an HTTP error, the response body too (services put their reason
    /// there).
    pub message: String,
}

pub type Result<T, E = NetError> = std::result::Result<T, E>;

/// Download progress: bytes so far, and the total when the server says.
pub type Progress<'a> = &'a mut dyn FnMut(u64, Option<u64>);

pub trait Http: Send + Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>>;
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>>;
    /// A GET with extra request headers.
    fn get_with(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>>;
    /// A POST with extra request headers.
    fn post_with(
        &self,
        url: &str,
        content_type: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>>;
    /// Downloads `url` into the file `dest`, which is created or replaced.
    fn download(&self, url: &str, dest: &Path, progress: Progress) -> Result<()>;
}

pub struct UreqHttp {
    agent: ureq::Agent,
}

/// Largest response UMM reads into memory (package zips can be large). Downloads to a file
/// have no limit.
const MAX_BODY: u64 = 4 << 30;

impl Default for UreqHttp {
    fn default() -> Self {
        let agent = ureq::AgentBuilder::new()
            .user_agent(concat!("UMM/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .build();
        UreqHttp { agent }
    }
}

fn net(url: &str, e: impl std::fmt::Display) -> NetError {
    NetError {
        url: url.to_string(),
        status: None,
        message: e.to_string(),
    }
}

/// Turns an error from ureq into ours, keeping an error response's status and body.
fn call_err(url: &str, e: ureq::Error) -> NetError {
    match e {
        ureq::Error::Status(code, resp) => {
            let mut body = String::new();
            let _ = resp.into_reader().take(64 << 10).read_to_string(&mut body);
            let body = body.trim();
            NetError {
                url: url.to_string(),
                status: Some(code),
                message: if body.is_empty() || is_html(body) {
                    match status_text(code) {
                        Some(t) => format!("HTTP {code} ({t})"),
                        None => format!("HTTP {code}"),
                    }
                } else {
                    format!("HTTP {code}: {body}")
                },
            }
        }
        e => net(url, e),
    }
}

/// Whether an error response's body is a web page (a site's error page), which says nothing a
/// message can use.
fn is_html(body: &str) -> bool {
    let head = body.get(..body.len().min(256)).unwrap_or(body);
    let head = head.to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html") || head.contains("<head")
}

/// What an HTTP error status means, for the common ones.
fn status_text(code: u16) -> Option<&'static str> {
    Some(match code {
        400 => "bad request",
        401 => "not signed in",
        403 => "access denied",
        404 => "not found",
        408 => "timed out",
        429 => "too many requests",
        500 => "server error",
        502 | 504 => "the server's gateway failed",
        503 => "service unavailable",
        _ => return None,
    })
}

fn read_body(url: &str, resp: ureq::Response) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    resp.into_reader()
        .take(MAX_BODY)
        .read_to_end(&mut buf)
        .map_err(|e| net(url, e))?;
    Ok(buf)
}

/// Copies `from` into `dest` through a `.part` file beside it, renamed once complete.
fn save(
    url: &str,
    mut from: impl Read,
    total: Option<u64>,
    dest: &Path,
    progress: Progress,
) -> Result<()> {
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = std::path::PathBuf::from(part);
    let result = (|| -> Result<()> {
        let mut out = File::create(&part)
            .map_err(|e| net(url, format!("creating {}: {e}", part.display())))?;
        let mut buf = vec![0u8; 256 << 10];
        let mut done = 0u64;
        progress(0, total);
        loop {
            let n = from.read(&mut buf).map_err(|e| net(url, e))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])
                .map_err(|e| net(url, format!("writing {}: {e}", part.display())))?;
            done += n as u64;
            progress(done, total);
        }
        if total.is_some_and(|t| t != done) {
            return Err(net(url, format!("the download stopped after {done} bytes")));
        }
        out.sync_all().map_err(|e| net(url, e))?;
        drop(out);
        fs::rename(&part, dest).map_err(|e| net(url, format!("saving {}: {e}", dest.display())))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

impl Http for UreqHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        self.get_with(url, &[])
    }

    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>> {
        self.post_with(url, content_type, body, &[])
    }

    fn post_with(
        &self,
        url: &str,
        content_type: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>> {
        let mut req = self.agent.post(url).set("Content-Type", content_type);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = req.send_bytes(body).map_err(|e| call_err(url, e))?;
        read_body(url, resp)
    }

    fn get_with(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = req.call().map_err(|e| call_err(url, e))?;
        read_body(url, resp)
    }

    fn download(&self, url: &str, dest: &Path, progress: Progress) -> Result<()> {
        let resp = self.agent.get(url).call().map_err(|e| call_err(url, e))?;
        let total = resp.header("Content-Length").and_then(|v| v.parse().ok());
        save(url, resp.into_reader(), total, dest, progress)
    }
}

/// Answers from a fixed map; for tests.
#[derive(Default)]
pub struct FakeHttp {
    pub responses: Mutex<HashMap<String, Vec<u8>>>,
    /// Error answers: status and body.
    pub errors: Mutex<HashMap<String, (u16, Vec<u8>)>>,
    pub requests: Mutex<Vec<String>>,
    /// The headers of each `get_with` and `post_with` request, with its URL.
    pub headers: Mutex<Vec<(String, Vec<(String, String)>)>>,
    /// Answers to POSTs by URL and a piece of the request body: the first that matches wins.
    pub post_rules: Mutex<Vec<(String, String, Vec<u8>)>>,
    /// Bodies of each POST, with its URL.
    pub posts: Mutex<Vec<(String, String)>>,
}

impl FakeHttp {
    pub fn with(self, url: &str, body: impl Into<Vec<u8>>) -> Self {
        self.set(url, body);
        self
    }

    /// Answers `url` with an HTTP error.
    pub fn with_error(self, url: &str, status: u16, body: impl Into<Vec<u8>>) -> Self {
        self.errors
            .lock()
            .unwrap()
            .insert(url.to_string(), (status, body.into()));
        self
    }

    /// Answers POSTs to `url` whose body contains `needle`.
    pub fn with_post(self, url: &str, needle: &str, body: impl Into<Vec<u8>>) -> Self {
        self.post_rules
            .lock()
            .unwrap()
            .push((url.to_string(), needle.to_string(), body.into()));
        self
    }

    /// Adds or replaces an answer after the fake was built.
    pub fn set(&self, url: &str, body: impl Into<Vec<u8>>) {
        self.errors.lock().unwrap().remove(url);
        self.responses
            .lock()
            .unwrap()
            .insert(url.to_string(), body.into());
    }
}

impl Http for FakeHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        self.requests.lock().unwrap().push(url.to_string());
        if let Some((status, body)) = self.errors.lock().unwrap().get(url) {
            return Err(NetError {
                url: url.to_string(),
                status: Some(*status),
                message: format!("HTTP {status}: {}", String::from_utf8_lossy(body)),
            });
        }
        self.responses
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or_else(|| net(url, "no such page (test)"))
    }

    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>> {
        self.post_with(url, content_type, body, &[])
    }

    fn post_with(
        &self,
        url: &str,
        _content_type: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>> {
        let text = String::from_utf8_lossy(body).into_owned();
        self.headers.lock().unwrap().push((
            url.to_string(),
            headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ));
        self.posts
            .lock()
            .unwrap()
            .push((url.to_string(), text.clone()));
        let rule = self
            .post_rules
            .lock()
            .unwrap()
            .iter()
            .find(|(u, needle, _)| u == url && text.contains(needle.as_str()))
            .map(|(_, _, b)| b.clone());
        match rule {
            Some(b) => {
                self.requests.lock().unwrap().push(url.to_string());
                Ok(b)
            }
            None => self.get(url),
        }
    }

    fn get_with(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        self.headers.lock().unwrap().push((
            url.to_string(),
            headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ));
        self.get(url)
    }

    fn download(&self, url: &str, dest: &Path, progress: Progress) -> Result<()> {
        let body = self.get(url)?;
        let total = Some(body.len() as u64);
        save(url, body.as_slice(), total, dest, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_go_through_a_part_file() {
        let tmp = tempfile::tempdir().unwrap();
        let http = FakeHttp::default().with("https://cdn/a.7z", vec![7u8; 1000]);
        let dest = tmp.path().join("a.7z");
        let mut seen = Vec::new();
        http.download("https://cdn/a.7z", &dest, &mut |d, t| seen.push((d, t)))
            .unwrap();
        assert_eq!(fs::read(&dest).unwrap().len(), 1000);
        assert_eq!(seen.last(), Some(&(1000, Some(1000))));
        assert!(!tmp.path().join("a.7z.part").exists());

        let b = tmp.path().join("b.7z");
        assert!(http
            .download("https://cdn/missing", &b, &mut |_, _| {})
            .is_err());
        assert!(!b.exists() && !tmp.path().join("b.7z.part").exists());
    }

    #[test]
    fn errors_keep_their_status() {
        let http =
            FakeHttp::default().with_error("https://api/x", 403, r#"{"message":"premium only"}"#);
        let err = http
            .get_with("https://api/x", &[("apikey", "k")])
            .unwrap_err();
        assert_eq!(err.status, Some(403));
        assert!(err.message.contains("premium only"));
        assert_eq!(
            http.headers.lock().unwrap()[0].1,
            [("apikey".to_string(), "k".to_string())]
        );
    }
}
