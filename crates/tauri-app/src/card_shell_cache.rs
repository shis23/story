//! Host-mediated card shell resource fetch + disk cache + allowlist.
//!
//! Complete path (no degradation): remote shells/deps are fetched by the host
//! process and served to CardShellHost. Non-allowlisted URLs fail explicitly.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Default allowlist hosts for test-card and common ST card CDNs.
pub fn default_allowed_hosts() -> Vec<String> {
    vec![
        "testingcf.jsdelivr.net".into(),
        "cdn.jsdelivr.net".into(),
        "cdnjs.cloudflare.com".into(),
        "fonts.googleapis.com".into(),
        "fonts.gstatic.com".into(),
        "files.catbox.moe".into(),
        "raw.githubusercontent.com".into(),
        "github.com".into(),
        "gitee.com".into(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellFetchResult {
    pub url: String,
    pub content_type: String,
    pub body_text: Option<String>,
    pub body_base64: Option<String>,
    pub cached_path: String,
    pub from_cache: bool,
    pub byte_len: usize,
}

#[derive(Debug)]
pub struct CardShellCache {
    cache_dir: PathBuf,
    allowed_hosts: Mutex<HashSet<String>>,
    max_bytes: usize,
    timeout: Duration,
}

impl CardShellCache {
    pub fn new(data_dir: &Path) -> Self {
        let cache_dir = data_dir.join("card-shell-cache");
        let _ = std::fs::create_dir_all(&cache_dir);
        let allowed: HashSet<String> = default_allowed_hosts().into_iter().collect();
        Self {
            cache_dir,
            allowed_hosts: Mutex::new(allowed),
            max_bytes: 12 * 1024 * 1024,
            timeout: Duration::from_secs(30),
        }
    }

    #[allow(dead_code)]
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn list_allowed_hosts(&self) -> Vec<String> {
        let guard = self.allowed_hosts.lock().unwrap_or_else(|p| p.into_inner());
        let mut v: Vec<_> = guard.iter().cloned().collect();
        v.sort();
        v
    }

    pub fn allow_host(&self, host: &str) {
        let host = host.trim().to_ascii_lowercase();
        if host.is_empty() {
            return;
        }
        let mut guard = self.allowed_hosts.lock().unwrap_or_else(|p| p.into_inner());
        guard.insert(host);
    }

    pub fn is_url_allowed(&self, url: &str) -> Result<(), String> {
        let host = host_of(url).ok_or_else(|| format!("无法解析 URL host: {url}"))?;
        let guard = self.allowed_hosts.lock().unwrap_or_else(|p| p.into_inner());
        if guard.contains(&host) {
            Ok(())
        } else {
            Err(format!(
                "URL host 不在 allowlist: {host}（url={url}）。可在设置中始终允许该 host。"
            ))
        }
    }

    fn cache_path_for_url(&self, url: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        let dig = hasher.finalize();
        let hex = dig
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        // Keep a short readable suffix for debugging
        let suffix = url
            .rsplit('/')
            .next()
            .unwrap_or("bin")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
            .take(48)
            .collect::<String>();
        let name = if suffix.is_empty() {
            hex
        } else {
            format!("{hex}_{suffix}")
        };
        self.cache_dir.join(name)
    }

    pub fn read_cache(&self, url: &str) -> Option<(PathBuf, Vec<u8>, String)> {
        let path = self.cache_path_for_url(url);
        let meta_path = path.with_extension("meta.json");
        if !path.exists() || !meta_path.exists() {
            return None;
        }
        let meta_raw = std::fs::read_to_string(&meta_path).ok()?;
        let meta: serde_json::Value = serde_json::from_str(&meta_raw).ok()?;
        let content_type = meta
            .get("content_type")
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = std::fs::read(&path).ok()?;
        Some((path, bytes, content_type))
    }

    pub fn write_cache(&self, url: &str, content_type: &str, bytes: &[u8]) -> Result<PathBuf, String> {
        let _ = std::fs::create_dir_all(&self.cache_dir);
        let path = self.cache_path_for_url(url);
        std::fs::write(&path, bytes).map_err(|e| format!("write cache failed: {e}"))?;
        let meta = serde_json::json!({
            "url": url,
            "content_type": content_type,
            "byte_len": bytes.len(),
        });
        let meta_path = path.with_extension("meta.json");
        std::fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&meta).unwrap_or_default(),
        )
        .map_err(|e| format!("write cache meta failed: {e}"))?;
        Ok(path)
    }

    /// Fetch URL via host HTTP, with allowlist + cache. Text is for unit tests.
    pub fn fetch_blocking_with_client(
        &self,
        url: &str,
        client: &reqwest::blocking::Client,
    ) -> Result<ShellFetchResult, String> {
        self.is_url_allowed(url)?;
        if let Some((path, bytes, content_type)) = self.read_cache(url) {
            return Ok(to_result(url, content_type, bytes, path, true));
        }
        let resp = client
            .get(url)
            .send()
            .map_err(|e| format!("shell fetch network error: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "shell fetch HTTP {}: {url}",
                resp.status().as_u16()
            ));
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = resp
            .bytes()
            .map_err(|e| format!("shell fetch body error: {e}"))?
            .to_vec();
        if bytes.len() > self.max_bytes {
            return Err(format!(
                "shell resource too large: {} bytes (max {}) url={url}",
                bytes.len(),
                self.max_bytes
            ));
        }
        let path = self.write_cache(url, &content_type, &bytes)?;
        Ok(to_result(url, content_type, bytes, path, false))
    }

    pub fn build_client(&self) -> Result<reqwest::blocking::Client, String> {
        reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .user_agent("StoryForge-CardShell/0.1")
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| format!("build shell http client: {e}"))
    }
}

