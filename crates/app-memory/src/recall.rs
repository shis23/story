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

/// 记忆命中结果（Agent 工具 / 远记忆自动召回）
///
/// A2 补 `id`：可追溯到向量库记录，merge 去重优先用 id。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryHit {
    /// 向量库记录 id；关键词路径同样来自 VectorHit
    #[serde(default)]
    pub id: String,
    pub content: String,
    pub score: f32,
    pub kind: String,
    pub keywords: Vec<String>,
}

impl From<VectorHit> for MemoryHit {
    fn from(hit: VectorHit) -> Self {
        Self {
            id: hit.id.to_string(),
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
///
/// `campaign_id` 若提供，只返回 metadata.campaign_id 匹配或无 campaign 标签的旧记录
///（兼容归档器尚未写 campaign 标签的历史向量）。
pub fn recall_archived_by_query(
    store: &dyn VectorStore,
    query: &str,
    limit: usize,
) -> Result<Vec<MemoryHit>, MemoryError> {
    recall_archived_by_query_filtered(store, query, limit, None)
}

/// 带可选 campaign 过滤的远记忆关键词召回。
pub fn recall_archived_by_query_filtered(
    store: &dyn VectorStore,
    query: &str,
    limit: usize,
    campaign_id: Option<&str>,
) -> Result<Vec<MemoryHit>, MemoryError> {
    let tokens = extract_query_tokens(query);
    if tokens.is_empty() || limit == 0 {
        return Ok(vec![]);
    }
    // 多取一些再按 kind / campaign 过滤，避免被 WorldInfo 占满
    let fetch = limit.saturating_mul(8).max(limit);
    let hits = store.search_by_keywords(&tokens, fetch)?;
    Ok(filter_archived_hits(hits, limit, campaign_id, None))
}

/// 判断 ArchivedSummary 命中是否属于目标 campaign。
///
/// - 无过滤条件：全部接受
/// - 有 campaign 标签且不匹配：拒绝
/// - 无标签的旧归档：接受（兼容 MemoryArchiver 历史记录）
fn accepts_campaign(hit: &VectorHit, campaign_id: Option<&str>) -> bool {
    let Some(cid) = campaign_id else {
        return true;
    };
    match hit.metadata.get("campaign_id").and_then(|v| v.as_str()) {
        Some(hit_cid) => hit_cid == cid,
        None => true,
    }
}

/// 从原始 VectorHit 过滤出 ArchivedSummary，并按 campaign / min_score / limit 裁剪。
fn filter_archived_hits(
    hits: Vec<VectorHit>,
    limit: usize,
    campaign_id: Option<&str>,
    min_score: Option<f32>,
) -> Vec<MemoryHit> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for hit in hits {
        if hit.kind != storyforge_infra_vector::VectorKind::ArchivedSummary {
            continue;
        }
        if !accepts_campaign(&hit, campaign_id) {
            continue;
        }
        if let Some(min) = min_score
            && hit.score < min
        {
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
    out
}

/// 合并向量命中与关键词命中：向量优先（已按 score 排），关键词补齐未出现的 id。
pub fn merge_memory_hits(
    vector_hits: Vec<MemoryHit>,
    keyword_hits: Vec<MemoryHit>,
    limit: usize,
) -> Vec<MemoryHit> {
    if limit == 0 {
        return vec![];
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for hit in vector_hits.into_iter().chain(keyword_hits) {
        // 优先用 id 去重；无 id 时回退 content+kind
        let key = if hit.id.is_empty() {
            format!("{}|{}", hit.kind, hit.content)
        } else {
            format!("id:{}", hit.id)
        };
        if !seen.insert(key) {
            continue;
        }
        out.push(hit);
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// 混合远记忆召回：关键词基线 + 可选 Embedder 向量召回合并。
///
/// - 无 Embedder / 嵌入失败：退化为纯关键词（与 `recall_archived_by_query_filtered` 等价）
/// - 有 Embedder：向量命中（min_score 默认 0.45）优先，关键词补齐
/// - 空向量记录（仅关键词索引的 RoundSummary）会被向量路径自然跳过，由关键词兜底
pub async fn recall_archived_hybrid(
    store: &dyn VectorStore,
    query: &str,
    limit: usize,
    campaign_id: Option<&str>,
    embedder: Option<&Embedder>,
) -> Result<Vec<MemoryHit>, MemoryError> {
    if limit == 0 || query.trim().is_empty() {
        return Ok(vec![]);
    }

    let keyword_hits = recall_archived_by_query_filtered(store, query, limit, campaign_id)?;

    let Some(embedder) = embedder else {
        return Ok(keyword_hits);
    };

    let query_vec = match embedder.embed(query).await {
        Ok(v) => v,
        Err(e) => {
            debug!(target: "app-memory", "远记忆嵌入失败，回退关键词: {e}");
            return Ok(keyword_hits);
        }
    };

    // 多取再过滤 kind/campaign；空向量记录会被 cosine 跳过
    let fetch = limit.saturating_mul(8).max(limit);
    let vector_raw = store.search_by_vector(&query_vec, fetch)?;
    let vector_hits = filter_archived_hits(vector_raw, limit, campaign_id, Some(0.45));

    if vector_hits.is_empty() {
        return Ok(keyword_hits);
    }

    Ok(merge_memory_hits(vector_hits, keyword_hits, limit))
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
        assert_eq!(memory_hit.id, "test");
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

    #[test]
    fn test_recall_archived_by_query_filtered_by_campaign() {
        let store = BruteForceStore::new();
        let mut meta_a = std::collections::HashMap::new();
        meta_a.insert(
            "campaign_id".into(),
            serde_json::Value::String("camp-a".into()),
        );
        let mut meta_b = std::collections::HashMap::new();
        meta_b.insert(
            "campaign_id".into(),
            serde_json::Value::String("camp-b".into()),
        );
        store
            .upsert(VectorRecord {
                id: Id::from_str("a1"),
                content: "A 营：昨夜有人潜入诊所".into(),
                vector: vec![],
                keywords: vec!["诊所".into()],
                kind: VectorKind::ArchivedSummary,
                metadata: meta_a,
            })
            .unwrap();
        store
            .upsert(VectorRecord {
                id: Id::from_str("b1"),
                content: "B 营：诊所火灾".into(),
                vector: vec![],
                keywords: vec!["诊所".into()],
                kind: VectorKind::ArchivedSummary,
                metadata: meta_b,
            })
            .unwrap();
        store
            .upsert(VectorRecord {
                id: Id::from_str("legacy"),
                content: "旧归档：诊所值班".into(),
                vector: vec![],
                keywords: vec!["诊所".into()],
                kind: VectorKind::ArchivedSummary,
                metadata: Default::default(),
            })
            .unwrap();

        let hits = recall_archived_by_query_filtered(&store, "诊所", 5, Some("camp-a")).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.content.contains("A 营")));
        assert!(hits.iter().any(|h| h.content.contains("旧归档")));
        assert!(!hits.iter().any(|h| h.content.contains("B 营")));
    }

    #[test]
    fn test_merge_memory_hits_vector_first_then_keyword_fill() {
        let vector = vec![MemoryHit {
            id: "v1".into(),
            content: "向量命中：诊所潜入".into(),
            score: 0.9,
            kind: "ArchivedSummary".into(),
            keywords: vec![],
        }];
        let keyword = vec![
            MemoryHit {
                id: "v1".into(), // 同 id 去重
                content: "向量命中：诊所潜入".into(),
                score: 1.0,
                kind: "ArchivedSummary".into(),
                keywords: vec![],
            },
            MemoryHit {
                id: "k2".into(),
                content: "关键词补齐：诊所值班".into(),
                score: 1.0,
                kind: "ArchivedSummary".into(),
                keywords: vec![],
            },
        ];
        let merged = merge_memory_hits(vector, keyword, 3);
        assert_eq!(merged.len(), 2);
        assert!(merged[0].content.contains("向量命中"));
        assert!(merged[1].content.contains("关键词补齐"));
    }

    #[tokio::test]
    async fn test_recall_archived_hybrid_without_embedder_equals_keyword() {
        let store = BruteForceStore::new();
        store
            .upsert(VectorRecord {
                id: Id::from_str("a1"),
                content: "昨夜有人潜入诊所".into(),
                vector: vec![],
                keywords: vec!["诊所".into()],
                kind: VectorKind::ArchivedSummary,
                metadata: Default::default(),
            })
            .unwrap();
        let hybrid = recall_archived_hybrid(&store, "诊所", 3, None, None)
            .await
            .unwrap();
        let keyword = recall_archived_by_query_filtered(&store, "诊所", 3, None).unwrap();
        assert_eq!(hybrid.len(), keyword.len());
        assert_eq!(hybrid[0].content, keyword[0].content);
    }
}
