//! Card frontend shell manifest — extract remote/inline shells from ST cards.
//!
//! test-card (命定之诗 v4.1) drives the golden assertions:
//! - Display regex `$('body').load(URL)` for home / custom_start / status
//! - tavern_helper remote ES modules (MagVarUpdate, data_schema, …)
//! - CDN deps inside regex HTML (jquery / lodash / fonts / js-yaml)

use serde::{Deserialize, Serialize};

use crate::character::Character;
use crate::preset::RegexScript;

/// Shell surface kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardShellKind {
    OpeningHome,
    OpeningCustom,
    StatusBar,
    MessageHtml,
    TavernHelperModule,
    Other,
}

/// Where the shell content comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardShellEntry {
    RemoteUrl { url: String },
    InlineHtml { html: String },
    InlineJs { js: String },
}

/// One extractable frontend shell unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardFrontendShell {
    pub kind: CardShellKind,
    pub entry: CardShellEntry,
    pub deps: Vec<String>,
    /// Human label (regex script name / TH script name).
    pub label: String,
    /// Trigger hint (find_regex snippet or th-script).
    pub trigger: String,
    /// Visible tavern_helper button labels (empty for regex shells).
    #[serde(default)]
    pub buttons: Vec<String>,
}

/// Full manifest for a character card.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardShellManifest {
    pub shells: Vec<CardFrontendShell>,
    /// All remote URLs discovered (shells + deps + TH imports).
    pub remote_urls: Vec<String>,
}

impl CardShellManifest {
    pub fn opening_home_url(&self) -> Option<&str> {
        self.shells.iter().find_map(|s| {
            if s.kind == CardShellKind::OpeningHome
                && let CardShellEntry::RemoteUrl { url } = &s.entry
            {
                return Some(url.as_str());
            }
            None
        })
    }

    pub fn opening_custom_url(&self) -> Option<&str> {
        self.shells.iter().find_map(|s| {
            if s.kind == CardShellKind::OpeningCustom
                && let CardShellEntry::RemoteUrl { url } = &s.entry
            {
                return Some(url.as_str());
            }
            None
        })
    }

    pub fn status_bar_url(&self) -> Option<&str> {
        self.shells.iter().find_map(|s| {
            if s.kind == CardShellKind::StatusBar
                && let CardShellEntry::RemoteUrl { url } = &s.entry
            {
                return Some(url.as_str());
            }
            None
        })
    }

    /// Ordered tavern_helper modules (preserves card script order).
    pub fn tavern_helper_modules(&self) -> Vec<&CardFrontendShell> {
        self.shells
            .iter()
            .filter(|s| s.kind == CardShellKind::TavernHelperModule)
            .collect()
    }
}

/// Extract shell manifest from a Character domain model (extensions + regex).
pub fn extract_card_shell_manifest(character: &Character) -> CardShellManifest {
    let mut shells = Vec::new();
    let mut remote_urls = Vec::new();

    // 1) Display / markdown regex replaceString → $('body').load + CDN deps
    for script in character.scoped_regex_scripts() {
        extract_from_regex_script(&script, &mut shells, &mut remote_urls);
    }

    // 2) tavern_helper.scripts content → remote ES modules / inline JS
    extract_from_tavern_helper(&character.extensions, &mut shells, &mut remote_urls);

    // 3) extensions.assets still listed as Other remote if present as URLs
    if let Some(assets) = character.extensions.get("assets") {
        collect_urls_from_value(assets, &mut remote_urls);
    }

    remote_urls.sort();
    remote_urls.dedup();

    CardShellManifest {
        shells,
        remote_urls,
    }
}

