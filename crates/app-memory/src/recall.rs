/// 记忆召回器（对应设计 §7.3）
///
/// Agent 工具调用入口：query → 关键词生成 → 向量召回 → rerank → 结果。
/// 借鉴 shujuku 的 keywordPromptGroup 工作流。
use std::sync::Arc;

use tracing::{debug, info};

use storyforge_infra_llm::Embedder;
use storyforge_infra_vector::{VectorHit, VectorStore};

use crate::archiver::MemoryError;

// ─── 记忆命中结果 ──────────────────────────────────────────────────────────

/// 记忆命中结果（Agent 工具返回用）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryHit {
    pub content: String,
    pub score: f32,
    pub kind: String,
    pub keywords: Vec<String>,
}

impl From<VectorHit> for MemoryHit {
    fn from(hit: VectorHit) -> Self {
        Self {
            content: hit.content,
            score: hit.score,
            kind: format!("{:?}", hit.kind),
            keywords: hit.keywords,
        }
    }
}

// ─── 召回器 ────────────────────────────────────────────────────────────────

/// 记忆召回器
pub struct MemoryRecaller {
    embedder: Arc<Embedder>,
    vector_store: Arc<dyn VectorStore>,
    /// 最低相似度阈值
    min_score: f32,
}

impl MemoryRecaller {
    pub fn new(embedder: Arc<Embedder>, vector_store: Arc<dyn VectorStore>) -> Self {
        Self {
            embedder,
            vector_store,
            min_score: 0.45, // 设计 §7.3 默认值
        }
    }

    /// 设置最低相似度阈值
    pub fn with_min_score(mut self, min_score: f32) -> Self {
        self.min_score = min_score;
        self
    }

    /// Agent 工具调用入口（替代 shujuku 的自动注入）
    ///
    /// 流程：
    /// 1. LLM 生成检索关键词
    /// 2. 向量相似度召回
    /// 3. minScore 过滤
    pub async fn recall(&self, query: &str, top_k: usize) -> Result<Vec<MemoryHit>, MemoryError> {
        info!(target: "app-memory", "召回请求: query={query}, top_k={top_k}");

        // Step 1: 向量相似度召回
        let query_vec = self.embedder.embed(query).await.map_err(MemoryError::Llm)?;
        let hits = self.vector_store.search_by_vector(&query_vec, top_k)?;

        // Step 2: minScore 过滤
        let filtered: Vec<MemoryHit> = hits
            .into_iter()
            .filter(|h| h.score >= self.min_score)
            .map(MemoryHit::from)
            .collect();

        debug!(target: "app-memory", "召回 {} 条结果（阈值 {}）", filtered.len(), self.min_score);
        Ok(filtered)
    }

    /// 关键词召回（不用向量，直接关键词匹配）
    pub fn recall_by_keywords(
        &self,
        keywords: &[String],
        limit: usize,
    ) -> Result<Vec<MemoryHit>, MemoryError> {
        let hits = self.vector_store.search_by_keywords(keywords, limit)?;
        Ok(hits.into_iter().map(MemoryHit::from).collect())
    }
}

/// 从自然语言查询抽取检索 token（中英混合）。
///
/// - 英文/数字：按空白与标点切词，长度 ≥ 2
/// - 中文等非 ASCII：抽 2-gram，覆盖无空格连续文本
pub fn extract_query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for w in query.split(|c: char| c.is_whitespace() || c.is_ascii_punctuation()) {
        let w = w.trim();
        if w.chars().count() >= 2 {
            tokens.push(w.to_string());
        }
    }
    let chars: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
        .collect();
    for window in chars.windows(2) {
        if window.iter().any(|c| !c.is_ascii()) {
            tokens.push(window.iter().collect());
        }
    }
    // 去重保序
    let mut seen = std::collections::HashSet::new();
    tokens.retain(|t| seen.insert(t.clone()));
    tokens
}

/// 纯关键词远记忆召回（不依赖 Embedder）。
///
/// 用于 ContextCompiler 最小版：写作开始时按用户意图检索 `ArchivedSummary`，
/// 注入 Director volatile tail。无命中 / 无 token 时返回空。
pub fn recall_archived_by_query(
    store: &dyn VectorStore,
    query: &str,
    limit: usize,
) -> Result<Vec<MemoryHit>, MemoryError> {
    let tokens = extract_query_tokens(query);
    if tokens.is_empty() || limit == 0 {
        return Ok(vec![]);
    }
    // 多取一些再按 kind 过滤，避免被 WorldInfo 占满
    let fetch = limit.saturating_mul(4).max(limit);
    let hits = store.search_by_keywords(&tokens, fetch)?;
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for hit in hits {
        if hit.kind != storyforge_infra_vector::VectorKind::ArchivedSummary {
            continue;
        }
        if !seen.insert(hit.id.as_str().to_string()) {
            continue;
        }
        out.push(MemoryHit::from(hit));
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::Id;
    use storyforge_infra_vector::{BruteForceStore, VectorKind, VectorRecord};

    #[test]
    fn test_memory_hit_from_vector_hit() {
        let hit = VectorHit {
            id: storyforge_domain::Id::from_str("test"),
            content: "测试内容".into(),
            score: 0.8,
            kind: storyforge_infra_vector::VectorKind::ArchivedSummary,
            keywords: vec!["测试".into()],
            metadata: std::collections::HashMap::new(),
        };

        let memory_hit = MemoryHit::from(hit);
        assert_eq!(memory_hit.content, "测试内容");
        assert_eq!(memory_hit.score, 0.8);
    }

    #[test]
    fn test_extract_query_tokens_chinese_bigrams() {
        let tokens = extract_query_tokens("雨夜诊所");
        assert!(tokens.iter().any(|t| t == "雨夜"));
        assert!(tokens.iter().any(|t| t == "夜诊"));
        assert!(tokens.iter().any(|t| t == "诊所"));
    }

    #[test]
    fn test_recall_archived_by_query_filters_kind() {
        let store = BruteForceStore::new();
        store
            .upsert(VectorRecord {
                id: Id::from_str("a1"),
                content: "昨夜有人潜入诊所".into(),
                vector: vec![],
                keywords: vec!["诊所".into(), "潜入".into()],
                kind: VectorKind::ArchivedSummary,
                metadata: Default::default(),
            })
            .unwrap();
        store
            .upsert(VectorRecord {
                id: Id::from_str("w1"),
                content: "诊所常驻世界设定".into(),
                vector: vec![],
                keywords: vec!["诊所".into()],
                kind: VectorKind::WorldInfo,
                metadata: Default::default(),
            })
            .unwrap();

        let hits = recall_archived_by_query(&store, "诊所发生了什么", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("潜入诊所"));
    }
}
