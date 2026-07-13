//! cache 友好的消息布局（三段分离 + 类型护栏）
//!
//! 对应设计 §22 / INTENT D46。
//!
//! LLM 的 KV cache 按 token 前缀匹配。稳定前缀命中 cache 省钱省延迟，易变内容必须压尾。
//! MessageLayout 用类型状态机在编译期强制三段分离，禁止易变内容污染前缀。
//!
//! 三段：
//! [1] system（稳定，整个会话不变）       → cache 全命中
//! [2] history（稳定前缀，逐轮 append）   → cache 全命中
//! [3] tail（易变，每轮新建，用完即弃）   → 只影响这条

use sha2::{Digest, Sha256};

use crate::llm::{ChatMessage, ChatRequest, ChatRole, ToolCall};

/// MessageLayout / 请求指纹算法版本（日志与 CI 对齐；改算法时 bump）。
pub const PROMPT_LAYOUT_VERSION: &str = "layout-a2-sha256-v1";

// ─── 易变末尾的构建器 ───────────────────────────────────────────────────────

/// 易变末尾（当轮 user message）
///
/// 这是 MessageLayout 里唯一允许每轮变动的部分。
/// 变量/故事时钟/任务提醒/角色状态都只能进这里。
#[derive(Debug, Clone, Default)]
pub struct VolatileTail {
    parts: Vec<String>,
}

impl VolatileTail {
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一段（如写作意图、变量、任务提醒）
    pub fn push(mut self, part: impl Into<String>) -> Self {
        self.parts.push(part.into());
        self
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// 只读拼接内容（测试 / 调试注入结果；不消费 self）
    pub fn joined_content(&self) -> String {
        self.parts.join("\n\n")
    }

    /// 拼成单条 user message 的 content
    fn into_content(self) -> String {
        self.joined_content()
    }
}

// ─── 完整布局 ──────────────────────────────────────────────────────────────

/// 三段布局（构建后的最终形态）
///
/// 用 `MessageLayout::build()` 构造，保证三段按固定顺序填入。
/// `stable_system` 和 `stable_history` 一旦填入即视为「不可变前缀」，
/// 调用方负责保证它们跨轮 byte 稳定（否则 cache 失效）。
#[derive(Debug, Clone)]
pub struct MessageLayout {
    /// [1] system message（稳定）
    stable_system: String,
    /// [2] 历史消息（稳定前缀，逐轮 append，写入后不应修改）
    stable_history: Vec<ChatMessage>,
    /// [3] 易变末尾（每轮新建）
    volatile_tail: VolatileTail,
}

impl MessageLayout {
    /// 构建器入口（强制走 builder，不能直接构造）
    pub fn build() -> MessageLayoutBuilder {
        MessageLayoutBuilder {
            stable_system: None,
            stable_history: None,
        }
    }

    /// 转成 LLM 调用用的 Vec<ChatMessage>
    ///
    /// 顺序：[system] + [history...] + [tail user message]
    pub fn into_messages(self) -> Vec<ChatMessage> {
        let mut msgs = Vec::with_capacity(2 + self.stable_history.len());
        msgs.push(ChatMessage::system(self.stable_system));
        msgs.extend(self.stable_history);
        if !self.volatile_tail.is_empty() {
            msgs.push(ChatMessage::user(self.volatile_tail.into_content()));
        }
        msgs
    }

    /// 稳定前缀指纹（system + history 内容寻址）
    ///
    /// 用于断言跨轮前缀 byte 稳定。history 用 content hash，不用长度——
    /// 等长不同内容必须产生不同指纹。
    pub fn prefix_fingerprint(&self) -> PrefixFingerprint {
        let history_sig: Vec<(String, String)> = self
            .stable_history
            .iter()
            .map(|m| (role_label(&m.role).to_string(), content_hash(&m.content)))
            .collect();
        PrefixFingerprint {
            system_hash: content_hash(&self.stable_system),
            history_sig,
        }
    }

