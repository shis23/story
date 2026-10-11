//! 派生向量索引的维护（2026-10-11 审计新增）。
//!
//! 向量库只是派生索引，真相在角色库与 RoundSummary 存储里。本模块负责让索引
//! 跟上真相，不再依赖各命令各自零散地 upsert / delete：
//! - 角色库世界书（`VectorKind::WorldInfo`）：每次角色库世界书变化后按角色库**全量重建**，
//!   被删的卡、被删/改路由的条目不会再被 `search_vectors` 搜到；
//! - Campaign 远记忆（A 级 RoundSummary → `ArchivedSummary`）：分支 / 导入后补索引
//!   （分支复用来源 Campaign 已有的向量，不重新调用嵌入 API），启动时补齐缺口；
//! - 删档后清理该 Campaign 的记录；
//! - 消息归档（会话级远记忆）：分支时复制给分支会话、导出 / 导入时随故事快照搬运，
//!   并把归档水位一起带过去；
//! - 嵌入模型是可选项：没有向量的远记忆记录照样按关键词召回，配置嵌入模型后可以
//!   为它们补建向量（`embed_archive_records`）。
//!
//! 所有入口都是 best-effort：索引失败只告警，不让主操作失败。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_infra_vector::{MetadataFilter, VectorKind, VectorRecord, VectorStore};

use crate::AppState;
use crate::commands::bundle_runtime::BundleMessageArchive;
use crate::storage::StoredCharacter;

/// 角色库世界书向量记录的来源标签（metadata key → `StoredCharacter.id`）。
const WORLD_INFO_OWNER_KEY: &str = "world_info_character";

/// 串行化世界书重建：「读角色库 → 整类替换」必须成对执行。每次角色库变更之后
/// 都会再触发一次重建，所以串行化后最后一次重建读到的一定是最新角色库。
static WORLD_INFO_SYNC: Mutex<()> = Mutex::new(());

/// 删档后清理该 Campaign 在全局向量库里的远记忆记录（`campaign_id` 标签）。
///
/// 此前没有任何生产路径调用 `delete_by_campaign`，删档后其 RoundSummary/归档向量
/// 永久滞留，持续占用 H-7 配额直到 upsert 被拒、所有 Campaign 的远记忆索引一起停摆。
/// Campaign 本体已删，清理失败只告警、不回报删档失败。
pub(crate) fn purge_campaign_vectors(state: &AppState, campaign_id: &Id) {
    match state.vector_store.delete_by_campaign(campaign_id) {
        Ok(0) => {}
        Ok(removed) => tracing::debug!(
            target: "far_memory",
            "已清理 campaign={} 的 {removed} 条向量记录",
            campaign_id.as_str()
        ),
        Err(e) => tracing::warn!(
            target: "far_memory",
            "删档后清理 campaign={} 的向量记录失败（可在下次删档/重建时再清）: {e}",
            campaign_id.as_str()
        ),
    }
}

// ─── 角色库世界书 ──────────────────────────────────────────────────────────

/// 按角色库全量重建 `WorldInfo` 向量记录（一次落盘）。
///
/// 取代旧的「导入时按关键词删旧条目 + 逐条 upsert」：旧做法删卡、删条目、改路由
/// 都不碰向量库，已删除的世界书仍会被 `search_vectors` 搜到；按关键词去重还会误删
/// 其它卡共享关键词的条目。角色库读取失败时保持现状（不把索引清空）。
pub(crate) fn sync_world_info_vectors(state: &AppState) {
    let _serial = WORLD_INFO_SYNC.lock().unwrap_or_else(|p| p.into_inner());
    let characters = match state.storage().list_characters() {
        Ok(characters) => characters,
        Err(e) => {
            tracing::warn!("世界书向量重建跳过：角色库读取失败: {e}");
            return;
        }
    };
    let records = world_info_records(&characters);
    let count = records.len();
    match state
        .vector_store
        .replace_kind(VectorKind::WorldInfo, records)
    {
        Ok(removed) => {
            tracing::debug!("世界书向量已按角色库重建：{count} 条（替换旧记录 {removed} 条）")
        }
        Err(e) => tracing::warn!("世界书向量重建失败（search_vectors 可能检索到旧条目）: {e}"),
    }
}

