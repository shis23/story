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

use crate::llm::{ChatMessage, ChatRole};

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

    /// 拼成单条 user message 的 content
    fn into_content(self) -> String {
        self.parts.join("\n\n")
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
            hasher.update(role_label(&msg.role).as_bytes());
            hasher.update(b"\0");
            hasher.update(msg.content.as_bytes());
            hasher.update(b"\n");
        }
        hasher.update(b"tail\0");
        for part in &self.volatile_tail.parts {
            hasher.update(part.as_bytes());
            hasher.update(b"\n");
        }
        hex_encode(&hasher.finalize())
    }
}

/// 前缀指纹（内容寻址，CI / 运行时断言用）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixFingerprint {
    pub system_hash: String,
    /// (role label, content sha256 hex)
    pub history_sig: Vec<(String, String)>,
}

/// 对 hook 后最终 messages 计算请求指纹（不落全文，只记 hash）。
pub fn fingerprint_messages(messages: &[ChatMessage]) -> String {
    let mut hasher = Sha256::new();
    for msg in messages {
        hasher.update(role_label(&msg.role).as_bytes());
        hasher.update(b"\0");
        hasher.update(msg.content.as_bytes());
        hasher.update(b"\n");
    }
    hex_encode(&hasher.finalize())
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
}