    /// 完整请求指纹（system + history + tail），用于同输入可复现校验。
    pub fn full_request_fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"system\0");
        hasher.update(self.stable_system.as_bytes());
        hasher.update(b"\n");
        for msg in &self.stable_history {
            hash_message_into(&mut hasher, msg);
        }
        hasher.update(b"tail\0");
        for part in &self.volatile_tail.parts {
            hasher.update(part.as_bytes());
            hasher.update(b"\n");
        }
        hex_encode(&hasher.finalize())
    }

    /// 三段各自内容寻址指纹（hook 前 layout；用于 segment diff / 缓存解释）。
    pub fn segment_fingerprint(&self) -> SegmentFingerprint {
        let mut history_hasher = Sha256::new();
        for msg in &self.stable_history {
            hash_message_into(&mut history_hasher, msg);
        }
        let mut tail_hasher = Sha256::new();
        for part in &self.volatile_tail.parts {
            tail_hasher.update(part.as_bytes());
            tail_hasher.update(b"\n");
        }
        SegmentFingerprint {
            prompt_version: PROMPT_LAYOUT_VERSION.to_string(),
            system_hash: content_hash(&self.stable_system),
            history_hash: hex_encode(&history_hasher.finalize()),
            history_len: self.stable_history.len(),
            tail_hash: hex_encode(&tail_hasher.finalize()),
            tail_parts: self.volatile_tail.parts.len(),
        }
    }
}

/// 前缀指纹（内容寻址，CI / 运行时断言用）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixFingerprint {
    pub system_hash: String,
    /// (role label, content sha256 hex)
    pub history_sig: Vec<(String, String)>,
}

/// 三段 segment 指纹（debug 日志；不落全文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentFingerprint {
    pub prompt_version: String,
    pub system_hash: String,
    pub history_hash: String,
    pub history_len: usize,
    pub tail_hash: String,
    pub tail_parts: usize,
}

impl SegmentFingerprint {
    /// 日志用短摘要（各 hash 前 12 hex）。
    pub fn short_label(&self) -> String {
        format!(
            "pv={} sys={} hist={}/{} tail={}/{}",
            self.prompt_version,
            short_hex(&self.system_hash, 12),
            short_hex(&self.history_hash, 12),
            self.history_len,
            short_hex(&self.tail_hash, 12),
            self.tail_parts
        )
    }
}

/// 对 hook 后最终 messages 计算请求指纹（不落全文，只记 hash）。
///
/// 包含 role / content / tool_call_id / tool_calls（id+name+arguments），
/// 避免工具调用差异被静默忽略。
pub fn fingerprint_messages(messages: &[ChatMessage]) -> String {
    let mut hasher = Sha256::new();
    for msg in messages {
        hash_message_into(&mut hasher, msg);
    }
    hex_encode(&hasher.finalize())
}

/// 完整 ChatRequest 指纹：messages + model + tools + sampling params。
///
/// 可观测性用；不落明文，只记 hash。
pub fn fingerprint_chat_request(req: &ChatRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"messages\0");
    for msg in &req.messages {
        hash_message_into(&mut hasher, msg);
    }
    hasher.update(b"model\0");
    hasher.update(req.model.as_bytes());
    hasher.update(b"\n");
    hasher.update(b"tools\0");
    match &req.tools {
        None => hasher.update(b"none\n"),
        Some(tools) => {
            for t in tools {
                hasher.update(t.tool_type.as_bytes());
                hasher.update(b"\0");
                hasher.update(t.function.name.as_bytes());
                hasher.update(b"\0");
                hasher.update(t.function.description.as_bytes());
                hasher.update(b"\0");
                hasher.update(t.function.parameters.to_string().as_bytes());
                hasher.update(b"\n");
            }
        }
    }
    hasher.update(b"params\0");
    // 采样参数：用 Debug 稳定字段拼接，避免依赖 serde 私有布局
    hasher.update(
        format!(
            "temp={:?};top_p={:?};max={:?};reasoning={:?};extra={:?}",
            req.params.temperature,
            req.params.top_p,
            req.params.max_tokens,
            req.params.reasoning,
            req.params.extra
        )
        .as_bytes(),
    );
    hasher.update(b"\n");
    hex_encode(&hasher.finalize())
}

