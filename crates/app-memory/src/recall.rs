/// 远记忆召回（对应设计 §7.3）
///
/// 写作开始时按用户意图检索 `ArchivedSummary`：关键词基线 + 可选向量召回合并。
use tracing::debug;

use storyforge_infra_llm::Embedder;
pub use storyforge_infra_vector::extract_query_tokens;
use storyforge_infra_vector::{MetadataFilter, VectorHit, VectorStore};

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

/// 向量命中的最低余弦相似度（设计 §7.3 默认值）。
const MIN_VECTOR_SCORE: f32 = 0.45;

/// 纯关键词远记忆召回（不依赖 Embedder），legacy 作用域（无 campaign 参数）。
///
/// 用于 ContextCompiler 最小版：写作开始时按用户意图检索 `ArchivedSummary`，
/// 注入 Director volatile tail。无命中 / 无 token 时返回空。
///
/// 本入口无作用域参数，等价于 `recall_archived_by_query_filtered(_, _, _, None, None)`：
/// 2026-10-11 审计起只返回**未带 `campaign_id` 标签**的记录（legacy 会话归档），
/// 任何 Campaign 的远记忆都不会出现（见 `accepts_scope`）。需要某个 Campaign
/// 的记录必须走 `recall_archived_by_query_filtered(..., Some(cid), _)`。
pub fn recall_archived_by_query(
    store: &dyn VectorStore,
    query: &str,
    limit: usize,
) -> Result<Vec<MemoryHit>, MemoryError> {
    recall_archived_by_query_filtered(store, query, limit, None, None)
}

/// 带作用域过滤的远记忆关键词召回（作用域语义见 `accepts_scope`）。
pub fn recall_archived_by_query_filtered(
    store: &dyn VectorStore,
    query: &str,
    limit: usize,
    campaign_id: Option<&str>,
    conversation_id: Option<&str>,
) -> Result<Vec<MemoryHit>, MemoryError> {
    let tokens = extract_query_tokens(query);
    if tokens.is_empty() || limit == 0 {
        return Ok(vec![]);
    }
    let hits = match campaign_id {
        // 2026-10-11 审计：campaign 过滤必须在截断**之前**做。旧实现先取全局前
        // `limit*8` 条再过滤，别的 Campaign 记录一多就把本局命中整批挤掉。
        Some(cid) => store.search_by_keywords_filtered(
            &tokens,
            store.count().max(limit),
            &MetadataFilter::new().for_campaign(&storyforge_domain::Id::from_str(cid)),
        )?,
        // 无 campaign：「无标签」无法用等值过滤表达，取全部命中后再过滤。
        None => store.search_by_keywords(&tokens, store.count().max(limit))?,
    };
    Ok(filter_archived_hits(
        hits,
        limit,
        campaign_id,
        conversation_id,
        None,
    ))
}

/// 判断 ArchivedSummary 命中是否属于当前写作作用域。
///
/// - 无 campaign（legacy 写作路径）：只接受**未带** `campaign_id` 标签的记录；
///   给了 `conversation_id` 时还必须带同一个 `conversation_id` 标签。2026-10-11
///   审计：旧实现在这里全部接受，非 Campaign 的角色对话会召回到所有 Campaign 和
///   其它 legacy 会话的剧情摘要；随后（W-14 收尾）完全无标签的更早数据也不再
///   接受——它无法归属到任何会话，放行就会出现在每一条 legacy 会话里。
/// - 有 campaign 过滤：必须带 `campaign_id` 标签且匹配（fail closed）；
///   `conversation_id` 不参与（一个 Campaign 一条会话，fork 另立 Campaign）
///
/// W-14：旧实现把"无标签"当作匹配，legacy 会话归档会泄漏进任意 campaign 的远记忆。
/// 生产归档唯一入口（`commands/conversations.rs` 的水位归档）对 campaign 会话始终
/// 写入 `campaign_id`，因此无标签记录只可能来自无 campaign 的 legacy 会话或更早的
/// 未标记数据；收紧后它们在 campaign 召回中不再出现，未过滤召回不受影响。
fn accepts_scope(
    hit: &VectorHit,
    campaign_id: Option<&str>,
    conversation_id: Option<&str>,
) -> bool {
    let Some(cid) = campaign_id else {
        if hit.metadata.contains_key("campaign_id") {
            return false;
        }
        return match (
            conversation_id,
            hit.metadata.get("conversation_id").and_then(|v| v.as_str()),
        ) {
            (Some(current), Some(tagged)) => current == tagged,
            (Some(_), None) => false,
            (None, _) => true,
        };
    };
    match hit.metadata.get("campaign_id").and_then(|v| v.as_str()) {
        Some(hit_cid) => hit_cid == cid,
        None => {
            tracing::debug!(
                target: "far_memory",
                "召回丢弃未标记 campaign 的归档 {}（当前过滤 campaign={}）",
                hit.id.as_str(),
                cid
            );
            false
        }
    }
}

