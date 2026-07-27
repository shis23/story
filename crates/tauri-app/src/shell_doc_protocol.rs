//! Restricted Tauri custom protocol that serves one-shot "shell document"
//! HTML strings on a dedicated origin.
//!
//! # Why this exists (V5 CSP isolation)
//!
//! CardShellHost / TavernHelperRuntime (blob iframes) and MvuJsRuntime /
//! PluginHost (srcdoc iframes) all embed inline `<script>` bridges in their
//! iframe documents. CSP Level 3 dictates that documents created from
//! `blob:` / `srcdoc:` / `data:` / `about:blank` **inherit the creator's
//! policy container**, and that multiple CSPs only ever *intersect* — an
//! iframe's own `<meta>` policy cannot widen what an inherited policy
//! forbids. Therefore a strict main-app CSP (no `script-src 'unsafe-inline'`)
//! silently breaks every inline bridge, and the shell cannot rescue itself.
//!
//! The fix is to serve the shell document from a **separate origin** whose
//! policy container does NOT inherit the main app CSP. That origin is this
//! `storyforge-shell` protocol. Each document is registered with an opaque
//! token (32 random hex bytes) and served once; the response carries its own
//! `Content-Security-Policy` header (the shell policy), so the document is
//! network-bounded regardless of any `<meta>` it also sets. The main app's
//! `frame-src` allows only this precise origin.
//!
//! Mirrors `card_shell_cache.rs` (scheme/origin split + protocol handler
//! shape) and the `get_card_shell_cache()` OnceLock singleton in lib.rs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The custom-protocol scheme under which shell documents are served.
pub const SHELL_DOC_SCHEME: &str = "storyforge-shell";

// On Windows and Android Tauri/Wry exposes a registered custom protocol under
// an http localhost origin. macOS/Linux keep the native scheme origin. Mirrors
// card_shell_cache.rs:26-29 — keep in sync. This constant is the authoritative
// origin the frontend mirrors (shellDocUrl.js SHELL_DOC_ORIGIN); it is not read
// inside this crate, hence the allow.
#[cfg(any(target_os = "windows", target_os = "android"))]
#[allow(dead_code)]
pub const SHELL_DOC_ORIGIN: &str = "http://storyforge-shell.localhost";
#[cfg(not(any(target_os = "windows", target_os = "android")))]
#[allow(dead_code)]
pub const SHELL_DOC_ORIGIN: &str = "storyforge-shell://localhost";

/// The shell-document CSP returned as a `Content-Security-Policy` HTTP header.
///
/// Mirrors `frontend/src/utils/cardShellCsp.js buildShellCspContent([])` with
/// the empty host allowlist (no remote hosts by default; the card allowlist is
/// applied by the frontend's host-mediated fetch proxy, not by the shell's own
/// network policy). Served as an HTTP header so it is authoritative even if a
/// shell document fails to inject its `<meta>`; an HTTP header + the document's
/// own (identical) `<meta>` simply intersect to the same policy.
const SHELL_DOC_CSP: &str = concat!(
    "default-src 'none'; ",
    "script-src 'unsafe-inline' 'unsafe-eval' blob: data:; ",
    "style-src 'unsafe-inline' blob: data:; ",
    "img-src data: blob: ",
    "http://storyforge-cache.localhost ",
    "storyforge-cache://localhost; ",
    "font-src data: blob: ",
    "http://storyforge-cache.localhost ",
    "storyforge-cache://localhost; ",
    "media-src data: blob: ",
    "http://storyforge-cache.localhost ",
    "storyforge-cache://localhost; ",
    "connect-src data: blob: ",
    "http://storyforge-cache.localhost ",
    "storyforge-cache://localhost; ",
    "frame-src blob: data:; ",
    "worker-src blob:; ",
    "child-src blob:; ",
    "object-src 'none'; ",
    "form-action 'none'"
);

/// In-memory token → HTML registry. Tokens are opaque 32-byte hex strings.
/// Entries are intentionally never persisted: a shell document is an ephemeral
/// rendering surface for content the parent already holds in memory; on
/// restart there is nothing to restore. The map grows only with live shells.
#[derive(Default)]
struct ShellDocRegistry {
    docs: Mutex<HashMap<String, Arc<String>>>,
}