fn extract_from_regex_script(
    script: &RegexScript,
    shells: &mut Vec<CardFrontendShell>,
    remote_urls: &mut Vec<String>,
) {
    if script.disabled {
        return;
    }
    let replace = &script.replace_string;
    let find = &script.find_regex;
    let label = if script.script_name.is_empty() {
        script.id.clone()
    } else {
        script.script_name.clone()
    };

    let load_urls = capture_jquery_load_urls(replace);
    let all_urls = capture_http_urls(replace);
    for u in &all_urls {
        push_url(remote_urls, u);
    }

    if !load_urls.is_empty() {
        for url in load_urls {
            let kind = classify_shell_kind(find, &label, &url);
            let deps: Vec<String> = all_urls.iter().filter(|u| *u != &url).cloned().collect();
            shells.push(CardFrontendShell {
                kind,
                entry: CardShellEntry::RemoteUrl { url: url.clone() },
                deps,
                label: label.clone(),
                trigger: find.clone(),
                buttons: vec![],
            });
            push_url(remote_urls, &url);
        }
        return;
    }

    // Large HTML replace without .load → message HTML shell (inline)
    let looks_html = replace.contains("<html")
        || replace.contains("<!DOCTYPE")
        || replace.contains("<!doctype")
        || (replace.contains("<body") && replace.contains("<script"));
    // D-24：两个阈值单位必须各自明确，避免"字符 vs 字节"混用被误读。
    // `INLINE_HTML_MIN_CHARS` 用字符数（中文卡正文按字算更直观）；
    // `INLINE_HTML_IPC_MAX_BYTES` 用字节数（IPC 载荷上限本身是字节概念）。
    if looks_html && replace.chars().count() > INLINE_HTML_MIN_CHARS {
        let deps = all_urls.clone();
        // Huge ST display HTML (viewer shells 80KB+) must not ride the IPC manifest.
        let byte_len = replace.len();
        let html = if byte_len > INLINE_HTML_IPC_MAX_BYTES {
            // D-24：此前静默丢弃正文——manifest 里留下 `html: ""`，
            // 前端只能靠 display 文本自己认出内联文档，失败时无从排查。
            tracing::warn!(
                "内联 HTML 壳正文 {} 字节超过 IPC 上限 {}，manifest 仅保留触发信息（html 置空，label={}）",
                byte_len,
                INLINE_HTML_IPC_MAX_BYTES,
                label
            );
            String::new()
        } else {
            replace.clone()
        };
        shells.push(CardFrontendShell {
            kind: CardShellKind::MessageHtml,
            entry: CardShellEntry::InlineHtml { html },
            deps,
            label,
            trigger: find.clone(),
            buttons: vec![],
        });
    }
}

/// 内联 HTML 壳的最小正文长度（字符数，D-24）。
const INLINE_HTML_MIN_CHARS: usize = 80;

/// 内联 HTML 壳正文进入 IPC manifest 的字节上限（D-24）。
const INLINE_HTML_IPC_MAX_BYTES: usize = 8_192;