/// 从原始 VectorHit 过滤出 ArchivedSummary，并按 campaign / min_score / limit 裁剪。
fn filter_archived_hits(
    hits: Vec<VectorHit>,
    limit: usize,
    campaign_id: Option<&str>,
    conversation_id: Option<&str>,
    min_score: Option<f32>,
) -> Vec<MemoryHit> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for hit in hits {
        if hit.kind != storyforge_infra_vector::VectorKind::ArchivedSummary {
            continue;
        }
        if !accepts_scope(&hit, campaign_id, conversation_id) {
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
    conversation_id: Option<&str>,
    embedder: Option<&Embedder>,
) -> Result<Vec<MemoryHit>, MemoryError> {
    if limit == 0 || query.trim().is_empty() {
        return Ok(vec![]);
    }

    let keyword_hits =
        recall_archived_by_query_filtered(store, query, limit, campaign_id, conversation_id)?;

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

    Ok(merge_vector_stage(
        store,
        &query_vec,
        keyword_hits,
        limit,
        campaign_id,
        conversation_id,
    ))
}

/// 混合召回的向量阶段：向量命中优先、关键词补齐。
///
/// 向量检索失败（例如库里有维度与当前 Embedder 不一致的旧记录）时回退为关键词
/// 结果——与嵌入失败同语义。此前用 `?` 直接上抛，已经算好的关键词命中会被一起
/// 丢掉，调用方只能"整次召回失败（跳过）"。空向量的关键词专用记录由向量库自身跳过。
fn merge_vector_stage(
    store: &dyn VectorStore,
    query_vec: &[f32],
    keyword_hits: Vec<MemoryHit>,
    limit: usize,
    campaign_id: Option<&str>,
    conversation_id: Option<&str>,
) -> Vec<MemoryHit> {
    // 多取再过滤 kind/min_score；campaign 过滤在向量库内、截断之前完成（同关键词路径）。
    let fetch = limit.saturating_mul(8).max(limit);
    let searched = match campaign_id {
        Some(cid) => store.search_by_vector_filtered(
            query_vec,
            fetch,
            &MetadataFilter::new().for_campaign(&storyforge_domain::Id::from_str(cid)),
        ),
        None => store.search_by_vector(query_vec, store.count().max(fetch)),
    };
    let vector_raw = match searched {
        Ok(hits) => hits,
        Err(e) => {
            tracing::warn!(target: "far_memory", "远记忆向量检索失败，回退关键词: {e}");
            return keyword_hits;
        }
    };
    let vector_hits = filter_archived_hits(
        vector_raw,
        limit,
        campaign_id,
        conversation_id,
        Some(MIN_VECTOR_SCORE),
    );

    if vector_hits.is_empty() {
        return keyword_hits;
    }

    merge_memory_hits(vector_hits, keyword_hits, limit)
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

        let hits =
            recall_archived_by_query_filtered(&store, "诊所", 5, Some("camp-a"), None).unwrap();
        // W-14：campaign 过滤下只接受带匹配标签的记录；无标签的 legacy 归档不再泄漏进来
        assert_eq!(hits.len(), 1);
        assert!(hits.iter().any(|h| h.content.contains("A 营")));
        assert!(!hits.iter().any(|h| h.content.contains("旧归档")));
        assert!(!hits.iter().any(|h| h.content.contains("B 营")));

        // 未过滤召回（legacy 写作路径）仍可见 legacy 归档，但 2026-10-11 审计起
        // 不再包含任何 Campaign 的记录（旧实现返回 3 条，A/B 两局剧情都串进来）。
        let unscoped = recall_archived_by_query_filtered(&store, "诊所", 5, None, None).unwrap();
        assert_eq!(unscoped.len(), 1);
        assert!(unscoped.iter().any(|h| h.content.contains("旧归档")));
        assert!(!unscoped.iter().any(|h| h.content.contains("营")));
    }

    #[test]
    fn campaign_recall_is_not_starved_by_other_campaigns() {
        // 2026-10-11 审计：旧实现先取全局前 `limit*8` 条再按 campaign 过滤，
        // 别的 Campaign 有足够多同关键词记录时，本局命中被整批挤出窗口。
        let store = BruteForceStore::new();
        let tagged = |cid: &str| {
            let mut meta = std::collections::HashMap::new();
            meta.insert("campaign_id".into(), serde_json::Value::String(cid.into()));
            meta
        };
        // id 以 "a-" 开头的噪声记录在同分排序（id 升序）里排在本局记录之前。
        for i in 0..40 {
            store
                .upsert(VectorRecord {
                    id: Id::from_str(format!("a-noise-{i:02}")),
                    content: format!("别的活动：诊所 {i}"),
                    vector: vec![],
                    keywords: vec!["诊所".into()],
                    kind: VectorKind::ArchivedSummary,
                    metadata: tagged("camp-noise"),
                })
                .unwrap();
        }
        store
            .upsert(VectorRecord {
                id: Id::from_str("z-mine"),
                content: "本局：诊所的地下室".into(),
                vector: vec![],
                keywords: vec!["诊所".into()],
                kind: VectorKind::ArchivedSummary,
                metadata: tagged("camp-mine"),
            })
            .unwrap();

        let hits =
            recall_archived_by_query_filtered(&store, "诊所", 3, Some("camp-mine"), None).unwrap();
        assert_eq!(
            hits.len(),
            1,
            "本局记录不得被其它 Campaign 的同关键词记录挤掉"
        );
        assert!(hits[0].content.contains("本局"));
    }

    #[test]
    fn legacy_recall_is_scoped_to_its_own_conversation() {
        // 2026-10-11 审计：legacy 归档只带 conversation_id 标签；旧实现在无
        // campaign 时全部接受，角色 A 的对话会召回角色 B 对话的剧情摘要。
        let store = BruteForceStore::new();
        let record = |id: &str, conv: Option<&str>| {
            let mut meta = std::collections::HashMap::new();
            if let Some(conv) = conv {
                meta.insert(
                    "conversation_id".into(),
                    serde_json::Value::String(conv.into()),
                );
            }
            VectorRecord {
                id: Id::from_str(id),
                content: format!("{id}：诊所"),
                vector: vec![],
                keywords: vec!["诊所".into()],
                kind: VectorKind::ArchivedSummary,
                metadata: meta,
            }
        };
        store.upsert(record("mine", Some("conv-a"))).unwrap();
        store.upsert(record("other", Some("conv-b"))).unwrap();
        store.upsert(record("untagged", None)).unwrap();

        let hits =
            recall_archived_by_query_filtered(&store, "诊所", 5, None, Some("conv-a")).unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert!(ids.contains(&"mine"));
        assert!(
            !ids.contains(&"untagged"),
            "W-14：无法归属会话的旧归档不得出现在任意 legacy 会话里"
        );
        assert!(!ids.contains(&"other"), "不得召回其它 legacy 会话的归档");
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
        let hybrid = recall_archived_hybrid(&store, "诊所", 3, None, None, None)
            .await
            .unwrap();
        let keyword = recall_archived_by_query_filtered(&store, "诊所", 3, None, None).unwrap();
        assert_eq!(hybrid.len(), keyword.len());
        assert_eq!(hybrid[0].content, keyword[0].content);
    }

    fn archived(id: &str, content: &str, vector: Vec<f32>) -> VectorRecord {
        VectorRecord {
            id: Id::from_str(id),
            content: content.into(),
            vector,
            keywords: vec!["诊所".into()],
            kind: VectorKind::ArchivedSummary,
            metadata: Default::default(),
        }
    }

    /// 2026-10-11 审计：库里有关键词专用（空向量）的世界书条目时，向量阶段仍要
    /// 产出向量命中，而不是让整次混合召回失败。
    #[test]
    fn vector_stage_tolerates_keyword_only_records() {
        let store = BruteForceStore::new();
        store
            .upsert(archived("embedded", "已嵌入的归档", vec![1.0, 0.0]))
            .unwrap();
        store
            .upsert(VectorRecord {
                kind: VectorKind::WorldInfo,
                ..archived("wi", "绿灯世界书", vec![])
            })
            .unwrap();
        store
            .upsert(archived("kw-only", "仅关键词的归档", vec![]))
            .unwrap();

        let keyword_hits =
            recall_archived_by_query_filtered(&store, "诊所", 3, None, None).unwrap();
        let merged = merge_vector_stage(&store, &[1.0, 0.0], keyword_hits, 3, None, None);
        let ids: Vec<&str> = merged.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, vec!["embedded", "kw-only"], "向量命中在前、关键词补齐");
    }

    /// 向量检索出错（维度与当前 Embedder 不一致的旧记录）时回退关键词结果，
    /// 不丢掉已算好的关键词命中。
    #[test]
    fn vector_stage_falls_back_to_keyword_hits_on_search_error() {
        let store = BruteForceStore::new();
        store
            .upsert(archived("old-dim", "旧维度归档", vec![1.0, 0.0, 0.0]))
            .unwrap();
        let keyword_hits =
            recall_archived_by_query_filtered(&store, "诊所", 3, None, None).unwrap();
        assert_eq!(keyword_hits.len(), 1);
        let merged = merge_vector_stage(&store, &[1.0, 0.0], keyword_hits, 3, None, None);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "old-dim");
    }
}
