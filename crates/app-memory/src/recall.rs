/// 记忆召回器（对应设计 §7.3）
///
/// Agent 工具调用入口：query → 关键词生成 → 向量召回 → rerank → 结果。
/// 借鉴 shujuku 的 keywordPromptGroup 工作流。
use std::sync::Arc;

use tracing::{debug, info};

use storyforge_infra_llm::{Embedder, LlmClient};
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
    llm: Arc<dyn LlmClient>,
    embedder: Arc<Embedder>,
    vector_store: Arc<dyn VectorStore>,
    /// 最低相似度阈值
    min_score: f32,
}

impl MemoryRecaller {
    pub fn new(
        llm: Arc<dyn LlmClient>,
        embedder: Arc<Embedder>,
        vector_store: Arc<dyn VectorStore>,
    ) -> Self {
        Self {
            llm,
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
    pub async fn recall(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<MemoryHit>, MemoryError> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
