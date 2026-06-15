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

use crate::llm::ChatMessage;

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

    /// 测试用：取稳定前缀的指纹（用于 CI 断言跨轮一致）
    ///
    /// 返回 (system 内容, history 消息数 + 各消息 content 长度)。
    /// 若两轮调用的指纹一致，说明前缀稳定，cache 不会失效。
    pub fn prefix_fingerprint(&self) -> PrefixFingerprint {
        let history_sig: Vec<(String, usize)> = self
            .stable_history
            .iter()
            .map(|m| (format!("{:?}", m.role), m.content.len()))
            .collect();
        PrefixFingerprint {
            system_len: self.stable_system.len(),
            system_hash: simple_hash(&self.stable_system),
            history_sig,
        }
    }
}

/// 前缀指纹（CI 测试用）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixFingerprint {
    pub system_len: usize,
    pub system_hash: u64,
    pub history_sig: Vec<(String, usize)>,
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

/// 简易字符串 hash（不引入额外 crate，测试用）
fn simple_hash(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
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
            .tail(|t| {
                t.push("第一段")
                    .push("第二段")
                    .push("第三段")
            });
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
    }

    #[test]
    fn test_prefix_fingerprint_changes_when_system_differs() {
        let a = MessageLayout::build()
            .system("系统A")
            .tail(|t| t);
        let b = MessageLayout::build()
            .system("系统B")
            .tail(|t| t);
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
}
