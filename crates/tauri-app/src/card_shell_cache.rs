//! Host-mediated card shell resource fetch + disk cache + allowlist.
//!
//! Complete path (no degradation): remote shells/deps are fetched by the host
//! process and served to CardShellHost. Non-allowlisted URLs fail explicitly.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const RANGE_FETCH_THRESHOLD_BYTES: usize = 1024 * 1024;
const RANGE_FETCH_CHUNK_BYTES: usize = 4 * 1024 * 1024;
/// Avoid expanding large maps into base64 strings while they cross Tauri IPC.
pub const LOCAL_PROTOCOL_BINARY_THRESHOLD_BYTES: usize = 4 * 1024 * 1024;
pub const LOCAL_PROTOCOL_SCHEME: &str = "storyforge-cache";
const DESTINY_STANDARD_MAP_URL: &str = "https://i.ibb.co/07F075B/Maplite.webp";
const DESTINY_ULTRA_MAP_URL: &str = "https://i.ibb.co/wFQqdywB/Map.webp";
const ULTRA_MAP_FALLBACK_MESSAGE: &str = "超清地图源响应过慢，已暂时显示高清地图。";

// On Windows and Android Tauri/Wry exposes a registered custom protocol under
// an http localhost origin. macOS/Linux keep the native scheme origin.
#[cfg(any(target_os = "windows", target_os = "android"))]
const LOCAL_PROTOCOL_ORIGIN: &str = "http://storyforge-cache.localhost";
#[cfg(not(any(target_os = "windows", target_os = "android")))]
const LOCAL_PROTOCOL_ORIGIN: &str = "storyforge-cache://localhost";

