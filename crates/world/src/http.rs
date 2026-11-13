//! HTTP abstraction kept deliberately tiny so the weather/geo logic is testable
//! offline.  Only [`UreqClient`] touches the network; everything else takes a
//! `&dyn HttpClient`.

#[cfg(feature = "net")]
use std::time::Duration;

#[cfg(feature = "net")]
const TIMEOUT: Duration = Duration::from_secs(10);
/// UA prefix — full agent string is `citylive-iso/<version>`.
#[cfg(feature = "net")]
const UA_PREFIX: &str = "citylive-iso";

/// Errors from a blocking HTTP GET.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// Transport / TLS / no response.
    Network(String),
    /// The server returned a non-2xx status.
    BadStatus(u16),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Network(s) => write!(f, "network error: {s}"),
            HttpError::BadStatus(c) => write!(f, "bad status {c}"),
        }
    }
}
impl std::error::Error for HttpError {}

/// Minimal blocking GET.  `url` must be absolute.
pub trait HttpClient {
    fn get(&self, url: &str) -> Result<String, HttpError>;
}

/// Real client: `ureq` with rustls, 10 s timeout, `citylive-iso/<version>` UA.
/// Behind the `net` feature (B20 item 5) so a static musl build stays possible.
/// The real client: a **pure-Rust HTTPS GET** (B27 item 1).
///
/// `ureq` + `ring` needed a C compiler for musl (`x86_64-linux-musl-gcc`), which
/// defeats a static build. This is a blocking HTTP/1.1 GET over `rustls` with
/// the RustCrypto provider and `webpki-roots`, so it builds with `rustc` alone:
///
/// * TLS 1.3 (and 1.2 when the server insists) with a bundled root store;
/// * one request per connection (`Connection: close`), which is what a 15-minute
///   weather poll wants anyway;
/// * 10 s timeout, `citylive-iso/<version>` user agent, gzip **not** requested
///   (the payloads are a few KB).
#[cfg(feature = "net")]
pub struct UreqClient {
    cfg: std::sync::Arc<rustls::ClientConfig>,
}

#[cfg(feature = "net")]
impl UreqClient {
    pub fn new() -> Self {
        // First call wins; a second client reuses whatever is installed.
        let _ = rustls::crypto::CryptoProvider::install_default(rustls_rustcrypto::provider());
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let cfg = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Self {
            cfg: std::sync::Arc::new(cfg),
        }
    }
}

#[cfg(feature = "net")]
impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "net")]
impl UreqClient {
    /// One HTTPS request, the raw bytes of the answer (status line, headers,
    /// body). `max_body` caps what is read.
    fn exchange(&self, url: &str, headers: &[(&str, &str)], max_body: u64) -> Result<Vec<u8>, HttpError> {
        use std::io::{Read, Write};
        let (scheme, rest) = url
            .split_once("://")
            .ok_or_else(|| HttpError::Network("url without scheme".into()))?;
        if scheme != "https" {
            return Err(HttpError::Network(format!("only https:// is supported, got {scheme}")));
        }
        let (host_port, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = match host_port.rsplit_once(':') {
            Some((h, p)) => (
                h.to_string(),
                p.parse::<u16>().map_err(|_| HttpError::Network("bad port".into()))?,
            ),
            None => (host_port.to_string(), 443u16),
        };
        let server = rustls::pki_types::ServerName::try_from(host.clone())
            .map_err(|_| HttpError::Network(format!("bad host name '{host}'")))?;
        let mut conn = rustls::ClientConnection::new(self.cfg.clone(), server)
            .map_err(|e| HttpError::Network(format!("tls: {e}")))?;
        let mut sock = std::net::TcpStream::connect((host.as_str(), port))
            .map_err(|e| HttpError::Network(format!("connect: {e}")))?;
        sock.set_read_timeout(Some(TIMEOUT)).ok();
        sock.set_write_timeout(Some(TIMEOUT)).ok();
        let mut tls = rustls::Stream::new(&mut conn, &mut sock);
        let mut req = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {UA_PREFIX}/{}\r\nConnection: close\r\n",
            env!("CARGO_PKG_VERSION")
        );
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept")) {
            req.push_str("Accept: */*\r\n");
        }
        for (k, v) in headers {
            // Header values come from our own code, never from the network.
            if k.contains(['\r', '\n']) || v.contains(['\r', '\n']) {
                return Err(HttpError::Network("bad header".into()));
            }
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        tls.write_all(req.as_bytes())
            .map_err(|e| HttpError::Network(format!("write: {e}")))?;
        let mut raw = Vec::with_capacity(8192);
        // Cap the body: a misbehaving server must not eat the heap.
        (&mut tls)
            .take(max_body)
            .read_to_end(&mut raw)
            .map_err(|e| HttpError::Network(format!("read: {e}")))?;
        Ok(raw)
    }

    /// A GET with extra request headers and a binary body; any status is
    /// returned (a 304 or a 302 is an answer, not an error). Used by the
    /// updater (B97), which follows redirects itself.
    pub fn request(&self, url: &str, headers: &[(&str, &str)], max_body: u64) -> Result<RawResponse, HttpError> {
        let raw = self.exchange(url, headers, max_body)?;
        parse_raw(&raw)
    }
}