/// 角色库 → 绿灯世界书向量记录。
///
/// 进入检索池的条件与域模型一致：路由为 Selective / Both（未知路由按 Selective，
/// 同 tool_ctx 重建的兜底），且至少有一个非空关键词。记录 id 由角色 id + 条目序号
/// 决定，重复重建结果稳定。
fn world_info_records(characters: &[StoredCharacter]) -> Vec<VectorRecord> {
    use storyforge_domain::world_info::LoreRoute;

    let mut records = Vec::new();
    for stored in characters {
        for (index, entry) in stored.info.world_info_entries.iter().enumerate() {
            let route = crate::commands::world_info::parse_lore_route(entry.route.as_str())
                .unwrap_or(LoreRoute::Selective);
            if !matches!(route, LoreRoute::Selective | LoreRoute::Both) {
                continue;
            }
            let keywords: Vec<String> = entry
                .keys
                .iter()
                .map(|k| k.trim())
                .filter(|k| !k.is_empty())
                .map(String::from)
                .collect();
            if keywords.is_empty() {
                continue;
            }
            let mut metadata = HashMap::new();
            metadata.insert(
                WORLD_INFO_OWNER_KEY.to_string(),
                serde_json::Value::String(stored.id.clone()),
            );
            records.push(VectorRecord {
                id: Id::from_str(format!("wi:{}:{index}", stored.id)),
                content: entry.content.clone(),
                vector: vec![], // 关键词路径不需要真实向量
                keywords,
                kind: VectorKind::WorldInfo,
                metadata,
            });
        }
    }
    records
}

// ─── Campaign 远记忆 ──────────────────────────────────────────────────────

/// 参与远记忆索引的摘要：A 级（与 accept 路径一致）且正文非空。
fn indexable_summaries(summaries: Vec<RoundSummary>) -> Vec<RoundSummary> {
    summaries
        .into_iter()
        .filter(|s| s.level == 0 && !s.content.trim().is_empty())
        .collect()
}

/// 为某 Campaign 的 RoundSummary 建远记忆索引（分支 / 导入完成后调用）。
///
/// 分支与导入都会给摘要换新 id、改 `campaign_id`，旧实现从不为新 id 建索引——
/// 新局在产出新摘要之前远记忆召回为空，导入提示里的「向量索引将重建」也没有兑现。
///
/// `reuse_from` 为分支来源 Campaign：正文相同的摘要直接复用来源记录的向量，
/// 不重新调用嵌入 API。其余摘要先写关键词索引；配置了 Embedder 时在后台补嵌入
/// （与 accept 后的索引路径同一策略）。返回同步写入的记录数。
pub(crate) fn index_campaign_summaries(
    state: &Arc<AppState>,
    campaign_id: &Id,
    reuse_from: Option<&Id>,
) -> usize {
    let summaries = match state.storage().list_summaries(campaign_id) {
        Ok(summaries) => indexable_summaries(summaries),
        Err(e) => {
            tracing::warn!(
                target: "far_memory",
                "远记忆索引跳过：读取 campaign={} 的摘要失败: {e}",
                campaign_id.as_str()
            );
            return 0;
        }
    };
    if summaries.is_empty() {
        return 0;
    }

    let reusable: HashMap<String, Vec<f32>> = reuse_from
        .map(|source| {
            state
                .vector_store
                .records_matching(&MetadataFilter::new().for_campaign(source))
                .into_iter()
                .filter(|r| r.kind == VectorKind::ArchivedSummary && !r.vector.is_empty())
                .map(|r| (r.content.trim().to_string(), r.vector))
                .collect()
        })
        .unwrap_or_default();
    let vectors: Vec<(Id, Vec<f32>)> = summaries
        .iter()
        .filter_map(|s| {
            reusable
                .get(s.content.trim())
                .map(|v| (s.id.clone(), v.clone()))
        })
        .collect();

    let indexed = crate::runtime_support::index_summaries_with_vectors(
        state.vector_store.as_ref(),
        &summaries,
        &vectors,
    );
    tracing::info!(
        target: "far_memory",
        "campaign={} 远记忆索引 {indexed} 条（复用来源向量 {} 条）",
        campaign_id.as_str(),
        vectors.len()
    );

    let needs_embedding: Vec<RoundSummary> = summaries
        .into_iter()
        .filter(|s| !vectors.iter().any(|(id, _)| id == &s.id))
        .collect();
    let has_embedder = state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .is_some();
    if has_embedder && !needs_embedding.is_empty() {
        let state = state.clone();
        tauri::async_runtime::spawn(async move {
            crate::runtime_support::index_summaries_async(state, needs_embedding).await;
        });
    }
    indexed
}