/// Default allowlist hosts for test-card and common ST card CDNs.
pub fn default_allowed_hosts() -> Vec<String> {
    vec![
        "testingcf.jsdelivr.net".into(),
        "cdn.jsdelivr.net".into(),
        "cdnjs.cloudflare.com".into(),
        "fonts.googleapis.com".into(),
        "fonts.gstatic.com".into(),
        "files.catbox.moe".into(),
        // The Destiny card's two map sources are served from this image CDN.
        // Keep this explicit rather than allowing arbitrary image hosts.
        "i.ibb.co".into(),
        // 卿卿卡的立绘/图鉴图床（CARD-SHELL-REVIEW L5）：不在白名单时壳内
        // fetch() 硬失败且 <img> 只能直连网络（CSP 拦截、不缓存不代理）。
        // 与 i.ibb.co 同理，点名主机而非放开任意图床。
        "i.postimg.cc".into(),
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
    /// Large binary assets are served by the local `storyforge-cache` protocol
    /// so they never cross the Tauri IPC bridge as a base64 string.
    pub cache_url: Option<String>,
    /// An optional user-visible notice supplied when a known optional asset
    /// intentionally falls back to its cached standard-resolution equivalent.
    pub fallback_message: Option<String>,
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
            // The card's optional ultra map is about 31.5 MiB. Retain a hard
            // ceiling while allowing the known, allowlisted visual resource.
            max_bytes: 40 * 1024 * 1024,
            timeout: Duration::from_secs(30),
        }
    }

    #[allow(dead_code)]
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// 清空磁盘缓存（L6：未 pin 依赖首取即冻结，需要手动刷新通道）。
    /// 只清 card-shell-cache 目录自身内容，返回清掉的对象数。
    pub fn clear_cache(&self) -> Result<usize, String> {
        let entries =
            std::fs::read_dir(&self.cache_dir).map_err(|e| format!("read cache dir: {e}"))?;
        let mut removed = 0usize;
        for entry in entries.flatten() {
            let path = entry.path();
            let ok = if path.is_dir() {
                std::fs::remove_dir_all(&path).is_ok()
            } else {
                std::fs::remove_file(&path).is_ok()
            };
            if ok {
                removed += 1;
            }
        }
        Ok(removed)
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

    /// Validate each concrete request URL, including every redirect target.
    /// Card shells only need named, allowlisted CDN hosts; literal IP URLs are
    /// never valid and would turn this fetcher into an SSRF primitive.
    fn validate_fetch_url(&self, url: &str) -> Result<(), String> {
        let host = host_of(url).ok_or_else(|| format!("cannot parse URL host: {url}"))?;
        if host.parse::<std::net::IpAddr>().is_ok() {
            return Err(format!("IP-literal shell URL is not allowed: {url}"));
        }
        self.is_url_allowed(url)
    }

    fn cache_path_for_url(&self, url: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        let dig = hasher.finalize();
        let hex = dig.iter().map(|b| format!("{b:02x}")).collect::<String>();
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

    pub fn write_cache(
        &self,
        url: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, String> {
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

    /// Read a generated cache object for the local WebView protocol. This
    /// accepts only an opaque cache filename, never an arbitrary disk path.
    pub fn read_protocol_resource(&self, resource_name: &str) -> Result<(Vec<u8>, String), String> {
        if !is_safe_cache_resource_name(resource_name) {
            return Err("invalid card-shell cache resource".into());
        }
        let path = self.cache_dir.join(resource_name);
        let meta_path = path.with_extension("meta.json");
        let meta_raw = std::fs::read_to_string(&meta_path)
            .map_err(|_| "card-shell cache resource not found".to_string())?;
        let meta: serde_json::Value = serde_json::from_str(&meta_raw)
            .map_err(|_| "card-shell cache metadata is invalid".to_string())?;
        let content_type = meta
            .get("content_type")
            .and_then(|value| value.as_str())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes =
            std::fs::read(&path).map_err(|_| "card-shell cache resource not found".to_string())?;
        Ok((bytes, content_type))
    }

    /// The upstream 31 MiB Destiny ultra map routinely stalls for minutes on
    /// the image host. The card has a normal-resolution sibling and always
    /// opens that first, so serve the cached sibling immediately rather than
    /// freezing the map on a request the user cannot cancel or observe.
    fn cached_optional_map_fallback(&self, url: &str) -> Option<ShellFetchResult> {
        if url != DESTINY_ULTRA_MAP_URL {
            return None;
        }
        let (path, bytes, content_type) = self.read_cache(DESTINY_STANDARD_MAP_URL)?;
        let mut result = to_result(url, content_type, bytes, path, true);
        result.fallback_message = Some(ULTRA_MAP_FALLBACK_MESSAGE.into());
        Some(result)
    }

    /// Fetch URL via host HTTP, with allowlist + cache. Text is for unit tests.
    pub fn fetch_blocking_with_client(
        &self,
        url: &str,
        client: &reqwest::blocking::Client,
    ) -> Result<ShellFetchResult, String> {
        self.validate_fetch_url(url)?;
        // 自身缓存命中优先于标准图兜底：超清图一旦成功缓存过（如在标准图
        // 之前抓取），必须能命中自己的缓存——否则标准图先缓存后超清图永不
        // 可达（CARD-SHELL-REVIEW L1）。防卡死语义保留：仅在超清图未缓存
        // 时才用标准图顶替，绝不为它发起分钟级网络请求。
        if let Some((path, bytes, content_type)) = self.read_cache(url) {
            return Ok(to_result(url, content_type, bytes, path, true));
        }
        if let Some(result) = self.cached_optional_map_fallback(url) {
            return Ok(result);
        }
        let (_final_url, resp) = self.send_checked_request(client, url, None)?;
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
        let expected_bytes = resp
            .content_length()
            .and_then(|length| usize::try_from(length).ok());
        if expected_bytes.is_some_and(|length| length > self.max_bytes) {
            return Err(format!(
                "shell resource too large: {} bytes (max {}) url={url}",
                expected_bytes.unwrap_or_default(),
                self.max_bytes
            ));
        }
        let bytes = if let Some(length) = expected_bytes {
            if should_fetch_by_ranges(url, length) {
                fetch_by_ranges(self, client, url, length)?
            } else {
                read_limited_body(resp, self.max_bytes)?
            }
        } else {
            read_limited_body(resp, self.max_bytes)?
        };
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
            // Redirects are followed explicitly in `send_checked_request` so
            // every hop passes the host and IP validation above.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("build shell http client: {e}"))
    }

    fn send_checked_request(
        &self,
        client: &reqwest::blocking::Client,
        url: &str,
        range: Option<&str>,
    ) -> Result<(String, reqwest::blocking::Response), String> {
        let mut current = url.to_string();
        for redirect_count in 0..=5 {
            self.validate_fetch_url(&current)?;
            let mut request = client.get(&current);
            if let Some(value) = range {
                request = request.header(reqwest::header::RANGE, value);
            }
            let response = request
                .send()
                .map_err(|e| format!("shell fetch network error: {e}"))?;
            if !response.status().is_redirection() {
                return Ok((current, response));
            }
            if redirect_count == 5 {
                return Err(format!("shell fetch exceeded redirect limit: {url}"));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| format!("shell redirect has no valid Location: {current}"))?;
            let base = reqwest::Url::parse(&current)
                .map_err(|e| format!("invalid shell redirect base URL {current}: {e}"))?;
            current = base
                .join(location)
                .map_err(|e| format!("invalid shell redirect target {location}: {e}"))?
                .to_string();
        }
        Err(format!("shell fetch exceeded redirect limit: {url}"))
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
    let cache_url = (!is_text && bytes.len() > LOCAL_PROTOCOL_BINARY_THRESHOLD_BYTES)
        .then(|| cache_protocol_url_for_path(&path))
        .flatten();
    let (body_text, body_base64) = if is_text {
        match String::from_utf8(bytes.clone()) {
            Ok(s) => (Some(s), None),
            Err(_) => (None, Some(b64(&bytes))),
        }
    } else if cache_url.is_some() {
        (None, None)
    } else {
        (None, Some(b64(&bytes)))
    };
    ShellFetchResult {
        url: url.to_string(),
        content_type,
        body_text,
        body_base64,
        cache_url,
        fallback_message: None,
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

fn cache_protocol_url_for_path(path: &Path) -> Option<String> {
    let resource_name = path.file_name()?.to_str()?;
    is_safe_cache_resource_name(resource_name)
        .then(|| format!("{LOCAL_PROTOCOL_ORIGIN}/{resource_name}"))
}

fn is_safe_cache_resource_name(resource_name: &str) -> bool {
    let bytes = resource_name.as_bytes();
    bytes.len() >= 64
        && bytes[..64].iter().all(u8::is_ascii_hexdigit)
        && bytes[64..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// ibb reliably serves the card's large WebP maps by byte range in the
/// embedded app, while a single full-body request can stall after the headers.
/// Keep the workaround restricted to that allowlisted host and substantial
/// images; all other card resources retain the usual one-request path.
fn should_fetch_by_ranges(url: &str, byte_len: usize) -> bool {
    byte_len > RANGE_FETCH_THRESHOLD_BYTES && host_of(url).as_deref() == Some("i.ibb.co")
}

fn fetch_by_ranges(
    cache: &CardShellCache,
    client: &reqwest::blocking::Client,
    url: &str,
    expected_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(expected_bytes);
    let mut start = 0usize;
    while start < expected_bytes {
        let end_exclusive = (start + RANGE_FETCH_CHUNK_BYTES).min(expected_bytes);
        let end_inclusive = end_exclusive - 1;
        let range = format!("bytes={start}-{end_inclusive}");
        let (_final_url, response) = cache.send_checked_request(client, url, Some(&range))?;
        if !response.status().is_success() {
            return Err(format!(
                "shell range fetch HTTP {}: {url}",
                response.status().as_u16()
            ));
        }
        let expected_chunk_len = end_exclusive - start;
        let chunk = read_limited_body(response, expected_chunk_len)?;
        if chunk.len() != expected_chunk_len {
            return Err(format!(
                "shell range fetch returned {} bytes, expected {expected_chunk_len}: {url}",
                chunk.len()
            ));
        }
        bytes.extend_from_slice(&chunk);
        start = end_exclusive;
    }
    Ok(bytes)
}

fn read_limited_body<R: Read>(reader: R, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    let mut limited = reader.take(max_bytes.saturating_add(1) as u64);
    limited
        .read_to_end(&mut body)
        .map_err(|e| format!("shell fetch body error: {e}"))?;
    if body.len() > max_bytes {
        return Err(format!(
            "shell resource too large: more than {max_bytes} bytes"
        ));
    }
    Ok(body)
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
        assert!(
            cache
                .is_url_allowed("https://files.catbox.moe/a.png")
                .is_ok()
        );
        cache.allow_host("example.com");
        assert!(cache.is_url_allowed("https://example.com/a.js").is_ok());
    }

    #[test]
    fn defaults_allow_the_destiny_card_map_host_at_a_safe_map_size_limit() {
        let dir = tempdir().unwrap();
        let cache = CardShellCache::new(dir.path());

        assert!(
            cache
                .is_url_allowed("https://i.ibb.co/07F075B/Maplite.webp")
                .is_ok()
        );
        assert_eq!(cache.max_bytes, 40 * 1024 * 1024);
    }

    #[test]
    fn defaults_allow_the_qingqing_card_image_host() {
        // L5：卿卿卡立绘/图鉴走 i.postimg.cc，白名单缺失时壳内 fetch 硬失败
        let dir = tempdir().unwrap();
        let cache = CardShellCache::new(dir.path());
        assert!(
            cache
                .is_url_allowed("https://i.postimg.cc/abc123/portrait.png")
                .is_ok()
        );
    }

    #[test]
    fn keeps_large_binary_maps_out_of_the_ipc_base64_payload() {
        let dir = tempdir().unwrap();
        let cache = CardShellCache::new(dir.path());
        let path = cache.cache_path_for_url("https://i.ibb.co/example/Map.webp");
        let resource_name = path.file_name().unwrap().to_string_lossy().to_string();
        let result = to_result(
            "https://i.ibb.co/example/Map.webp",
            "image/webp".into(),
            vec![0; LOCAL_PROTOCOL_BINARY_THRESHOLD_BYTES + 1],
            path,
            false,
        );

        assert!(result.body_base64.is_none());
        assert_eq!(
            result.cache_url.as_deref(),
            Some(format!("http://storyforge-cache.localhost/{resource_name}").as_str())
        );
    }

    #[test]
    fn local_protocol_reads_only_safe_cache_resource_names() {
        let dir = tempdir().unwrap();
        let cache = CardShellCache::new(dir.path());
        let path = cache
            .write_cache(
                "https://i.ibb.co/example/Map.webp",
                "image/webp",
                &[1, 2, 3],
            )
            .unwrap();
        let resource_name = path.file_name().unwrap().to_string_lossy().to_string();

        assert_eq!(
            cache.read_protocol_resource(&resource_name).unwrap(),
            (vec![1, 2, 3], "image/webp".into())
        );
        assert!(
            cache
                .read_protocol_resource("../../campaigns.json")
                .is_err()
        );
    }

    #[test]
    fn optional_ultra_map_uses_the_cached_standard_map_with_a_visible_notice() {
        let dir = tempdir().unwrap();
        let cache = CardShellCache::new(dir.path());
        cache
            .write_cache(
                DESTINY_STANDARD_MAP_URL,
                "image/webp",
                &vec![0; LOCAL_PROTOCOL_BINARY_THRESHOLD_BYTES + 1],
            )
            .unwrap();

        let result = cache
            .cached_optional_map_fallback(DESTINY_ULTRA_MAP_URL)
            .expect("a cached standard map fallback");

        assert_eq!(result.url, DESTINY_ULTRA_MAP_URL);
        assert_eq!(
            result.fallback_message.as_deref(),
            Some(ULTRA_MAP_FALLBACK_MESSAGE)
        );
        assert!(result.cache_url.is_some());
        assert!(result.body_base64.is_none());
    }

    #[test]
    fn clear_cache_removes_objects_and_keeps_directory_usable() {
        let dir = tempdir().unwrap();
        let cache = CardShellCache::new(dir.path());
        cache
            .write_cache("https://i.ibb.co/example/a.webp", "image/webp", &[1, 2])
            .unwrap();
        cache
            .write_cache("https://i.ibb.co/example/b.webp", "image/webp", &[3, 4])
            .unwrap();

        let removed = cache.clear_cache().unwrap();
        assert!(removed >= 2, "对象与 meta 都应清掉, removed={removed}");
        assert!(
            cache
                .read_cache("https://i.ibb.co/example/a.webp")
                .is_none()
        );

        // 清空后目录仍可写
        cache
            .write_cache("https://i.ibb.co/example/c.webp", "image/webp", &[5])
            .unwrap();
        assert!(
            cache
                .read_cache("https://i.ibb.co/example/c.webp")
                .is_some()
        );
    }

    #[test]
    fn cached_ultra_map_wins_over_standard_map_fallback() {
        // L1 回归：超清图自身已缓存时必须命中自己的缓存，
        // 不得被标准图兜底劫持。
        let dir = tempdir().unwrap();
        let cache = CardShellCache::new(dir.path());
        cache
            .write_cache(DESTINY_STANDARD_MAP_URL, "image/webp", &[1u8; 8])
            .unwrap();
        cache
            .write_cache(DESTINY_ULTRA_MAP_URL, "image/webp", &[2u8; 8])
            .unwrap();

        let client = cache.build_client().unwrap();
        let result = cache
            .fetch_blocking_with_client(DESTINY_ULTRA_MAP_URL, &client)
            .expect("cached ultra map should be served");

        assert_eq!(result.url, DESTINY_ULTRA_MAP_URL);
        assert!(result.fallback_message.is_none(), "缓存命中不应带兜底提示");
        assert_eq!(
            result.body_base64.as_deref(),
            Some(b64(&[2u8; 8]).as_str()),
            "应返回超清图自身字节而非标准图"
        );
    }

    #[test]
    fn uses_byte_ranges_for_large_destiny_map_cdn_images() {
        assert!(should_fetch_by_ranges(
            "https://i.ibb.co/07F075B/Maplite.webp",
            7 * 1024 * 1024,
        ));
        assert!(!should_fetch_by_ranges(
            "https://testingcf.jsdelivr.net/npm/openseadragon/+esm",
            7 * 1024 * 1024,
        ));
        assert!(!should_fetch_by_ranges(
            "https://i.ibb.co/07F075B/Maplite.webp",
            1024,
        ));
    }

    #[test]
    fn rejects_ip_literal_redirect_targets_and_limits_unknown_length_bodies() {
        let dir = tempdir().unwrap();
        let cache = CardShellCache::new(dir.path());
        assert!(
            cache
                .validate_fetch_url("http://127.0.0.1/internal")
                .is_err()
        );

        let body = std::io::Cursor::new(vec![0u8; 9]);
        assert!(read_limited_body(body, 8).is_err());
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
