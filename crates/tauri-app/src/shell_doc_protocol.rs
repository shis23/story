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
    "script-src 'unsafe-inline' 'unsafe-eval' blob: data: ",
    "http://storyforge-shell.localhost ",
    "storyforge-shell://localhost; ",
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
    "storyforge-cache://localhost ",
    "http://storyforge-shell.localhost ",
    "storyforge-shell://localhost; ",
    "frame-src blob: data:; ",
    "worker-src blob:; ",
    "child-src blob:; ",
    "object-src 'none'; ",
    "form-action 'none'"
);

const MAX_SHELL_DOCS: usize = 128;
const MAX_SHELL_DOC_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellResourceKind {
    Document,
    Module,
}

#[derive(Clone)]
struct ShellResource {
    body: Arc<String>,
    kind: ShellResourceKind,
}

/// In-memory token → shell-resource registry. Tokens are opaque 32-byte hex strings.
///
/// A successful GET consumes the entry. HEAD only probes it, while the parent
/// can explicitly unregister a document that was replaced before navigation.
/// Size and entry-count limits keep a compromised renderer from turning this
/// bridge into unbounded process memory.
struct ShellDocRegistry {
    docs: Mutex<HashMap<String, ShellResource>>,
    max_docs: usize,
    max_doc_bytes: usize,
}

impl ShellDocRegistry {
    fn new() -> Self {
        Self::with_limits(MAX_SHELL_DOCS, MAX_SHELL_DOC_BYTES)
    }

    fn with_limits(max_docs: usize, max_doc_bytes: usize) -> Self {
        Self {
            docs: Mutex::new(HashMap::new()),
            max_docs,
            max_doc_bytes,
        }
    }

    /// Register a shell document and return an opaque token. The same HTML
    /// registered twice yields two distinct tokens (no dedup: callers may
    /// legitimately want independent documents for identical content).
    fn register(&self, body: String, kind: ShellResourceKind) -> Result<String, String> {
        if body.len() > self.max_doc_bytes {
            return Err(format!(
                "shell resource exceeds {} byte limit",
                self.max_doc_bytes
            ));
        }

        let mut docs = self.docs.lock().expect("shell-doc registry poisoned");
        if docs.len() >= self.max_docs {
            return Err(format!(
                "shell document registry reached {} live entries",
                self.max_docs
            ));
        }

        for _ in 0..8 {
            let token = random_hex_token();
            if !docs.contains_key(&token) {
                docs.insert(
                    token.clone(),
                    ShellResource {
                        body: Arc::new(body),
                        kind,
                    },
                );
                return Ok(token);
            }
        }
        Err("could not allocate a unique shell document token".to_string())
    }

    /// Look up a document by token. Returns a cloned Arc (cheap) so the
    /// caller can build the response body without holding the lock.
    fn get(&self, token: &str, kind: ShellResourceKind) -> Option<ShellResource> {
        self.docs
            .lock()
            .expect("shell-doc registry poisoned")
            .get(token)
            .filter(|resource| resource.kind == kind)
            .cloned()
    }

    fn take(&self, token: &str, kind: ShellResourceKind) -> Option<ShellResource> {
        let mut docs = self.docs.lock().expect("shell-doc registry poisoned");
        if docs
            .get(token)
            .is_some_and(|resource| resource.kind == kind)
        {
            docs.remove(token)
        } else {
            None
        }
    }

    fn unregister(&self, token: &str) -> bool {
        is_valid_token(token)
            && self
                .docs
                .lock()
                .expect("shell-doc registry poisoned")
                .remove(token)
                .is_some()
    }
}

fn get_shell_doc_registry() -> &'static ShellDocRegistry {
    static REGISTRY: OnceLock<ShellDocRegistry> = OnceLock::new();
    REGISTRY.get_or_init(ShellDocRegistry::new)
}

/// Public entry point used by the `card_shell_register_doc` Tauri command.
/// Returns the opaque token only; the caller (frontend) builds the full URL
/// as `<SHELL_DOC_ORIGIN>/<token>`.
pub fn register_shell_doc(html: String) -> Result<String, String> {
    get_shell_doc_registry().register(html, ShellResourceKind::Document)
}