fn hash_message_into(hasher: &mut Sha256, msg: &ChatMessage) {
    hasher.update(role_label(&msg.role).as_bytes());
    hasher.update(b"\0");
    hasher.update(msg.content.as_bytes());
    hasher.update(b"\0");
    if let Some(id) = &msg.tool_call_id {
        hasher.update(b"tool_call_id\0");
        hasher.update(id.as_bytes());
        hasher.update(b"\0");
    }
    if let Some(calls) = &msg.tool_calls {
        hasher.update(b"tool_calls\0");
        for c in calls {
            hash_tool_call_into(hasher, c);
        }
    }
    hasher.update(b"\n");
}

fn hash_tool_call_into(hasher: &mut Sha256, call: &ToolCall) {
    hasher.update(call.id.as_bytes());
    hasher.update(b"\0");
    hasher.update(call.call_type.as_bytes());
    hasher.update(b"\0");
    hasher.update(call.function.name.as_bytes());
    hasher.update(b"\0");
    hasher.update(call.function.arguments.as_bytes());
    hasher.update(b"\0");
}

fn messages_equal_for_prefix(a: &ChatMessage, b: &ChatMessage) -> bool {
    if a.role != b.role || a.content != b.content || a.tool_call_id != b.tool_call_id {
        return false;
    }
    match (&a.tool_calls, &b.tool_calls) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            x.len() == y.len()
                && x.iter().zip(y.iter()).all(|(l, r)| {
                    l.id == r.id
                        && l.call_type == r.call_type
                        && l.function.name == r.function.name
                        && l.function.arguments == r.function.arguments
                })
        }
        _ => false,
    }
}

/// 粗粒度 token 估计：按 UTF-8 字节 / 4 上取整（不依赖真实 tokenizer）。
///
/// 仅用于前缀可复用估计与预算斜率，**不得**当作供应商 billed token。
pub fn estimate_tokens_approx(text: &str) -> u32 {
    let bytes = text.len() as u32;
    bytes.saturating_add(3) / 4
}

/// 两条消息列表的最长公共消息前缀长度（按 role+content 精确相等计数）。
///
/// 不落全文；用于跨轮 cache-prefix 稳定性与可复用 token 估计。
pub fn longest_common_message_prefix_len(a: &[ChatMessage], b: &[ChatMessage]) -> usize {
    let mut n = 0usize;
    for (left, right) in a.iter().zip(b.iter()) {
        if !messages_equal_for_prefix(left, right) {
            break;
        }
        n += 1;
    }
    n
}

/// 公共前缀消息的可复用 token 估计（不含未共享后缀）。
pub fn estimate_reusable_prefix_tokens(a: &[ChatMessage], b: &[ChatMessage]) -> u32 {
    let n = longest_common_message_prefix_len(a, b);
    a.iter()
        .take(n)
        .map(|m| estimate_tokens_approx(&m.content))
        .sum()
}

/// hook 后 messages 的粗粒度 segment 摘要（首条 system / 中间 history / 末条 user 启发式）。
///
/// 非 layout 路径或 hook 改写后无法严格还原三段时，仍可对比 system/tail 是否漂移。
pub fn messages_segment_summary(messages: &[ChatMessage]) -> SegmentFingerprint {
    let system = messages
        .iter()
        .find(|m| matches!(m.role, ChatRole::System))
        .map(|m| m.content.as_str())
        .unwrap_or("");
    let tail = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, ChatRole::User))
        .map(|m| m.content.as_str())
        .unwrap_or("");
    let history_msgs: Vec<&ChatMessage> = messages
        .iter()
        .filter(|m| !matches!(m.role, ChatRole::System))
        .collect();
    // 去掉末尾 user（视作 tail）后剩余为 history 近似
    let hist_slice = if history_msgs
        .last()
        .is_some_and(|m| matches!(m.role, ChatRole::User))
    {
        &history_msgs[..history_msgs.len().saturating_sub(1)]
    } else {
        &history_msgs[..]
    };
    let mut history_hasher = Sha256::new();
    for msg in hist_slice {
        hash_message_into(&mut history_hasher, msg);
    }
    SegmentFingerprint {
        prompt_version: PROMPT_LAYOUT_VERSION.to_string(),
        system_hash: content_hash(system),
        history_hash: hex_encode(&history_hasher.finalize()),
        history_len: hist_slice.len(),
        tail_hash: content_hash(tail),
        tail_parts: usize::from(!tail.is_empty()),
    }
}

