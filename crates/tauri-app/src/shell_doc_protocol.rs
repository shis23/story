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

/// M-01 mitigation (P0-2, appended to the Tauri invoke initialization script).
///
/// wry on Windows injects every initialization script into subframes
/// (`for_main_frame_only` is a no-op there — wry-0.55.1 `src/lib.rs:990`), so a
/// `storyforge-shell` subframe receives the full Tauri IPC bootstrap, whose
/// `__TAURI_INTERNALS__.postMessage` closure carries the per-session invoke
/// key, while `is_local_url` classifies the shell origin as local and the ACL
/// branch in `webview/mod.rs` is skipped for app commands (no `permissions/`
/// manifest ⇒ `has_app_acl_manifest == false`). Runtime PoC (2026-09-28,
/// `artifacts/p02-poc/`): a plain iframe on the shell origin invoked
/// `list_conversations` and `list_agent_profile_configs` successfully
/// (pre-fix). Shell documents are designed to talk to the host ONLY via
/// postMessage bridges (see the module docs), so this guard runs at the tail
/// of the invoke initialization script and, in any subframe:
///
/// 1. freezes `__TAURI_INTERNALS__` — the property itself is non-configurable
///    (`Object.defineProperty` default in tauri's first init script) and
///    cannot be deleted, but freezing the value stops every later tauri init
///    script (metadata, core.js `invoke`/`convertFileSrc`, event/plugin
///    scripts) from attaching anything. Without `convertFileSrc`, the fetch
///    branch of `sendIpcMessage` (`tauri-2.11.5/scripts/ipc-protocol.js:37`)
///    throws before a request is sent, and the `window.ipc.postMessage`
///    fallback (`:84`) is unreachable behind the closure-private
///    `customProtocolIpcFailed` flag;
/// 2. blocks the two IPC transports for page code as well — `fetch` to
///    `ipc:`/`http(s)://ipc.localhost` (all other URLs pass through, so shell
///    asset/module fetches are unaffected) and `window.ipc`. The invoke key
///    never becomes page-readable text (it is deliberately declared outside
///    the exposed functions so `toString` cannot leak it), so hand-crafted
///    raw payloads cannot pass the key check in `on_message` either.
///
/// On platforms that honor `for_main_frame_only` (macOS/Linux) this script
/// never runs in subframes; on Android the direct postMessage branch is
/// covered by the `window.ipc` removal. The main frame is untouched
/// (`window.self === window.top`).
pub const SUBFRAME_IPC_GUARD: &str = r#"
;(function () {
  if (window.self !== window.top) {
    var sf = window.__TAURI_INTERNALS__
    if (sf) {
      try { Object.freeze(sf) } catch (e) {}
    }
    try {
      var nativeFetch = window.fetch && window.fetch.bind(window)
      if (nativeFetch) {
        window.fetch = function (input, init) {
          var u = typeof input === 'string' ? input : (input && input.url) || ''
          if (/^ipc:/i.test(u) || /^https?:\/\/ipc\.localhost($|[:/])/i.test(u)) {
            return Promise.reject(new Error('storyforge: subframe IPC is disabled (P0-2)'))
          }
          return nativeFetch(input, init)
        }
      }
    } catch (e) {}
    try { delete window.ipc } catch (e) {}
    try {
      Object.defineProperty(window, 'ipc', { value: undefined, writable: false, configurable: false })
    } catch (e) {}
  }
})()
"#;

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
    // M-01（卫生项，不构成安全修复）：把自定义协议 IPC 需要的来源补进
    // connect-src，使 `invoke` 能走 tauri-2.11.5/scripts/ipc-protocol.js 的
    // 自定义协议 fetch 路径，而不是落到它文档里那条「CSP 拦截后回退到
    // window.ipc.postMessage」的路径（`ipc-protocol.js` 注释：either the
    // webview blocked a custom protocol or it was a CSP error）。两条路径最终
    // 都会到达同一个 IPC handler，所以这里消除的是对回退路径的依赖，不是
    // 子帧可达 IPC 这件事本身——真正的控制点是 ACL manifest / 帧级门禁，
    // 见 tests 中的 acl_manifest_absence_is_a_known_risk 守卫。
    // 与主窗 CSP（tauri.conf.json connect-src ipc: http://ipc.localhost）同源。
    "ipc: http://ipc.localhost ",
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

    /// M-01 守卫 1：壳文档 CSP 必须保留自定义协议 IPC 所需来源。
    /// 依据 `tauri-2.11.5/scripts/ipc-protocol.js`：自定义协议 fetch 失败时
    /// （该文件注释：要么 webview 拦了自定义协议，要么是 CSP 错误）会回退到
    /// `window.ipc.postMessage`，而回退路径不受 CSP 约束。保留这两个来源能让
    /// IPC 走显式声明的通道，而不是依赖未声明的回退。
    #[test]
    fn shell_csp_keeps_tauri_ipc_sources_for_custom_protocol_fetch() {
        let connect_src = SHELL_DOC_CSP
            .split(';')
            .find(|directive| directive.trim_start().starts_with("connect-src"))
            .expect("connect-src directive");
        assert!(
            connect_src.contains("ipc:"),
            "shell connect-src must keep `ipc:` (tauri custom-protocol IPC): {connect_src}"
        );
        assert!(
            connect_src.contains("http://ipc.localhost"),
            "shell connect-src must keep `http://ipc.localhost`: {connect_src}"
        );
    }

    /// M-01 守卫 2：壳文档仍然是零信任来源——不得出现远端主机或通配指令。
    #[test]
    fn shell_csp_stays_locked_down() {
        assert!(SHELL_DOC_CSP.contains("default-src 'none'"));
        assert!(SHELL_DOC_CSP.contains("object-src 'none'"));
        assert!(SHELL_DOC_CSP.contains("form-action 'none'"));
        assert!(
            !SHELL_DOC_CSP.contains("https://"),
            "shell CSP must not allow remote https hosts: {SHELL_DOC_CSP}"
        );
        assert!(
            !SHELL_DOC_CSP.contains('*'),
            "shell CSP must not contain wildcards: {SHELL_DOC_CSP}"
        );
    }

    /// M-01 守卫 3：capability 只能绑定主窗口 `main`。
    /// 依据 `tauri-2.11.5/src/webview/mod.rs:1787-1852`：ACL 判定用
    /// `Origin::Local` + 窗口标签，子帧与主帧在 ACL 眼里完全一样；一旦
    /// capability 放宽到 `webviews`/`remote`，任何壳 iframe 直接继承权限。
    #[test]
    fn capability_grants_only_the_main_window() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            compact.contains("\"windows\":[\"main\"]"),
            "capability must stay scoped to the `main` window only:\n{raw}"
        );
        assert!(
            !compact.contains("\"webviews\"") && !compact.contains("\"remote\""),
            "capability must not widen scope to webviews/remote:\n{raw}"
        );
    }

    /// M-01 守卫 4：app ACL manifest 的存在性必须被显式记录（防静默回退）。
    /// - 当前仓库没有 `permissions/`，因此 `has_app_acl_manifest == false`
    ///   （`tauri-2.11.5/src/ipc/authority.rs:132-134` →
    ///   `webview/mod.rs:1819-1826`），本地源的应用命令**整体跳过 ACL**。
    ///   ACL 本身也区分不了同 webview 的主帧与同源子帧（粒度是
    ///   window/webview label + Local/Remote），所以补 manifest 单独并不
    ///   解决子帧问题——真正的运行时缓解是 `SUBFRAME_IPC_GUARD`（见下）。
    /// - 一旦有人新增 `permissions/`，就必须同时把 `__app__` 编进
    ///   `gen/schemas/acl-manifests.json`；否则 manifest 没编进去，
    ///   `has_app_acl_manifest` 仍是 false，会造成"以为加了 ACL"的假安全感。
    ///   此时还必须为前端真正需要的命令逐条定义权限，否则主窗口命令会被拒
    ///   （`Command X not allowed by ACL`）。
    #[test]
    fn acl_manifest_absence_is_a_known_risk() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let permissions_dir = manifest_dir.join("permissions");
        let has_permissions_dir = permissions_dir
            .read_dir()
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false);
        if !has_permissions_dir {
            eprintln!(
                "warning: M-01 — no `permissions/` app ACL manifest, so `has_app_acl_manifest == \
                 false` and Tauri skips ACL for every app command issued from a local origin. \
                 ACL cannot distinguish a same-origin subframe from the main frame anyway, so the \
                 shipped runtime mitigation is `SUBFRAME_IPC_GUARD` (subframe IPC neutered at init \
                 time; runtime PoC evidence 2026-09-28 in artifacts/p02-poc/). Do NOT add a \
                 manifest without whitelisting every command the app UI calls."
            );
            return;
        }
        let acl = manifest_dir.join("gen/schemas/acl-manifests.json");
        let raw = std::fs::read_to_string(&acl).unwrap_or_else(|e| {
            panic!(
                "`permissions/` exists but {} is unreadable: {e}",
                acl.display()
            )
        });
        assert!(
            raw.contains("\"__app__\""),
            "`permissions/` exists but `__app__` is missing from acl-manifests.json — the \
             manifest is not compiled in, so ACL is still skipped (false confidence)"
        );
    }

    /// M-01 守卫 5：子帧 IPC 毒化脚本必须在场且只针对子帧。
    /// 静态断言（运行时证据见 artifacts/p02-poc/ 的前后对照 PoC）：
    /// ① 用 `window.self !== window.top` 限定子帧（主帧零影响）；
    /// ② 冻结 `__TAURI_INTERNALS__`（阻断后续 init 脚本装 invoke/
    ///    convertFileSrc，fetch 分支因缺 convertFileSrc 而发不出请求）；
    /// ③ 拦截 `ipc:` / `ipc.localhost` 目标的 fetch 且放行其它 URL
    ///    （壳文档的资产/模块 fetch 不受影响）；
    /// ④ 删除并封死 `window.ipc`（postMessage 传输 + Android 直连分支）。
    #[test]
    fn subframe_guard_only_acts_in_subframes_and_blocks_both_transports() {
        assert!(
            SUBFRAME_IPC_GUARD.contains("window.self !== window.top"),
            "guard must be scoped to subframes only"
        );
        assert!(
            SUBFRAME_IPC_GUARD.contains("Object.freeze(sf)"),
            "guard must freeze __TAURI_INTERNALS__ so later init scripts attach nothing"
        );
        assert!(
            SUBFRAME_IPC_GUARD.contains(r"/^ipc:/i.test(u)")
                && SUBFRAME_IPC_GUARD.contains(r"/^https?:\/\/ipc\.localhost($|[:/])/i.test(u)"),
            "guard must block fetch to the ipc custom protocol targets"
        );
        assert!(
            SUBFRAME_IPC_GUARD.contains("return nativeFetch(input, init)"),
            "guard must pass non-ipc fetches through untouched"
        );
        assert!(
            SUBFRAME_IPC_GUARD.contains("delete window.ipc")
                && SUBFRAME_IPC_GUARD.contains("defineProperty(window, 'ipc'"),
            "guard must remove and seal the wry postMessage bridge"
        );
        assert!(
            SUBFRAME_IPC_GUARD.is_ascii(),
            "guard must stay pure ASCII (PS5.1 ASCII rule extends to embedded scripts)"
        );
        assert!(
            !SUBFRAME_IPC_GUARD.contains("__TAURI_INVOKE_KEY__"),
            "guard must not reference the invoke key (nothing may make it page-readable)"
        );
    }

    /// M-01 守卫 6：毒化脚本必须真的接在 Builder 上（防止重构时被静默摘除）。
    #[test]
    fn subframe_guard_is_wired_into_the_builder() {
        let lib_rs = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("lib.rs");
        let raw = std::fs::read_to_string(&lib_rs)
            .unwrap_or_else(|e| panic!("read {}: {e}", lib_rs.display()));
        assert!(
            raw.contains(
                "append_invoke_initialization_script(shell_doc_protocol::SUBFRAME_IPC_GUARD)"
            ),
            "lib.rs must keep SUBFRAME_IPC_GUARD appended to the invoke initialization script"
        );
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
        // M-01（卫生项）：connect-src 额外保留 Tauri 自定义协议 IPC 来源
        // （`ipc:` / `http://ipc.localhost`），使 invoke 走 ipc-protocol.js 的
        // 自定义协议 fetch 而不是不受 CSP 约束的 window.ipc.postMessage 回退。
        assert!(SHELL_DOC_CSP.contains(
            "connect-src data: blob: ipc: http://ipc.localhost \
             http://storyforge-cache.localhost \
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