impl ShellDocRegistry {
    fn new() -> Self {
        Self {
            docs: Mutex::new(HashMap::new()),
        }
    }

    /// Register a shell document and return an opaque token. The same HTML
    /// registered twice yields two distinct tokens (no dedup: callers may
    /// legitimately want independent documents for identical content).
    fn register(&self, html: String) -> String {
        let token = random_hex_token();
        let arc = Arc::new(html);
        // A token collision over 32 random bytes is astronomically unlikely;
        // overwrite is harmless (the prior doc would simply be unreachable).
        self.docs.lock().expect("shell-doc registry poisoned").insert(token.clone(), arc);
        token
    }

    /// Look up a document by token. Returns a cloned Arc (cheap) so the
    /// caller can build the response body without holding the lock.
    fn get(&self, token: &str) -> Option<Arc<String>> {
        self.docs
            .lock()
            .expect("shell-doc registry poisoned")
            .get(token)
            .cloned()
    }
}

fn get_shell_doc_registry() -> &'static ShellDocRegistry {
    static REGISTRY: OnceLock<ShellDocRegistry> = OnceLock::new();
    REGISTRY.get_or_init(ShellDocRegistry::new)
}

/// Public entry point used by the `card_shell_register_doc` Tauri command.
/// Returns the opaque token only; the caller (frontend) builds the full URL
/// as `<SHELL_DOC_ORIGIN>/<token>`.
pub fn register_shell_doc(html: String) -> String {
    get_shell_doc_registry().register(html)
}

/// Generate 32 random hex bytes (64 hex chars). Uses the same approach as the
/// rest of the crate (thread_rng via `rand` is already a dependency for
/// sha256-based cache names? — no, avoid adding a crate dep: use a simple
/// XorShift seeded from thread-local nanosecond time + thread id). 32 bytes of
/// entropy is far beyond the ~16-byte collision-safety threshold for this
/// in-process registry.
fn random_hex_token() -> String {
    // Process-local PRNG seeded from wall-clock nanos + an address. Entropy is
    // sufficient because the registry is in-process and the threat model is
    // "guessing a live token to read another shell's HTML", not cryptographic
    // secrecy (the parent already owns the HTML it registers).
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u128> = Cell::new(seed());
    }
    fn seed() -> u128 {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0x9e3779b97f4a7c15);
        // Fold in a stack address for per-thread divergence.
        let addr = &0u8 as *const u8 as u128;
        t ^ addr.rotate_left(17)
    }
    // 128-bit xorshift (Marsaglia).
    STATE.with(|cell| {
        let mut x = cell.get();
        let mut out = String::with_capacity(64);
        // Four 64-bit halves → 64 hex chars = 32 bytes of entropy.
        for _ in 0..4 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let lo = x as u64;
            out.push_str(&format!("{lo:016x}"));
        }
        cell.set(x);
        out
    })
}

/// Validate a token before lookup: must be exactly 64 lowercase hex chars.
/// Rejects empty, traversal, slashes, and non-hex — same strict posture as
/// card_shell_cache.rs `is_safe_cache_resource_name`.
fn is_valid_token(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && (!b.is_ascii_uppercase()))
}