fn to_result(
    url: &str,
    content_type: String,
    bytes: Vec<u8>,
    path: PathBuf,
    from_cache: bool,
) -> ShellFetchResult {
    let is_text = content_type.starts_with("text/")
        || content_type.contains("javascript")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("svg")
        || url.ends_with(".js")
        || url.ends_with(".css")
        || url.ends_with(".html")
        || url.ends_with(".htm")
        || url.ends_with(".mjs");
    let (body_text, body_base64) = if is_text {
        match String::from_utf8(bytes.clone()) {
            Ok(s) => (Some(s), None),
            Err(_) => (None, Some(b64(&bytes))),
        }
    } else {
        (None, Some(b64(&bytes)))
    };
    ShellFetchResult {
        url: url.to_string(),
        content_type,
        body_text,
        body_base64,
        cached_path: path.display().to_string(),
        from_cache,
        byte_len: bytes.len(),
    }
}

fn b64(bytes: &[u8]) -> String {
    use std::io::Write;
    // minimal base64 without extra dep: use a tiny encoder
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((triple >> 18) & 63) as usize]);
        out.push(T[((triple >> 12) & 63) as usize]);
        if chunk.len() > 1 {
            out.push(T[((triple >> 6) & 63) as usize]);
        } else {
            out.push(b'=');
        }
        if chunk.len() > 2 {
            out.push(T[(triple & 63) as usize]);
        } else {
            out.push(b'=');
        }
    }
    let _ = out.flush();
    String::from_utf8(out).unwrap_or_default()
}

pub fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split('/').next()?.split('@').next_back()?;
    let host = host.split(':').next()?.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn allowlist_blocks_unknown_host() {
        let dir = tempdir().unwrap();
        let cache = CardShellCache::new(dir.path());
        let err = cache
            .is_url_allowed("https://evil.example/x.js")
            .unwrap_err();
        assert!(err.contains("allowlist"));
        assert!(cache
            .is_url_allowed(
                "https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/home/index.html"
            )
            .is_ok());
    }

    #[test]
    fn allow_host_extends_list() {
        let dir = tempdir().unwrap();
        let cache = CardShellCache::new(dir.path());
        assert!(cache.is_url_allowed("https://files.catbox.moe/a.png").is_ok());
        cache.allow_host("example.com");
        assert!(cache.is_url_allowed("https://example.com/a.js").is_ok());
    }

    #[test]
    fn host_parser() {
        assert_eq!(
            host_of("https://cdn.jsdelivr.net/npm/x").as_deref(),
            Some("cdn.jsdelivr.net")
        );
        assert_eq!(host_of("not-a-url"), None);
    }
}