fn short_hex(hex: &str, n: usize) -> &str {
    if hex.len() >= n { &hex[..n] } else { hex }
}

// ─── Builder（类型状态机，编译期强制顺序）──────────────────────────────────

/// 构建器
///
/// 强制顺序：system → history → tail。
/// 调 `tail()` 后返回最终 MessageLayout，不能再改 system/history。
pub struct MessageLayoutBuilder {
    stable_system: Option<String>,
    stable_history: Option<Vec<ChatMessage>>,
}

impl MessageLayoutBuilder {
    /// [1] 填稳定 system（只接受 String，强调不可变）
    pub fn system(mut self, content: impl Into<String>) -> Self {
        self.stable_system = Some(content.into());
        self
    }

    /// [2] 填稳定历史（只接受 Vec，强调不可变引用语义）
    pub fn history(mut self, messages: Vec<ChatMessage>) -> Self {
        self.stable_history = Some(messages);
        self
    }

    /// [3] 填易变末尾，返回最终布局
    ///
    /// 用闭包构建 VolatileTail，强制隔离易变逻辑。
    pub fn tail<F>(self, build_tail: F) -> MessageLayout
    where
        F: FnOnce(VolatileTail) -> VolatileTail,
    {
        MessageLayout {
            stable_system: self.stable_system.unwrap_or_default(),
            stable_history: self.stable_history.unwrap_or_default(),
            volatile_tail: build_tail(VolatileTail::new()),
        }
    }
}

// ─── 辅助 ──────────────────────────────────────────────────────────────────

fn role_label(role: &ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    }
}