/// 启动对账：按角色库重建世界书向量、清理已删 Campaign 的残留记录、补齐所有
/// Campaign 缺失的远记忆索引。
///
/// 只写关键词索引，**不在启动时调用嵌入 API**（避免一次启动产生大量计费请求）；
/// 已有记录（含已嵌入的向量）原样保留。幂等，可重复调用。
pub(crate) fn reconcile_on_startup(state: &AppState) {
    sync_world_info_vectors(state);
    let purged = purge_orphan_campaign_vectors(state);
    if purged > 0 {
        tracing::info!(target: "far_memory", "启动对账清理已删 Campaign 的向量记录 {purged} 个局");
    }
    let backfilled = backfill_missing_summary_vectors(state);
    if backfilled > 0 {
        tracing::info!(target: "far_memory", "启动对账补齐远记忆索引 {backfilled} 条");
    }
}

/// 清理 `campaign_id` 标签指向已不存在 Campaign 的记录。
///
/// 删档清理接线之前删掉的局，其远记忆 / 角色知识向量一直滞留并占用 H-7 配额。
/// 只在存储**确认**该 Campaign 不存在（`Ok(None)`）时才清；读取出错一律保留。
fn purge_orphan_campaign_vectors(state: &AppState) -> usize {
    let tagged: std::collections::BTreeSet<String> = state
        .vector_store
        .records_matching(&MetadataFilter::new())
        .into_iter()
        .filter_map(|r| {
            r.metadata
                .get("campaign_id")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();
    let mut purged = 0;
    for raw in tagged {
        let campaign_id = Id::from_str(&raw);
        if matches!(state.storage().get_campaign(&campaign_id), Ok(None)) {
            purge_campaign_vectors(state, &campaign_id);
            purged += 1;
        }
    }
    purged
}

fn backfill_missing_summary_vectors(state: &AppState) -> usize {
    let campaigns = match state.storage().list_campaigns(None) {
        Ok(campaigns) => campaigns,
        Err(e) => {
            tracing::warn!(target: "far_memory", "远记忆对账跳过：列出 Campaign 失败: {e}");
            return 0;
        }
    };
    let mut total = 0;
    for record in campaigns {
        let campaign_id = record.campaign.id;
        let missing: Vec<RoundSummary> = match state.storage().list_summaries(&campaign_id) {
            Ok(summaries) => indexable_summaries(summaries)
                .into_iter()
                .filter(|s| !state.vector_store.contains(&s.id))
                .collect(),
            Err(e) => {
                tracing::warn!(
                    target: "far_memory",
                    "远记忆对账跳过 campaign={}：读取摘要失败: {e}",
                    campaign_id.as_str()
                );
                continue;
            }
        };
        // 按 Campaign 分批：某一局超配额失败不影响其它局。
        total += crate::runtime_support::index_summaries_with_vectors(
            state.vector_store.as_ref(),
            &missing,
            &[],
        );
    }
    total
}

// ─── 消息归档（会话级远记忆）───────────────────────────────────────────────

/// 消息归档记录的 `source` 标签（`app-memory` 归档器写入）。
const MESSAGE_ARCHIVE_SOURCE: &str = "message_archive";

/// 某个会话的消息归档记录（按覆盖的消息区间排序，旧记录无区间排在最后）。
fn message_archive_records(state: &AppState, conversation_id: &Id) -> Vec<VectorRecord> {
    let mut records: Vec<VectorRecord> = state
        .vector_store
        .records_matching(&MetadataFilter::new().match_eq(
            "conversation_id",
            serde_json::Value::String(conversation_id.to_string()),
        ))
        .into_iter()
        .filter(|r| {
            r.kind == VectorKind::ArchivedSummary
                && r.metadata.get("source").and_then(|v| v.as_str()) == Some(MESSAGE_ARCHIVE_SOURCE)
        })
        .collect();
    records.sort_by_key(|r| {
        (
            message_range(r).map_or(usize::MAX, |(start, _)| start),
            r.id.to_string(),
        )
    });
    records
}

fn message_range(record: &VectorRecord) -> Option<(usize, usize)> {
    let read = |key: &str| {
        record
            .metadata
            .get(key)
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
    };
    Some((read("message_start")?, read("message_end")?))
}

/// 搬运归档后，新会话的归档水位。
///
/// 不超过来源水位；所有记录都带消息区间时，再收紧到「从 0 起连续覆盖」的末端——
/// 保证水位之前的每条消息确实有归档，缺的那段由自动归档重新补上，不会留下永久空洞。
/// 有旧记录（无区间）时无法核对，沿用来源水位。没有任何归档时为 0。
pub(crate) fn carried_archive_watermark(
    ranges: &[Option<(usize, usize)>],
    source_upto: usize,
) -> usize {
    if ranges.is_empty() {
        return 0;
    }
    if ranges.iter().any(Option::is_none) {
        return source_upto;
    }
    let mut spans: Vec<(usize, usize)> = ranges.iter().flatten().copied().collect();
    spans.sort_unstable();
    let mut covered = 0usize;
    for (start, end) in spans {
        if start > covered {
            break;
        }
        covered = covered.max(end.saturating_add(1));
    }
    covered.min(source_upto)
}

fn retag_archive(mut record: VectorRecord, campaign_id: &Id, conversation_id: &Id) -> VectorRecord {
    record.id = Id::new();
    record.metadata.insert(
        "campaign_id".into(),
        serde_json::Value::String(campaign_id.to_string()),
    );
    record.metadata.insert(
        "conversation_id".into(),
        serde_json::Value::String(conversation_id.to_string()),
    );
    record
}

/// 写入新会话的归档记录并推进它的归档水位；写入失败时水位保持 0（自动归档会重建）。
fn land_archives(
    state: &AppState,
    records: Vec<VectorRecord>,
    source_upto: usize,
    conversation_id: &Id,
) -> usize {
    if records.is_empty() {
        return 0;
    }
    let ranges: Vec<Option<(usize, usize)>> = records.iter().map(message_range).collect();
    let count = records.len();
    if let Err(e) = state.vector_store.upsert_batch(records) {
        tracing::warn!(
            target: "far_memory",
            "会话 {} 的消息归档写入失败（自动归档会从头重建）: {e}",
            conversation_id.as_str()
        );
        return 0;
    }
    let watermark = carried_archive_watermark(&ranges, source_upto);
    if watermark > 0
        && let Err(e) = state
            .conv_store
            .advance_archived_upto(conversation_id, watermark)
    {
        tracing::warn!(
            target: "far_memory",
            "会话 {} 推进归档水位失败（自动归档会重复归档这一段）: {e}",
            conversation_id.as_str()
        );
    }
    count
}

/// 分支：把来源会话的消息归档复制给分支会话（新 id、新标签，原样复用向量），
/// 并把分支会话的归档水位对齐到来源。
///
/// 分支只允许从已提交的末尾正文创建，分支会话的消息与来源逐条相同，所以来源的
/// 归档区间对分支同样成立。此前分支会话的水位被置 0 且没有任何归档——在下一次
/// 自动归档之前，分支没有会话级远记忆，自动归档还要把同一段旧剧情再花钱总结一遍。
pub(crate) fn copy_message_archives(
    state: &AppState,
    source_conversation_id: &Id,
    target_campaign_id: &Id,
    target_conversation_id: &Id,
) -> usize {
    let source_upto = state.conv_store.archived_upto(source_conversation_id);
    let records: Vec<VectorRecord> = message_archive_records(state, source_conversation_id)
        .into_iter()
        .map(|r| retag_archive(r, target_campaign_id, target_conversation_id))
        .collect();
    let copied = land_archives(state, records, source_upto, target_conversation_id);
    if copied > 0 {
        tracing::info!(
            target: "far_memory",
            "分支复制消息归档 {copied} 条：{} → {}",
            source_conversation_id.as_str(),
            target_conversation_id.as_str()
        );
    }
    copied
}

/// 导出：某会话的消息归档（不含向量）。
pub(crate) fn export_message_archives(
    state: &AppState,
    conversation_id: &Id,
) -> Vec<BundleMessageArchive> {
    message_archive_records(state, conversation_id)
        .into_iter()
        .map(|r| {
            let range = message_range(&r);
            BundleMessageArchive {
                content: r.content,
                keywords: r.keywords,
                message_start: range.map(|(start, _)| start),
                message_end: range.map(|(_, end)| end),
            }
        })
        .collect()
}

/// 导入：把包内的消息归档写成新会话的关键词记录，恢复归档水位；配置了嵌入模型时
/// 在后台补建向量。返回写入条数。
pub(crate) fn import_message_archives(
    state: &Arc<AppState>,
    archives: Vec<BundleMessageArchive>,
    source_upto: usize,
    campaign_id: &Id,
    conversation_id: &Id,
) -> usize {
    let records: Vec<VectorRecord> = archives
        .into_iter()
        .filter(|a| !a.content.trim().is_empty())
        .map(|archive| {
            let mut keywords = archive.keywords;
            if keywords.is_empty() {
                keywords = storyforge_app_memory::extract_keywords(&archive.content);
            }
            let mut metadata = HashMap::new();
            metadata.insert(
                "source".into(),
                serde_json::Value::String(MESSAGE_ARCHIVE_SOURCE.into()),
            );
            if let (Some(start), Some(end)) = (archive.message_start, archive.message_end) {
                metadata.insert("message_start".into(), start.into());
                metadata.insert("message_end".into(), end.into());
            }
            retag_archive(
                VectorRecord {
                    id: Id::new(),
                    content: archive.content,
                    vector: vec![],
                    keywords,
                    kind: VectorKind::ArchivedSummary,
                    metadata,
                },
                campaign_id,
                conversation_id,
            )
        })
        .collect();
    let ids: Vec<Id> = records.iter().map(|r| r.id.clone()).collect();
    let imported = land_archives(state, records, source_upto, conversation_id);
    if imported > 0 {
        spawn_embedding_backfill(state, Some(ids));
    }
    imported
}

// ─── 远记忆向量补建（嵌入模型可选）──────────────────────────────────────────

/// 一次补建的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct MemoryEmbeddingReport {
    /// 需要补建的记录数（无向量，或向量维度与当前嵌入模型不一致）
    pub candidates: usize,
    pub embedded: usize,
    pub failed: usize,
    /// 第一次失败的原因（嵌入接口不可用时整批停下，不再继续请求）
    pub error: Option<String>,
}

/// 同一时刻只跑一个补建任务（全局向量库只有一份）。
static EMBEDDING_BACKFILL_RUNNING: AtomicBool = AtomicBool::new(false);

/// 持有期间占住补建槽位；drop 时释放。
pub(crate) struct EmbeddingBackfillGuard(());

impl EmbeddingBackfillGuard {
    pub(crate) fn try_acquire() -> Option<Self> {
        EMBEDDING_BACKFILL_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then_some(Self(()))
    }

    pub(crate) fn is_running() -> bool {
        EMBEDDING_BACKFILL_RUNNING.load(Ordering::Acquire)
    }
}

impl Drop for EmbeddingBackfillGuard {
    fn drop(&mut self) {
        EMBEDDING_BACKFILL_RUNNING.store(false, Ordering::Release);
    }
}

/// 需要（重新）嵌入的远记忆记录：没有向量，或维度与当前嵌入模型不一致
/// （换了嵌入模型之后旧向量无法与新查询向量比较，向量召回会整体退回关键词）。
pub(crate) fn archive_records_needing_embedding(
    store: &dyn VectorStore,
    dim: usize,
) -> Vec<VectorRecord> {
    let mut records: Vec<VectorRecord> = store
        .records_matching(&MetadataFilter::new())
        .into_iter()
        .filter(|r| r.kind == VectorKind::ArchivedSummary && r.vector.len() != dim)
        .filter(|r| !r.content.trim().is_empty())
        .collect();
    records.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    records
}

/// 远记忆记录总数 / 其中缺向量的条数（`dim = None` 表示未配置嵌入模型，全部按缺失计）。
pub(crate) fn archive_embedding_counts(
    store: &dyn VectorStore,
    dim: Option<usize>,
) -> (usize, usize) {
    let records: Vec<VectorRecord> = store
        .records_matching(&MetadataFilter::new())
        .into_iter()
        .filter(|r| r.kind == VectorKind::ArchivedSummary)
        .collect();
    let missing = records
        .iter()
        .filter(|r| match dim {
            Some(dim) => r.vector.len() != dim,
            None => r.vector.is_empty(),
        })
        .count();
    (records.len(), missing)
}

/// 为给定记录补建向量：每 16 条一次批量嵌入，成功一批落盘一批。
///
/// 嵌入接口失败时立即停下（多半是配置或网络问题，继续请求只会重复失败、白白计费），
/// 已完成的批次保留。补建期间被删除的记录（删档）不会被写回。
pub(crate) async fn embed_archive_records(
    store: &dyn VectorStore,
    embedder: &storyforge_infra_llm::Embedder,
    records: Vec<VectorRecord>,
) -> MemoryEmbeddingReport {
    const CHUNK: usize = 16;
    let mut report = MemoryEmbeddingReport {
        candidates: records.len(),
        ..Default::default()
    };
    for chunk in records.chunks(CHUNK) {
        let texts: Vec<&str> = chunk.iter().map(|r| r.content.as_str()).collect();
        let vectors = match embedder.embed_batch(&texts).await {
            Ok(vectors) if vectors.len() == chunk.len() => vectors,
            Ok(vectors) => {
                report.error = Some(format!(
                    "嵌入接口返回 {} 条向量，请求了 {} 条",
                    vectors.len(),
                    chunk.len()
                ));
                break;
            }
            Err(e) => {
                report.error = Some(e.to_string());
                break;
            }
        };
        let updated: Vec<VectorRecord> = chunk
            .iter()
            .zip(vectors)
            .filter(|(record, _)| store.contains(&record.id))
            .map(|(record, vector)| VectorRecord {
                vector,
                ..record.clone()
            })
            .collect();
        let written = updated.len();
        if let Err(e) = store.upsert_batch(updated) {
            report.error = Some(e.to_string());
            break;
        }
        report.embedded += written;
    }
    report.failed = report.candidates - report.embedded;
    report
}

/// 用当前嵌入配置为远记忆补建向量（`only` 给定时只处理这些记录）。
///
/// 未配置嵌入模型、或已有补建在跑时返回 `None`。
pub(crate) async fn run_embedding_backfill(
    state: &AppState,
    only: Option<&[Id]>,
) -> Option<Result<MemoryEmbeddingReport, String>> {
    let config = state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()?;
    let _guard = EmbeddingBackfillGuard::try_acquire()?;
    let dim = config.dim;
    let embedder = match storyforge_infra_llm::Embedder::new(config) {
        Ok(embedder) => embedder,
        Err(e) => return Some(Err(e.to_string())),
    };
    let mut records = archive_records_needing_embedding(state.vector_store.as_ref(), dim);
    if let Some(only) = only {
        records.retain(|r| only.contains(&r.id));
    }
    let report = embed_archive_records(state.vector_store.as_ref(), &embedder, records).await;
    if report.candidates > 0 {
        tracing::info!(
            target: "far_memory",
            "远记忆向量补建：{} 条中完成 {}，失败 {}{}",
            report.candidates,
            report.embedded,
            report.failed,
            report
                .error
                .as_deref()
                .map(|e| format!("（{e}）"))
                .unwrap_or_default()
        );
    }
    Some(Ok(report))
}

/// 后台补建（导入后、保存嵌入配置后）。没配嵌入模型或已有补建在跑时什么也不做。
pub(crate) fn spawn_embedding_backfill(state: &Arc<AppState>, only: Option<Vec<Id>>) {
    if state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .is_none()
    {
        return;
    }
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(Err(e)) = run_embedding_backfill(&state, only.as_deref()).await {
            tracing::warn!(target: "far_memory", "后台远记忆向量补建失败: {e}");
        }
    });
}

#[cfg(test)]
#[path = "lib_tests_vector_index.rs"]
mod tests;