fn extract_from_tavern_helper(
    extensions: &serde_json::Value,
    shells: &mut Vec<CardFrontendShell>,
    remote_urls: &mut Vec<String>,
) {
    let Some(th) = extensions.get("tavern_helper") else {
        return;
    };
    let scripts = if let Some(arr) = th.get("scripts").and_then(|v| v.as_array()) {
        arr.clone()
    } else if let Some(arr) = th.as_array() {
        arr.clone()
    } else {
        return;
    };

    for sc in scripts {
        let enabled = sc.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
        if !enabled {
            continue;
        }
        let label = sc
            .get("name")
            .or_else(|| sc.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("tavern_helper")
            .to_string();
        let buttons = extract_visible_th_buttons(&sc);
        let content = sc
            .get("content")
            .or_else(|| sc.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if content.trim().is_empty() {
            continue;
        }

        let import_urls = capture_es_module_urls(&content);
        let http_urls = capture_http_urls(&content);
        for u in import_urls.iter().chain(http_urls.iter()) {
            push_url(remote_urls, u);
        }

        // Only ES `import 'https://...'` becomes RemoteUrl.
        // Bare CDN constants inside large IIFEs (创意工坊) must stay InlineJs.
        if let Some(entry_url) = import_urls.first().cloned() {
            let deps: Vec<String> = import_urls
                .iter()
                .chain(http_urls.iter())
                .filter(|u| *u != &entry_url)
                .cloned()
                .collect();
            shells.push(CardFrontendShell {
                kind: CardShellKind::TavernHelperModule,
                entry: CardShellEntry::RemoteUrl { url: entry_url },
                deps,
                label,
                trigger: "tavern_helper.scripts".into(),
                buttons,
            });
        } else if !content.trim().is_empty() {
            // Inline creative-workshop style script / any non-import TH payload
            shells.push(CardFrontendShell {
                kind: CardShellKind::TavernHelperModule,
                entry: CardShellEntry::InlineJs { js: content },
                deps: http_urls,
                label,
                trigger: "tavern_helper.scripts".into(),
                buttons,
            });
        }
    }
}

fn extract_visible_th_buttons(sc: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let button = sc.get("button");
    let buttons = button
        .and_then(|b| b.get("buttons"))
        .and_then(|v| v.as_array())
        .or_else(|| sc.get("buttons").and_then(|v| v.as_array()));
    let Some(arr) = buttons else {
        return out;
    };
    for b in arr {
        let visible = b.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
        if !visible {
            continue;
        }
        if let Some(name) = b.get("name").and_then(|v| v.as_str())
            && !name.trim().is_empty()
        {
            out.push(name.to_string());
        }
    }
    out
}

/// Shell surface kind.
///
/// `Other` 是**前向兼容 catch-all**：域内当前不构造它（`classify_shell_kind` 只返回
/// 上面四类），保留它是为了让未来/外部写入的 manifest 在反序列化时不至于整条失败，
/// 前端也可把它当作"未知壳"分组。D-22 复核：不删除，语义见 D-22 记录。
///
/// D-25（跨域记录）：前端 `frontend/src/utils/cardShellDisplay.js::classifyShellUrl`
/// 自带一套 URL-only 关键词分类（`/status/`、`/home/`、`custom_start`、`/intro/`），
/// 与这里的 find/label/url 三路分类**不等价**，只覆盖 URL 片段那一路：
/// - 前端不认 `find/label` 里的中文关键词（首页/状态栏/开场/自定义/自定义），
///   这类壳在 manifest 里 kind 正确、前端 URL 分类落到 `message_html`；
/// - 前端也不认 Rust 侧的 `statusplaceholder` / `status_placeholder` 片段。
///
/// 现状不影响挂载（kind 只用于展示分组/审计），但两套判定长期会漂移。
/// 建议（域1 无法单方面完成，需前端配合）：前端改用 manifest 的 `kind` 字段
/// （由 Rust 单一权威输出），仅在 manifest 缺失时回退到 URL 关键词。
fn classify_shell_kind(find: &str, label: &str, url: &str) -> CardShellKind {
    let f = find.to_lowercase();
    let l = label.to_lowercase();
    let u = url.to_lowercase();
    if f.contains("首页") || l.contains("首页") || u.contains("/home/") {
        return CardShellKind::OpeningHome;
    }
    // 状态判定先于开场：避免「开场后状态栏」类 find/label 被误分为开场壳。
    if f.contains("statusplaceholder")
        || f.contains("status_placeholder")
        || l.contains("状态栏")
        || u.contains("/status/")
    {
        return CardShellKind::StatusBar;
    }
    // 卿卿类卡：first_mes 用【开场介绍】标记触发唯一 .load 开场壳，
    // 关键词覆盖 开场/intro（find/label/URL 片段）。
    if f.contains("customized")
        || l.contains("自定义")
        || u.contains("custom_start")
        || f.contains("开场")
        || l.contains("开场")
        || l.contains("intro")
        || u.contains("/intro/")
    {
        return CardShellKind::OpeningCustom;
    }
    CardShellKind::MessageHtml
}

fn capture_jquery_load_urls(text: &str) -> Vec<String> {
    // $('body').load('URL') / $("body").load("URL")
    let mut out = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut search_from = 0;
    while search_from < lower.len() {
        let Some(rel) = lower[search_from..].find(".load(") else {
            break;
        };
        let abs = search_from + rel + ".load(".len();
        if abs > text.len() {
            break;
        }
        if !text.is_char_boundary(abs) {
            search_from = abs + 1;
            while search_from < text.len() && !text.is_char_boundary(search_from) {
                search_from += 1;
            }
            continue;
        }
        let rest = &text[abs..];
        let trimmed = rest.trim_start();
        let quote = trimmed.chars().next();
        if matches!(quote, Some('\'') | Some('"') | Some('`')) {
            let q = quote.unwrap();
            if let Some(end) = trimmed[1..].find(q) {
                let url = &trimmed[1..1 + end];
                if url.starts_with("http://") || url.starts_with("https://") {
                    out.push(url.to_string());
                }
            }
        }
        search_from = abs + 1;
        while search_from < text.len() && !text.is_char_boundary(search_from) {
            search_from += 1;
        }
    }
    out
}

fn capture_http_urls(text: &str) -> Vec<String> {
    // Advance by UTF-8 char boundaries — byte offsets panic on Chinese text.
    let mut out = Vec::new();
    let mut i = 0;
    let len = text.len();
    while i < len {
        let rest = &text[i..];
        let scheme = if rest.starts_with("https://") {
            Some(8)
        } else if rest.starts_with("http://") {
            Some(7)
        } else {
            None
        };
        if let Some(scheme_len) = scheme {
            let start = i;
            i += scheme_len;
            while i < len {
                let Some(ch) = text[i..].chars().next() else {
                    break;
                };
                if ch.is_whitespace()
                    || matches!(
                        ch,
                        '\'' | '"'
                            | '`'
                            | '<'
                            | '>'
                            | ')'
                            | '('
                            | ']'
                            | '['
                            | '}'
                            | '{'
                            | ','
                            | ';'
                    )
                {
                    break;
                }
                i += ch.len_utf8();
            }
            let mut url = text[start..i].to_string();
            while url.ends_with('.') || url.ends_with(',') || url.ends_with('。') {
                url.pop();
            }
            if (url.starts_with("http://") || url.starts_with("https://"))
                && !url.contains("www.w3.org/2000/svg")
            {
                out.push(url);
            }
        } else if let Some(ch) = text[i..].chars().next() {
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out.sort();
    out.dedup();
    out
}

fn capture_es_module_urls(text: &str) -> Vec<String> {
    // import 'url' / import "url" / from 'url' / import('url')
    let mut out = Vec::new();
    for (pat_start, _pat) in [("import ", true), ("from ", true), ("import(", true)] {
        let lower = text.to_ascii_lowercase();
        let mut search_from = 0;
        let needle = pat_start;
        while let Some(rel) = lower[search_from..].find(needle) {
            let abs = search_from + rel + needle.len();
            let rest = text[abs..].trim_start();
            let q = rest.chars().next();
            if matches!(q, Some('\'') | Some('"') | Some('`')) {
                let quote = q.unwrap();
                if let Some(end) = rest[1..].find(quote) {
                    let url = &rest[1..1 + end];
                    if url.starts_with("http://") || url.starts_with("https://") {
                        out.push(url.to_string());
                    }
                }
            }
            // Advance by UTF-8 char boundaries — a bare `abs + 1` panics when the
            // byte after the needle is inside a multi-byte char (e.g. `from 原作`).
            search_from = abs + 1;
            while search_from < text.len() && !text.is_char_boundary(search_from) {
                search_from += 1;
            }
            if search_from >= text.len() {
                break;
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn collect_urls_from_value(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::String(s) => {
            for u in capture_http_urls(s) {
                push_url(out, &u);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_urls_from_value(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_urls_from_value(item, out);
            }
        }
        _ => {}
    }
}

fn push_url(out: &mut Vec<String>, url: &str) {
    if !out.iter().any(|u| u == url) {
        out.push(url.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::{RegexPlacement, RegexScript, RegexScriptSource};
    use crate::{Id, Source};

    fn char_with_ext(extensions: serde_json::Value) -> Character {
        Character {
            id: Id::from_str("c1"),
            name: "命定".into(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions,
            renderable_assets: None,
            source: Source::ImportedFromST,
            spec_version: "2.0".into(),
            raw_card_json: serde_json::json!({}),
        }
    }

    fn regex(name: &str, find: &str, replace: &str) -> serde_json::Value {
        serde_json::json!({
            "id": name,
            "scriptName": name,
            "findRegex": find,
            "replaceString": replace,
            "placement": [2],
            "disabled": false,
            "markdownOnly": true,
        })
    }

    #[test]
    fn extracts_test_card_three_shells_and_th_modules() {
        let home = "https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/home/index.html";
        let custom = "https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/custom_start/index.html";
        let status = "https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/status/index.html";
        let mvu =
            "https://testingcf.jsdelivr.net/gh/MagicalAstrogy/MagVarUpdate/artifact/bundle.js";
        let schema = "https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/data_schema/index.js";
        let auto = "https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/Automated-script-for-destined-journey@3.2.10/dist/index.js";
        let preload = "https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/image_preload/index.js";
        let beautify = "https://testingcf.jsdelivr.net/gh/Akabanesaki/myrepo@1.1.1/AutoDialogueBeautifier/index.js";

        let extensions = serde_json::json!({
            "regex_scripts": [
                regex("状态栏", "<StatusPlaceHolderImpl/>",
                    &format!("```\n<body>\n<script>\n$('body').load('{status}')\n</script>\n</body>\n```")),
                regex("首页", "【首页】",
                    &format!("```\n<body>\n<script>\n$('body').load('{home}')\n</script>\n</body>\n```")),
                regex("自定义开局", r"<customized>\s*(.*?)\s*</customized>",
                    &format!("```\n<body>\n<script>\n$('body').load('{custom}')\n</script>\n</body>\n```")),
                regex("战斗美化", r"<action_info>",
                    r#"```<!DOCTYPE html><html><head>
                    <script src="https://cdnjs.cloudflare.com/ajax/libs/jquery/3.7.1/jquery.min.js"></script>
                    <script src="https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.21/lodash.min.js"></script>
                    </head><body></body></html>```"#),
            ],
            "tavern_helper": {
                "scripts": [
                    {"name": "【命定之诗】MVU beta", "enabled": true,
                     "content": format!("import '{mvu}'"),
                     "button": {"enabled": true, "buttons": [
                        {"name": "重新读取初始变量", "visible": true},
                        {"name": "清除旧楼层变量", "visible": false},
                        {"name": "重新处理变量", "visible": true}
                     ]}},
                    {"name": "【命定之诗】mvu zod", "enabled": true,
                     "content": format!("import '{schema}'")},
                    {"name": "【命定之诗】自动化脚本", "enabled": true,
                     "content": format!("import '{auto}'")},
                    {"name": "【命定之诗】资源预载", "enabled": true,
                     "content": format!("import '{preload}'")},
                    {"name": "【命定之诗】创意工坊v6.1", "enabled": true,
                     "content": "x".repeat(300) + " // inline workshop"},
                    {"name": "【命定之诗】自动正则", "enabled": true,
                     "content": format!("import '{beautify}'")},
                ]
            }
        });

        let character = char_with_ext(extensions);
        let manifest = extract_card_shell_manifest(&character);

        assert_eq!(manifest.opening_home_url(), Some(home));
        assert_eq!(manifest.opening_custom_url(), Some(custom));
        assert_eq!(manifest.status_bar_url(), Some(status));

        let th_remote: Vec<_> = manifest
            .shells
            .iter()
            .filter(|s| s.kind == CardShellKind::TavernHelperModule)
            .collect();
        assert!(
            th_remote.len() >= 5,
            "expected MagVarUpdate/schema/auto/preload/beautify (+ maybe inline), got {}",
            th_remote.len()
        );
        assert!(
            th_remote.iter().any(|s| matches!(
                &s.entry,
                CardShellEntry::InlineJs { js } if js.len() >= 300
            )),
            "创意工坊 should be InlineJs"
        );
        assert!(
            manifest
                .remote_urls
                .iter()
                .any(|u| u.contains("jquery/3.7.1")),
            "jquery dep should be collected"
        );
        assert!(manifest.remote_urls.iter().any(|u| u == home));
        assert!(manifest.remote_urls.iter().any(|u| u == mvu));
        let mvu_shell = manifest
            .tavern_helper_modules()
            .into_iter()
            .find(|s| s.label.contains("MVU beta"))
            .expect("MVU shell");
        assert_eq!(
            mvu_shell.buttons,
            vec!["重新读取初始变量".to_string(), "重新处理变量".to_string()]
        );
    }

    #[test]
    fn disabled_regex_skipped() {
        let mut script = RegexScript {
            id: "x".into(),
            script_name: "状态栏".into(),
            find_regex: "<StatusPlaceHolderImpl/>".into(),
            replace_string: "$('body').load('https://example.com/status/index.html')".into(),
            placement: RegexPlacement::Output,
            placement_codes: vec![2],
            source: RegexScriptSource::Scoped,
            disabled: true,
            flags: String::new(),
            only_format_formatting: None,
            markdown_only: Some(true),
            prompt_only: None,
            run_on_edit: None,
            substitute_regex: None,
            trim_strings: vec![],
            min_depth: None,
            max_depth: None,
        };
        let mut shells = vec![];
        let mut urls = vec![];
        extract_from_regex_script(&script, &mut shells, &mut urls);
        assert!(shells.is_empty());
        script.disabled = false;
        extract_from_regex_script(&script, &mut shells, &mut urls);
        assert_eq!(shells.len(), 1);
        assert_eq!(shells[0].kind, CardShellKind::StatusBar);
    }

    #[test]
    fn qingqing_opening_intro_marker_classifies_as_opening_shell() {
        // 卿卿类卡：唯一 .load 壳由 first_mes 的【开场介绍】标记触发，
        // 此前关键词表 miss → MessageHtml → 开场面从不武装（H5）。
        let opening = "https://aireckchen-dot.example.com/qingqing/opening/index.html";
        let extensions = serde_json::json!({
            "regex_scripts": [
                regex("开场介绍", "【开场介绍】",
                    &format!("```\n<body>\n<script>\n$('body').load('{opening}')\n</script>\n</body>\n```")),
            ],
        });

        let manifest = extract_card_shell_manifest(&char_with_ext(extensions));
        assert_eq!(manifest.opening_custom_url(), Some(opening));

        // 直接覆盖关键词矩阵
        assert_eq!(
            classify_shell_kind("【开场介绍】", "开场介绍", "https://x/opening.html"),
            CardShellKind::OpeningCustom
        );
        assert_eq!(
            classify_shell_kind("<intro>", "Intro Page", "https://x/page.html"),
            CardShellKind::OpeningCustom
        );
        assert_eq!(
            classify_shell_kind("<x>", "x", "https://x/intro/index.html"),
            CardShellKind::OpeningCustom
        );
        // 状态判定先于开场：带「开场后状态栏」字样的壳仍是状态壳
        assert_eq!(
            classify_shell_kind(
                "<StatusPlaceHolderImpl/>",
                "开场后状态栏",
                "https://x/s.html"
            ),
            CardShellKind::StatusBar
        );
        // 普通消息壳不受影响
        assert_eq!(
            classify_shell_kind("<action_info>", "战斗美化", "https://x/b.html"),
            CardShellKind::MessageHtml
        );
    }

    #[test]
    fn chinese_text_does_not_panic_url_scan() {
        // Regression: byte-index URL scan panicked on multi-byte UTF-8.
        let text = "<details>
<summary>变量更新中{{random::.::..::...}}</summary>
$1
</details>
<script src=\"https://cdn.jsdelivr.net/npm/js-yaml@4.1.0/dist/js-yaml.min.js\"></script>";
        let urls = capture_http_urls(text);
        assert!(urls.iter().any(|u| u.contains("js-yaml")));
        let loads = capture_jquery_load_urls(
            "变量更新中 $('body').load('https://testingcf.jsdelivr.net/x/home/index.html')",
        );
        assert_eq!(loads.len(), 1);
    }

    #[test]
    fn es_module_scan_handles_multibyte_after_needle() {
        // Regression (D-01): `from 原作` / `import 模块` put a CJK char right after
        // the ASCII needle; the old `search_from = abs + 1` sliced mid-character and
        // panicked with "start byte index N is not a char boundary".
        for text in [
            "// 移植 from 原作：设定集",
            "// 参考 import 模块实现",
            "注释 from 原作\nimport 'https://cdn.jsdelivr.net/npm/js-yaml@4.1.0/dist/x.js'",
            "注释 import 模块\nfrom 'https://example.com/a.js'",
        ] {
            let urls = capture_es_module_urls(text);
            assert!(
                urls.iter().all(|u| u.starts_with("http")),
                "unexpected url in {text:?}: {urls:?}"
            );
        }
        // Positive path still works with CJK elsewhere in the document.
        let urls = capture_es_module_urls("说明：从原作提取\nimport 'https://example.com/ok.js'");
        assert_eq!(urls, vec!["https://example.com/ok.js".to_string()]);
    }

    // ─── D-24：内联 HTML 壳的阈值语义必须明确可见 ──────────────────────────

    fn inline_html_shells(replace: String) -> Vec<CardFrontendShell> {
        let extensions = serde_json::json!({
            "regex_scripts": [
                regex("<viewer>", "<viewer>", &replace),
            ],
        });
        crate::card_shell::extract_card_shell_manifest(&char_with_ext(extensions))
            .shells
            .into_iter()
            .filter(|s| matches!(s.entry, CardShellEntry::InlineHtml { .. }))
            .collect()
    }

    #[test]
    fn inline_html_shell_keeps_body_under_ipc_limit() {
        let html = format!(
            "<!DOCTYPE html><html><body><script>var a=1;</script>{}</body></html>",
            "字".repeat(40)
        );
        assert!(html.len() <= INLINE_HTML_IPC_MAX_BYTES);
        let shells = inline_html_shells(html.clone());
        assert_eq!(shells.len(), 1);
        match &shells[0].entry {
            CardShellEntry::InlineHtml { html: kept } => assert_eq!(kept, &html),
            other => panic!("expected InlineHtml, got {other:?}"),
        }
    }

    #[test]
    fn inline_html_shell_over_ipc_limit_is_still_listed_without_body() {
        // > 8KB 的内联文档：仍必须出现在 manifest 里（前端靠 trigger 认领），
        // 但正文被丢弃——此行为保留，且从静默改为 warn 可见（D-24）
        let big = format!(
            "<!DOCTYPE html><html><body><script>var a=1;</script>{}</body></html>",
            "字".repeat(4_000)
        );
        assert!(big.len() > INLINE_HTML_IPC_MAX_BYTES);
        let shells = inline_html_shells(big);
        assert_eq!(shells.len(), 1, "超大内联壳不得被整条丢弃");
        match &shells[0].entry {
            CardShellEntry::InlineHtml { html } => {
                assert!(html.is_empty(), "超过 IPC 上限时正文置空");
            }
            other => panic!("expected InlineHtml, got {other:?}"),
        }
        assert_eq!(shells[0].kind, CardShellKind::MessageHtml);
        assert_eq!(shells[0].trigger, "<viewer>");
    }

    #[test]
    fn short_html_snippet_is_not_treated_as_shell() {
        // 阈值下限用字符数（中文 20 字 < 80）：不构成壳，避免把普通消息里的
        // 短 HTML 片段误挂 iframe（D-24 单位说明）
        let shells = inline_html_shells("<html><body>短</body></html>".to_string());
        assert!(shells.is_empty());
    }
}
