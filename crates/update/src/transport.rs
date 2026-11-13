//! The one network seam: tests give a fixture, the program gives HTTPS.

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub etag: Option<String>,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

pub trait Transport {
    /// One request; any HTTP status is an answer. `Err` is a network failure.
    fn get(&self, url: &str, headers: &[(&str, &str)], max_body: u64) -> Result<Reply, String>;
}

/// Redirect targets must stay on GitHub's own hosts, over https.
pub fn redirect_allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', ':', '?']).next().unwrap_or("");
    host == "github.com"
        || host.ends_with(".github.com")
        || host == "githubusercontent.com"
        || host.ends_with(".githubusercontent.com")
}

/// GET that follows up to five redirects (release downloads redirect to a
/// storage host).
pub fn get_following(
    t: &dyn Transport,
    url: &str,
    headers: &[(&str, &str)],
    max_body: u64,
) -> Result<Reply, String> {
    let mut url = url.to_string();
    for _ in 0..6 {
        let r = t.get(&url, headers, max_body)?;
        if matches!(r.status, 301 | 302 | 303 | 307 | 308) {
            let next = r.location.ok_or("redirect without a location")?;
            if !redirect_allowed(&next) {
                return Err(format!("redirect to a foreign host refused: {next}"));
            }
            url = next;
            continue;
        }
        return Ok(r);
    }
    Err("too many redirects".into())
}

/// The real thing: the pure-Rust HTTPS client shared with the weather lookup.
#[cfg(feature = "net")]
pub struct HttpsTransport(world::UreqClient);

#[cfg(feature = "net")]
impl HttpsTransport {
    pub fn new() -> Self {
        Self(world::UreqClient::new())
    }
}

#[cfg(feature = "net")]
impl Default for HttpsTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "net")]
impl Transport for HttpsTransport {
    fn get(&self, url: &str, headers: &[(&str, &str)], max_body: u64) -> Result<Reply, String> {
        let r = self.0.request(url, headers, max_body).map_err(|e| e.to_string())?;
        if let Some(len) = r.header("content-length").and_then(|v| v.parse::<usize>().ok()) {
            if r.status == 200 && len != r.body.len() {
                return Err(format!("truncated download ({} of {len} bytes)", r.body.len()));
            }
        }
        Ok(Reply {
            status: r.status,
            etag: r.header("etag").map(str::to_string),
            location: r.header("location").map(str::to_string),
            body: r.body,
        })
    }
}