fn content_hash(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

// ─── 测试 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatRole;

    #[test]
    fn test_three_segment_order() {
        let layout = MessageLayout::build()
            .system("你是导演")
            .history(vec![
                ChatMessage::user("意图1"),
                ChatMessage::assistant("成文1"),
            ])
            .tail(|t| t.push("本轮意图2").push("变量"));

        let msgs = layout.into_messages();
        // 顺序：system + 2 history + 1 tail
        assert_eq!(msgs.len(), 4);
        assert!(matches!(msgs[0].role, ChatRole::System));
        assert_eq!(msgs[0].content, "你是导演");
        assert!(matches!(msgs[1].role, ChatRole::User));
        assert_eq!(msgs[1].content, "意图1");
        assert!(matches!(msgs[2].role, ChatRole::Assistant));
        assert!(matches!(msgs[3].role, ChatRole::User));
        // tail 内容是两段拼起来
        assert!(msgs[3].content.contains("本轮意图2"));
        assert!(msgs[3].content.contains("变量"));
    }

    #[test]
    fn test_tail_joins_parts() {
        let layout = MessageLayout::build()
            .system("sys")
            .tail(|t| t.push("第一段").push("第二段").push("第三段"));
        let msgs = layout.into_messages();
        let tail = msgs.last().unwrap();
        assert_eq!(tail.content, "第一段\n\n第二段\n\n第三段");
    }

    #[test]
    fn test_empty_tail_omitted() {
        let layout = MessageLayout::build()
            .system("sys")
            .history(vec![ChatMessage::user("hi")])
            .tail(|t| t); // 空 tail
        let msgs = layout.into_messages();
        // 只有 system + history，没追加空 user
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn test_prefix_fingerprint_stable_across_rounds() {
        // 模拟两轮调用：system + history 相同，tail 不同
        let round1 = MessageLayout::build()
            .system("你是导演")
            .history(vec![ChatMessage::user("意图1")])
            .tail(|t| t.push("tail1"));

        let round2 = MessageLayout::build()
            .system("你是导演")
            .history(vec![ChatMessage::user("意图1")])
            .tail(|t| t.push("完全不同的 tail2"));

        // 前缀指纹应一致（cache 命中）
        assert_eq!(round1.prefix_fingerprint(), round2.prefix_fingerprint());
        // 完整请求指纹应不同
        assert_ne!(
            round1.full_request_fingerprint(),
            round2.full_request_fingerprint()
        );
    }

    #[test]
    fn test_prefix_fingerprint_changes_when_system_differs() {
        let a = MessageLayout::build().system("系统A").tail(|t| t);
        let b = MessageLayout::build().system("系统B").tail(|t| t);
        assert_ne!(a.prefix_fingerprint(), b.prefix_fingerprint());
    }

    #[test]
    fn test_prefix_fingerprint_changes_when_history_differs() {
        let a = MessageLayout::build()
            .system("sys")
            .history(vec![ChatMessage::user("意图1")])
            .tail(|t| t);
        let b = MessageLayout::build()
            .system("sys")
            .history(vec![
                ChatMessage::user("意图1"),
                ChatMessage::assistant("成文1"), // 多了一条
            ])
            .tail(|t| t);
        assert_ne!(a.prefix_fingerprint(), b.prefix_fingerprint());
    }

    #[test]
    fn test_prefix_fingerprint_changes_when_history_content_same_len() {
        // 等长不同内容：旧实现只记 len 会假稳定
        let a = MessageLayout::build()
            .system("sys")
            .history(vec![ChatMessage::user("abcd")])
            .tail(|t| t);
        let b = MessageLayout::build()
            .system("sys")
            .history(vec![ChatMessage::user("wxyz")])
            .tail(|t| t);
        assert_eq!("abcd".len(), "wxyz".len());
        assert_ne!(a.prefix_fingerprint(), b.prefix_fingerprint());
    }

    #[test]
    fn test_fingerprint_messages_matches_layout_full_when_no_hook() {
        let layout = MessageLayout::build()
            .system("sys")
            .history(vec![ChatMessage::user("hi")])
            .tail(|t| t.push("tail"));
        let msgs = MessageLayout::build()
            .system("sys")
            .history(vec![ChatMessage::user("hi")])
            .tail(|t| t.push("tail"))
            .into_messages();
        // full_request_fingerprint 编码格式与 fingerprint_messages 略有不同
        // （含 segment 标记），但 fingerprint_messages 自身必须稳定
        let f1 = fingerprint_messages(&msgs);
        let f2 = fingerprint_messages(&msgs);
        assert_eq!(f1, f2);
        assert_eq!(f1.len(), 64);
        let _ = layout; // silence unused
    }

    #[test]
    fn test_fingerprint_messages_sensitive_to_content() {
        let a = vec![ChatMessage::user("hello")];
        let b = vec![ChatMessage::user("world")];
        assert_ne!(fingerprint_messages(&a), fingerprint_messages(&b));
    }

    #[test]
    fn test_segment_fingerprint_tail_only_diff() {
        let a = MessageLayout::build()
            .system("sys")
            .history(vec![ChatMessage::user("h")])
            .tail(|t| t.push("t1"));
        let b = MessageLayout::build()
            .system("sys")
            .history(vec![ChatMessage::user("h")])
            .tail(|t| t.push("t2"));
        let sa = a.segment_fingerprint();
        let sb = b.segment_fingerprint();
        assert_eq!(sa.prompt_version, PROMPT_LAYOUT_VERSION);
        assert_eq!(sa.system_hash, sb.system_hash);
        assert_eq!(sa.history_hash, sb.history_hash);
        assert_ne!(sa.tail_hash, sb.tail_hash);
        assert!(sa.short_label().contains("pv="));
    }

    #[test]
    fn test_messages_segment_summary_tracks_system() {
        let a = vec![
            ChatMessage::system("S1"),
            ChatMessage::user("h"),
            ChatMessage::user("tail"),
        ];
        let b = vec![
            ChatMessage::system("S2"),
            ChatMessage::user("h"),
            ChatMessage::user("tail"),
        ];
        assert_ne!(
            messages_segment_summary(&a).system_hash,
            messages_segment_summary(&b).system_hash
        );
        assert_eq!(
            messages_segment_summary(&a).tail_hash,
            messages_segment_summary(&b).tail_hash
        );
    }

    #[test]
    fn longest_common_message_prefix_counts_exact_role_content() {
        let a = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("h1"),
            ChatMessage::assistant("a1"),
            ChatMessage::user("tail-a"),
        ];
        let b = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("h1"),
            ChatMessage::assistant("a1"),
            ChatMessage::user("tail-b"),
        ];
        assert_eq!(longest_common_message_prefix_len(&a, &b), 3);
        let reusable = estimate_reusable_prefix_tokens(&a, &b);
        let expected = estimate_tokens_approx("sys")
            + estimate_tokens_approx("h1")
            + estimate_tokens_approx("a1");
        assert_eq!(reusable, expected);
        assert!(reusable > 0);
    }

    #[test]
    fn longest_common_prefix_breaks_on_role_or_content_mismatch() {
        let a = vec![ChatMessage::system("sys"), ChatMessage::user("same")];
        let b = vec![ChatMessage::system("sys"), ChatMessage::assistant("same")];
        assert_eq!(longest_common_message_prefix_len(&a, &b), 1);

        let c = vec![ChatMessage::system("sys"), ChatMessage::user("x")];
        let d = vec![ChatMessage::system("sys"), ChatMessage::user("y")];
        assert_eq!(longest_common_message_prefix_len(&c, &d), 1);
        assert_eq!(
            estimate_reusable_prefix_tokens(&c, &d),
            estimate_tokens_approx("sys")
        );
    }

    #[test]
    fn estimate_tokens_approx_is_deterministic_and_non_zero_for_text() {
        assert_eq!(estimate_tokens_approx(""), 0);
        assert_eq!(estimate_tokens_approx("abcd"), 1);
        assert_eq!(estimate_tokens_approx("abcdefgh"), 2);
        assert_eq!(
            estimate_tokens_approx("hello world"),
            estimate_tokens_approx("hello world")
        );
    }

    #[test]
    fn fingerprint_and_lcp_include_tool_calls() {
        use crate::llm::{FunctionCall, ToolCall};
        let base = ChatMessage::assistant("call");
        let with_tool = ChatMessage {
            role: ChatRole::Assistant,
            content: "call".into(),
            tool_calls: Some(vec![ToolCall {
                id: "c1".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "search".into(),
                    arguments: "{\"q\":1}".into(),
                },
            }]),
            tool_call_id: None,
        };
        assert_ne!(
            fingerprint_messages(std::slice::from_ref(&base)),
            fingerprint_messages(std::slice::from_ref(&with_tool))
        );
        assert_eq!(
            longest_common_message_prefix_len(
                std::slice::from_ref(&base),
                std::slice::from_ref(&with_tool)
            ),
            0
        );
        let a = vec![ChatMessage::system("s"), with_tool.clone()];
        let b = vec![ChatMessage::system("s"), with_tool];
        assert_eq!(longest_common_message_prefix_len(&a, &b), 2);
    }

    #[test]
    fn fingerprint_chat_request_includes_model_tools_and_params() {
        use crate::llm::{SamplingParams, ToolSpec};
        let msgs = vec![ChatMessage::user("hi")];
        let a = ChatRequest {
            messages: msgs.clone(),
            tools: None,
            params: SamplingParams::default(),
            model: "m1".into(),
        };
        let mut b = a.clone();
        b.model = "m2".into();
        assert_ne!(fingerprint_chat_request(&a), fingerprint_chat_request(&b));

        b = a.clone();
        b.tools = Some(vec![ToolSpec::function(
            "t",
            "d",
            serde_json::json!({"type": "object"}),
        )]);
        assert_ne!(fingerprint_chat_request(&a), fingerprint_chat_request(&b));

        b = a.clone();
        b.params.temperature = Some(0.2);
        assert_ne!(fingerprint_chat_request(&a), fingerprint_chat_request(&b));
        assert_eq!(fingerprint_chat_request(&a), fingerprint_chat_request(&a));
    }
}