#[cfg(feature = "net")]
impl HttpClient for UreqClient {
    fn get(&self, url: &str) -> Result<String, HttpError> {
        let raw = self.exchange(url, &[], 8 << 20)?;
        parse_response(&raw)
    }
}

/// A raw HTTP answer: status, headers (names lower-cased) and the body bytes
/// (chunked framing removed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RawResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Parse a whole raw answer into a [`RawResponse`].
pub fn parse_raw(raw: &[u8]) -> Result<RawResponse, HttpError> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| HttpError::Network("no header terminator".into()))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let body = &raw[split + 4..];
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| HttpError::Network("no status line".into()))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.split(',').any(|t| t.trim().eq_ignore_ascii_case("chunked")));
    let body = if chunked { dechunk_bytes(body) } else { body.to_vec() };
    Ok(RawResponse { status, headers, body })
}

/// Split a raw HTTP/1.1 response into status + body (no chunked support: the two
/// APIs we talk to answer with `Content-Length`).
#[cfg(feature = "net")]
fn parse_response(raw: &[u8]) -> Result<String, HttpError> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| HttpError::Network("no header terminator".into()))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let body = &raw[split + 4..];
    let status: u16 = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| HttpError::Network("no status line".into()))?;
    if !(200..300).contains(&status) {
        return Err(HttpError::BadStatus(status));
    }
    // Chunked detection per header line, tolerant of case/whitespace and of
    // repeated lines (some CDNs — e.g. the one fronting site.api.espn.com —
    // answer with `Transfer-Encoding:  chunked` doubled spaces, which a raw
    // substring match misses; that silently left the raw chunk framing in
    // the body and the JSON unparsable).
    let chunked = head.lines().any(|line| {
        let Some((k, v)) = line.split_once(':') else {
            return false;
        };
        k.trim().eq_ignore_ascii_case("transfer-encoding")
            && v
                .split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("chunked"))
    });
    if chunked {
        return Ok(dechunk(body));
    }
    Ok(String::from_utf8_lossy(body).to_string())
}

/// Minimal chunked-transfer decoder (HTTP/1.1 default when no length is given).
fn dechunk_bytes(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < body.len() {
        let Some(eol) = body[i..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let size_str = String::from_utf8_lossy(&body[i..i + eol]).to_string();
        let size_str = size_str.split(';').next().unwrap_or("").trim().to_string();
        let Ok(size) = usize::from_str_radix(&size_str, 16) else {
            break;
        };
        if size == 0 {
            break;
        }
        let start = i + eol + 2;
        let end = start.saturating_add(size).min(body.len());
        out.extend_from_slice(&body[start.min(body.len())..end]);
        i = end.saturating_add(2);
    }
    out
}

#[cfg(feature = "net")]
fn dechunk(body: &[u8]) -> String {
    String::from_utf8_lossy(&dechunk_bytes(body)).to_string()
}

#[cfg(test)]
pub mod tests {
    #[cfg(feature = "net")]
    #[test]
    fn dechunk_survives_absurd_chunk_sizes() {
        assert_eq!(dechunk(b"ffffffffffffffff\r\nabc"), "abc");
        assert_eq!(dechunk(b"3\r\nabc\r\n0\r\n\r\n"), "abc");
        assert_eq!(dechunk(b"10\r\nab"), "ab");
    }

    use super::*;
    use std::collections::HashMap;

    #[test]
    fn raw_response_headers_and_chunks() {
        let raw = b"HTTP/1.1 200 OK\r\nETag: \"abc\"\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\n\xff\x00\r\n0\r\n\r\n";
        let r = parse_raw(raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.header("etag"), Some("\"abc\""));
        assert_eq!(r.body, b"abc\xff\x00");
        let r = parse_raw(b"HTTP/1.1 304 Not Modified\r\n\r\n").unwrap();
        assert_eq!((r.status, r.body.len()), (304, 0));
    }
    use std::sync::Mutex;

    /// In-memory client: maps URL → body or error, and records every call.
    pub struct MapClient {
        pub map: HashMap<String, Result<String, HttpError>>,
        pub calls: Mutex<Vec<String>>,
    }

    impl MapClient {
        pub fn new() -> Self {
            Self {
                map: HashMap::new(),
                calls: Mutex::new(Vec::new()),
            }
        }
        pub fn ok(self, url: &str, body: &str) -> Self {
            let mut m = self;
            m.map.insert(url.to_string(), Ok(body.to_string()));
            m
        }
        pub fn with_log(mut self, url: &str, res: Result<&str, &str>) -> Self {
            self.map.insert(
                url.to_string(),
                match res {
                    Ok(b) => Ok(b.to_string()),
                    Err(e) => Err(HttpError::Network(e.to_string())),
                },
            );
            self
        }
        pub fn called_urls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl HttpClient for MapClient {
        fn get(&self, url: &str) -> Result<String, HttpError> {
            self.calls.lock().unwrap().push(url.to_string());
            self.map
                .get(url)
                .cloned()
                .unwrap_or_else(|| Err(HttpError::Network("not found".into())))
        }
    }
}