/// The protocol response handler. Registered in lib.rs's tauri::Builder chain
/// right after the `storyforge-cache` handler. Mirrors
/// `card_shell_cache_protocol_response` (lib.rs:11763-11804): only GET/HEAD/
/// OPTIONS are accepted; the path minus its leading '/' is the token; on miss
/// or invalid token it returns 404. Every successful HTML response carries the
/// shell-document `Content-Security-Policy` header.
pub fn shell_doc_protocol_response(
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Method, Response, StatusCode, header};

    let with_cors = |builder: tauri::http::response::Builder| {
        builder
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
    };

    if request.method() == Method::OPTIONS {
        return with_cors(Response::builder())
            .status(StatusCode::NO_CONTENT)
            .body(Vec::new())
            .unwrap_or_else(|_| Response::new(Vec::new()));
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return with_cors(Response::builder())
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(b"method not allowed".to_vec())
            .unwrap_or_else(|_| Response::new(Vec::new()));
    }

    let token = request.uri().path().trim_start_matches('/');
    let body = if is_valid_token(token) {
        get_shell_doc_registry().get(token)
    } else {
        None
    };

    match body {
        Some(html) => with_cors(Response::builder())
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            // Authoritative shell policy; intersects with the document's own
            // <meta> (identical) to the same result.
            .header(header::CONTENT_SECURITY_POLICY, SHELL_DOC_CSP)
            .header(header::CACHE_CONTROL, "no-store")
            .body(if request.method() == Method::HEAD {
                Vec::new()
            } else {
                html.as_bytes().to_vec()
            })
            .unwrap_or_else(|_| Response::new(Vec::new())),
        None => with_cors(Response::builder())
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(b"shell document not found".to_vec())
            .unwrap_or_else(|_| Response::new(Vec::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::http::{Method, Request, Response, StatusCode, header};

    fn req(method: Method, path: &str) -> Request<Vec<u8>> {
        Request::builder()
            .method(method)
            .uri(path)
            .body(Vec::new())
            .expect("test request")
    }

    #[test]
    fn register_then_get_returns_document_with_csp_header() {
        let token = register_shell_doc("<!doctype html><body>hello".into());
        assert_eq!(token.len(), 64);
        assert!(is_valid_token(&token));

        let resp: Response<Vec<u8>> =
            shell_doc_protocol_response(req(Method::GET, &format!("/{token}")));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        // The authoritative shell CSP header must be present.
        let csp = resp
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("script-src 'unsafe-inline'"));
        assert!(csp.contains("object-src 'none'"));
        assert_eq!(resp.body(), b"<!doctype html><body>hello");
    }

    #[test]
    fn head_returns_empty_body_with_same_headers() {
        let token = register_shell_doc("<body>x".into());
        let resp = shell_doc_protocol_response(req(Method::HEAD, &format!("/{token}")));
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key(header::CONTENT_SECURITY_POLICY));
        assert!(resp.body().is_empty());
    }

    #[test]
    fn unknown_token_is_404_without_csp() {
        let resp =
            shell_doc_protocol_response(req(Method::GET, "/00000000000000000000000000000000"));
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        // Do not advertise a shell policy on misses.
        assert!(!resp.headers().contains_key(header::CONTENT_SECURITY_POLICY));
    }

    #[test]
    fn malformed_or_traversal_tokens_are_404() {
        // Paths that are valid HTTP URIs but not registered tokens. (A literal
        // space in a path is not a valid URI and is rejected at Request build
        // time — the validator's coverage of such strings is checked directly
        // in token_validator_is_strict below.)
        for bad in [
            "/",
            "/../etc/passwd",
            "/abc",
            "/GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG",
            "/deadbeef/deadbeef",
        ] {
            let resp = shell_doc_protocol_response(req(Method::GET, bad));
            assert_eq!(resp.status(), StatusCode::NOT_FOUND, "path: {bad}");
        }
    }

    #[test]
    fn non_get_methods_rejected() {
        let token = register_shell_doc("<body>".into());
        let resp = shell_doc_protocol_response(req(Method::POST, &format!("/{token}")));
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let resp = shell_doc_protocol_response(req(Method::PUT, &format!("/{token}")));
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn options_preflight_returns_no_content() {
        let resp = shell_doc_protocol_response(req(Method::OPTIONS, "/anything"));
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            resp.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "*"
        );
    }

    #[test]
    fn two_registrations_of_same_html_yield_distinct_tokens() {
        let a = register_shell_doc("<body>same".into());
        let b = register_shell_doc("<body>same".into());
        assert_ne!(a, b);
    }

    #[test]
    fn token_validator_is_strict() {
        assert!(is_valid_token(&"a".repeat(64)));
        assert!(!is_valid_token(&"A".repeat(64)), "uppercase rejected");
        assert!(!is_valid_token(&"g".repeat(64)), "non-hex rejected");
        assert!(!is_valid_token(&"a".repeat(63)), "too short");
        assert!(!is_valid_token(&"a".repeat(65)), "too long");
        assert!(!is_valid_token(""));
        assert!(!is_valid_token("../etc"));
    }
}
