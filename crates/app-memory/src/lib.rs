/// 记忆系统（对应设计 §7）
///
/// 三层记忆：
/// - Layer 1: Recent Window（滑动窗口，直接在 ConversationStore 中管理）
/// - Layer 2: Archived Summary（归档总结，LLM 压缩）
/// - Layer 3: Vector Index（向量检索池，infra-vector）
///
/// 本 crate 实现 Layer 2（归档器）和 Layer 3（召回器）。
/// Layer 1 的滑动窗口在 ConversationStore 中管理，不在此 crate。
pub mod archiver;
pub mod recall;

// 重新导出核心类型
pub use archiver::{ArchiveConfig, ArchivedSummary, MemoryArchiver, extract_keywords};
pub use recall::{
    MemoryHit, MemoryRecaller, extract_query_tokens, recall_archived_by_query,
    recall_archived_by_query_filtered,
};