/// Register JavaScript for one-shot module loading by an opaque-origin shell.
pub fn register_shell_module(source: String) -> Result<String, String> {
    get_shell_doc_registry().register(source, ShellResourceKind::Module)
}

pub fn unregister_shell_doc(token: &str) -> bool {
    get_shell_doc_registry().unregister(token)
}

/// Generate a 64-character token from two independently generated UUID v4
/// values. `Uuid::new_v4()` uses the operating system RNG; concatenating the
/// compact forms preserves 244 random bits without adding another RNG API.
fn random_hex_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Validate a token before lookup: must be exactly 64 lowercase hex chars.
/// Rejects empty, traversal, slashes, and non-hex — same strict posture as
/// card_shell_cache.rs `is_safe_cache_resource_name`.
fn is_valid_token(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && (!b.is_ascii_uppercase()))
}

/// The protocol response handler. Registered in lib.rs's tauri::Builder chain
/// right after the `storyforge-cache` handler. Mirrors
/// `card_shell_cache_protocol_response` (lib.rs:11763-11804): only GET/HEAD
/// are accepted; the path minus its leading '/' is the token; on miss
/// or invalid token it returns 404. Every successful HTML response carries the
/// shell-document `Content-Security-Policy` header.
pub fn shell_doc_protocol_response(
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Method, Response, StatusCode, header};

    let path = request.uri().path().trim_start_matches('/');
    let (kind, token) = match path.strip_prefix("module/") {
        Some(token) => (ShellResourceKind::Module, token),
        None => (ShellResourceKind::Document, path),
    };
    if request.method() == Method::OPTIONS
        && kind == ShellResourceKind::Module
        && is_valid_token(token)
        && get_shell_doc_registry().get(token, kind).is_some()
    {
        return Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "null")
            .header(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
            .header("Cross-Origin-Resource-Policy", "cross-origin")
            .header(header::VARY, "Origin")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Vec::new())
            .unwrap_or_else(|_| Response::new(Vec::new()));
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(b"method not allowed".to_vec())
            .unwrap_or_else(|_| Response::new(Vec::new()));
    }

    let body = if !is_valid_token(token) {
        None
    } else if request.method() == Method::HEAD || kind == ShellResourceKind::Module {
        // WebView2 can read an ES module resource more than once while
        // resolving/importing a graph. Module leases are explicitly released
        // by the parent bridge after import settles; documents stay one-shot.
        get_shell_doc_registry().get(token, kind)
    } else {
        get_shell_doc_registry().take(token, kind)
    };

    match body {
        Some(resource) => {
            let mut response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CACHE_CONTROL, "no-store");
            response = match resource.kind {
                ShellResourceKind::Document => response
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    // Authoritative shell policy; intersects with the document's own
                    // <meta> (identical) to the same result.
                    .header(header::CONTENT_SECURITY_POLICY, SHELL_DOC_CSP),
                ShellResourceKind::Module => response
                    .header(header::CONTENT_TYPE, "text/javascript; charset=utf-8")
                    // Sandboxed shells intentionally have the opaque Origin `null`.
                    // A module response must opt into CORS for import() to consume it.
                    .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "null")
                    .header(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true")
                    .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
                    .header("Cross-Origin-Resource-Policy", "cross-origin")
                    .header(header::VARY, "Origin"),
            };
            response
                .body(if request.method() == Method::HEAD {
                    Vec::new()
                } else {
                    resource.body.as_bytes().to_vec()
                })
                .unwrap_or_else(|_| Response::new(Vec::new()))
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store")
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
    fn register_then_get_returns_document_once_with_csp_header() {
        let token = register_shell_doc("<!doctype html><body>hello".into())
            .expect("register shell document");
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
        assert!(
            !resp
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            "iframe navigation does not need wildcard CORS"
        );

        let replay = shell_doc_protocol_response(req(Method::GET, &format!("/{token}")));
        assert_eq!(replay.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn module_get_returns_javascript_with_cors_until_explicit_release() {
        let token = register_shell_module("export const answer = 42;".into())
            .expect("register shell module");
        let resp: Response<Vec<u8>> =
            shell_doc_protocol_response(req(Method::GET, &format!("/module/{token}")));

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "null"
        );
        assert_eq!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
                .unwrap(),
            "true"
        );
        assert_eq!(
            resp.headers().get("Cross-Origin-Resource-Policy").unwrap(),
            "cross-origin"
        );
        assert_eq!(resp.body(), b"export const answer = 42;");

        let replay = shell_doc_protocol_response(req(Method::GET, &format!("/module/{token}")));
        assert_eq!(replay.status(), StatusCode::OK);
        assert_eq!(replay.body(), b"export const answer = 42;");

        assert!(unregister_shell_doc(&token));
        let after_release =
            shell_doc_protocol_response(req(Method::GET, &format!("/module/{token}")));
        assert_eq!(after_release.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn module_options_preflight_has_cors_without_consuming_source() {
        let token =
            register_shell_module("export default 1;".into()).expect("register shell module");
        let preflight =
            shell_doc_protocol_response(req(Method::OPTIONS, &format!("/module/{token}")));
        assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            preflight
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "null"
        );
        assert_eq!(
            preflight
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
                .unwrap(),
            "true"
        );
        assert_eq!(
            preflight
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_METHODS)
                .unwrap(),
            "GET, HEAD, OPTIONS"
        );
        assert!(preflight.body().is_empty());

        let get_after_preflight =
            shell_doc_protocol_response(req(Method::GET, &format!("/module/{token}")));
        assert_eq!(get_after_preflight.status(), StatusCode::OK);
        assert_eq!(get_after_preflight.body(), b"export default 1;");
    }

    #[test]
    fn shell_csp_allows_only_the_restricted_protocol_as_a_module_origin() {
        assert!(SHELL_DOC_CSP.contains(
            "script-src 'unsafe-inline' 'unsafe-eval' blob: data: \
             http://storyforge-shell.localhost storyforge-shell://localhost"
        ));
        assert!(SHELL_DOC_CSP.contains(
            "connect-src data: blob: http://storyforge-cache.localhost \
             storyforge-cache://localhost http://storyforge-shell.localhost \
             storyforge-shell://localhost"
        ));
        assert!(!SHELL_DOC_CSP.contains("script-src https:"));
        assert!(!SHELL_DOC_CSP.contains("connect-src https:"));
    }

    #[test]
    fn head_returns_empty_body_with_same_headers() {
        let token = register_shell_doc("<body>x".into()).expect("register shell document");
        let resp = shell_doc_protocol_response(req(Method::HEAD, &format!("/{token}")));
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key(header::CONTENT_SECURITY_POLICY));
        assert!(resp.body().is_empty());

        let get_after_head = shell_doc_protocol_response(req(Method::GET, &format!("/{token}")));
        assert_eq!(get_after_head.status(), StatusCode::OK);
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
        let token = register_shell_doc("<body>".into()).expect("register shell document");
        let resp = shell_doc_protocol_response(req(Method::POST, &format!("/{token}")));
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let resp = shell_doc_protocol_response(req(Method::PUT, &format!("/{token}")));
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn options_is_rejected_and_never_opens_wildcard_cors() {
        let resp = shell_doc_protocol_response(req(Method::OPTIONS, "/anything"));
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert!(
            !resp
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        );
    }

    #[test]
    fn two_registrations_of_same_html_yield_distinct_tokens() {
        let a = register_shell_doc("<body>same".into()).expect("first registration");
        let b = register_shell_doc("<body>same".into()).expect("second registration");
        assert_ne!(a, b);
    }

    #[test]
    fn registry_rejects_oversized_documents_and_capacity_exhaustion() {
        let registry = ShellDocRegistry::with_limits(1, 4);
        assert!(
            registry
                .register("12345".into(), ShellResourceKind::Document)
                .is_err()
        );

        let token = registry
            .register("1234".into(), ShellResourceKind::Document)
            .expect("within limit");
        assert!(
            registry
                .register("next".into(), ShellResourceKind::Document)
                .is_err()
        );
        assert!(registry.unregister(&token));
        assert!(
            registry
                .register("next".into(), ShellResourceKind::Document)
                .is_ok()
        );
    }

    #[test]
    fn unregister_rejects_replay_and_malformed_tokens() {
        let registry = ShellDocRegistry::with_limits(2, 64);
        let token = registry
            .register("<body>x".into(), ShellResourceKind::Document)
            .expect("register");
        assert!(registry.unregister(&token));
        assert!(!registry.unregister(&token));
        assert!(!registry.unregister("../not-a-token"));
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
