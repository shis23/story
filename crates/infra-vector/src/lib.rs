/// 向量存储（对应设计 §7.4）
///
/// M0 用暴力余弦相似度，M2 引入 hnsw_rs 做 ANN。
/// 持久化用 JSON（与 ST 数据同构，便于导出）。
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use storyforge_domain::Id;
use tracing::warn;

// ─── 数据模型 ──────────────────────────────────────────────────────────────

/// 向量记录类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VectorKind {
    /// 归档的远记忆总结
    ArchivedSummary,
    /// 绿灯世界书条目
    WorldInfo,
    /// 角色设定片段
    Character,
    /// 角色可见信息（character_knowledge，D39 新增）
    ///
    /// metadata 必含：owner_character_id / campaign_id / source
    CharacterKnowledge,
}

/// 向量记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorRecord {
    pub id: Id,
    pub content: String,
    pub vector: Vec<f32>,
    pub keywords: Vec<String>,
    pub kind: VectorKind,
    /// 结构化标签（D39：owner_character_id / campaign_id / source 等）
    ///
    /// 用于按角色/会话隔离过滤。注入子 Agent 上下文时按标签筛选。
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// 向量搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorHit {
    pub id: Id,
    pub content: String,
    pub score: f32,
    pub kind: VectorKind,
    pub keywords: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// 标签过滤条件（按 metadata 键值匹配）
///
/// 用于角色知识隔离：只检索某角色 / 某 campaign 的记录。
/// 多个条件为 AND 关系（全满足才命中）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetadataFilter {
    /// (key, expected_value) 对，全满足才命中
    pub matches: Vec<(String, serde_json::Value)>,
}

impl MetadataFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 加一个匹配条件
    pub fn match_eq(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.matches.push((key.into(), value));
        self
    }

    /// 按角色 ID 过滤（character_knowledge 用）
    pub fn for_character(self, character_id: &Id) -> Self {
        self.match_eq(
            "owner_character_id",
            serde_json::Value::String(character_id.to_string()),
        )
    }

    /// 按 campaign 过滤（会话隔离）
    pub fn for_campaign(mut self, campaign_id: &Id) -> Self {
        self.matches.push((
            "campaign_id".into(),
            serde_json::Value::String(campaign_id.to_string()),
        ));
        self
    }

    /// 检查某记录是否满足过滤条件
    pub fn accepts(&self, record: &VectorRecord) -> bool {
        for (key, expected) in &self.matches {
            match record.metadata.get(key) {
                Some(actual) if actual == expected => continue,
                _ => return false,
            }
        }
        true
    }
}

// ─── 向量存储 trait ────────────────────────────────────────────────────────

/// 向量存储 trait（抽象接口）
pub trait VectorStore: Send + Sync {
    /// 插入或更新向量记录
    fn upsert(&self, record: VectorRecord) -> Result<(), VectorError>;

    /// 按向量相似度搜索（ANN 或暴力）
    fn search_by_vector(&self, query: &[f32], top_k: usize) -> Result<Vec<VectorHit>, VectorError>;

    /// 按向量相似度搜索，带标签过滤（D39：角色/会话隔离）
    fn search_by_vector_filtered(
        &self,
        query: &[f32],
        top_k: usize,
        filter: &MetadataFilter,
    ) -> Result<Vec<VectorHit>, VectorError>;

    /// 按关键词过滤候选
    fn search_by_keywords(
        &self,
        keywords: &[String],
        limit: usize,
    ) -> Result<Vec<VectorHit>, VectorError>;

    /// 按关键词过滤候选，带标签过滤
    fn search_by_keywords_filtered(
        &self,
        keywords: &[String],
        limit: usize,
        filter: &MetadataFilter,
    ) -> Result<Vec<VectorHit>, VectorError>;

    /// 删除记录
    fn delete(&self, id: &Id) -> Result<(), VectorError>;

    /// 按 campaign 删除所有记录（删档时清理）
    fn delete_by_campaign(&self, campaign_id: &Id) -> Result<usize, VectorError>;

    /// 按 character 删除所有记录（删卡时级联清理，M-2）
    fn delete_by_character(&self, character_id: &Id) -> Result<usize, VectorError>;

    /// 批量插入或更新：一次落盘，任一步失败整体回滚（内存不留半批）。
    fn upsert_batch(&self, records: Vec<VectorRecord>) -> Result<(), VectorError>;

    /// 列出满足标签过滤的全部记录（派生索引对账 / 分支复制向量用）。
    fn records_matching(&self, filter: &MetadataFilter) -> Vec<VectorRecord>;

    /// 是否已有该 id 的记录（对账补索引用，不克隆记录）。
    fn contains(&self, id: &Id) -> bool;

    /// 用 `records` 整体替换某一类记录（派生索引全量重建）：一次落盘，失败回滚。
    /// 返回被移除的旧记录数。
    fn replace_kind(
        &self,
        kind: VectorKind,
        records: Vec<VectorRecord>,
    ) -> Result<usize, VectorError>;

    /// 记录总数
    fn count(&self) -> usize;
}

// ─── 错误类型 ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum VectorError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("序列化错误: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("维度不匹配: 期望 {expected}，实际 {actual}（记录 id={id}）")]
    DimensionMismatch {
        expected: usize,
        actual: usize,
        id: String,
    },

    /// H-7：BruteForceStore 全内存，长期运行 campaign 远记忆持续累积无上限会
    /// 导致内存不可逆增长。新增配额，超限拒绝 upsert。
    #[error("向量库配额超限: 当前 {current} 条，上限 {limit} 条")]
    QuotaExceeded { current: usize, limit: usize },

    #[error("索引错误: {0}")]
    Index(String),
}

// ─── 暴力检索实现（M0，设计 §7.4 决策）─────────────────────────────────────

/// H-7：BruteForceStore 全内存，长期运行 campaign 远记忆持续累积无上限会
/// 导致内存不可逆增长。默认配额 5 万条（每条含向量+正文，约 4KB → ~200MB 上界）。
pub const DEFAULT_MAX_RECORDS: usize = 50_000;

/// 暴力向量存储（M0：Vec + 余弦相似度，数据量小时够用）
pub struct BruteForceStore {
    records: RwLock<HashMap<Id, VectorRecord>>,
    persist_path: Option<PathBuf>,
    /// H-7：记录数上限。超过时新 id 的 upsert 返回 `QuotaExceeded`；已存在 id
    /// 的替换不计入增长（覆盖旧记录）。
    max_records: usize,
}

impl BruteForceStore {
    pub fn new() -> Self {
        Self {
            records: RwLock::new(HashMap::new()),
            persist_path: None,
            max_records: DEFAULT_MAX_RECORDS,
        }
    }

    /// 带持久化的构造
    pub fn with_persistence(path: PathBuf) -> Self {
        Self::with_persistence_and_quota(path, DEFAULT_MAX_RECORDS)
    }

    /// H-7：带持久化 + 自定义配额的构造。
    pub fn with_persistence_and_quota(path: PathBuf, max_records: usize) -> Self {
        let records = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(data) => serde_json::from_str(&data).unwrap_or_else(|e| {
                    tracing::warn!("向量库 JSON 解析失败({e})，尝试 .tmp 备份");
                    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
                    std::fs::read_to_string(&tmp)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_else(|| {
                            quarantine_unrecoverable_corruption(&path, &e.to_string())
                        })
                }),
                Err(e) => {
                    warn!("向量库文件读取失败({e})，尝试 .tmp 备份");
                    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
                    std::fs::read_to_string(&tmp)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_else(|| {
                            quarantine_unrecoverable_corruption(&path, &e.to_string())
                        })
                }
            }
        } else {
            HashMap::new()
        };

        Self {
            records: RwLock::new(records),
            persist_path: Some(path),
            max_records,
        }
    }

    /// H-7：自定义配额的非持久化构造（测试/内存模式）。
    pub fn with_max_records(max_records: usize) -> Self {
        Self {
            records: RwLock::new(HashMap::new()),
            persist_path: None,
            max_records,
        }
    }

    /// 持久化到文件（原子写：.tmp → rename，由调用者持有写锁时调用，避免竞态）
    fn persist_records(&self, records: &HashMap<Id, VectorRecord>) -> Result<(), VectorError> {
        if let Some(path) = &self.persist_path {
            storyforge_infra_util::atomic_write_json(path, records)?;
        }
        Ok(())
    }

    /// 批量删除满足谓词的记录；落盘失败时把删掉的记录放回内存再返回错误。
    fn delete_where(&self, matches: impl Fn(&VectorRecord) -> bool) -> Result<usize, VectorError> {
        self.apply_changes(matches, Vec::new())
    }

    /// 单次落盘的批量变更：先删掉满足 `remove` 的记录，再写入 `insert`。
    ///
    /// 配额只拦**净增长**超限（与单条 upsert 一致：覆盖已存在 id 不算增长）；
    /// 落盘失败时把内存恢复到变更前，避免内存与磁盘分叉。返回删除的记录数。
    fn apply_changes(
        &self,
        remove: impl Fn(&VectorRecord) -> bool,
        insert: Vec<VectorRecord>,
    ) -> Result<usize, VectorError> {
        let mut records = self.records.write().unwrap_or_else(|p| p.into_inner());
        let doomed: Vec<Id> = records
            .values()
            .filter(|r| remove(r))
            .map(|r| r.id.clone())
            .collect();
        if doomed.is_empty() && insert.is_empty() {
            return Ok(0);
        }
        let before = records.len();
        let mut final_ids: std::collections::HashSet<&Id> = records.keys().collect();
        for id in &doomed {
            final_ids.remove(id);
        }
        for record in &insert {
            final_ids.insert(&record.id);
        }
        let after = final_ids.len();
        if after > self.max_records && after > before {
            return Err(VectorError::QuotaExceeded {
                current: before,
                limit: self.max_records,
            });
        }

        let mut undo: HashMap<Id, Option<VectorRecord>> = HashMap::new();
        let removed = doomed.len();
        for id in doomed {
            let old = records.remove(&id);
            undo.entry(id).or_insert(old);
        }
        for record in insert {
            let id = record.id.clone();
            let old = records.insert(id.clone(), record);
            undo.entry(id).or_insert(old);
        }
        if let Err(e) = self.persist_records(&records) {
            for (id, old) in undo {
                match old {
                    Some(old) => {
                        records.insert(id, old);
                    }
                    None => {
                        records.remove(&id);
                    }
                }
            }
            return Err(e);
        }
        Ok(removed)
    }
}

impl Default for BruteForceStore {
    fn default() -> Self {
        Self::new()
    }
}

/// 主文件与 `.tmp` 备份都损坏时的兜底（D-03）。
///
/// 两件事都必须做，缺一即数据永久丢失：
/// 1. 把损坏的原文件复制成 `.corrupt` 抢救副本——**复制失败不再是 `let _ =`**，
///    而是显式 `error!`（这是用户数据唯一的抢救来源）；
/// 2. 冻结该路径的写栅栏（V4）：损坏未确认前 `atomic_write` 一律拒绝，
///    否则下一次 `upsert` 会把内存里的空集写回主文件，把"可抢救的损坏文件"
///    变成"干净的空文件"。
///
/// 上层登记与解除：tauri-app `storage_health::record_write_fence_state()` 启动时把
/// `write_fence::frozen_entries()` 登记为阻断事件，用户在 `StorageHealthGate` 确认后
/// 经 `storage_health_acknowledge` → `write_fence::unfreeze` 解除。
fn quarantine_unrecoverable_corruption(
    path: &std::path::Path,
    err: &str,
) -> HashMap<Id, VectorRecord> {
    let backup = path.with_extension("json.corrupt");
    tracing::error!(
        "向量库主文件与 .tmp 备份均损坏，文件: {}，错误: {}。尝试保存 .corrupt 备份并冻结写入（D-03）",
        path.display(),
        err
    );
    let backup_note = match std::fs::copy(path, &backup) {
        Ok(_) => {
            tracing::error!("已保存损坏备份: {}", backup.display());
            format!("已保存抢救副本 {}", backup.display())
        }
        Err(copy_err) => {
            tracing::error!(
                "损坏备份保存失败({copy_err})，原文件保持原样: {}；已冻结写入避免覆盖未抢救数据",
                path.display()
            );
            format!("抢救副本保存失败({copy_err})，原文件保持原样")
        }
    };
    storyforge_infra_util::write_fence::freeze_with_reason(
        path,
        &format!("向量库主文件与 .tmp 均无法解析（{err}）；{backup_note}"),
    );
    HashMap::new()
}

impl VectorStore for BruteForceStore {
    fn upsert(&self, record: VectorRecord) -> Result<(), VectorError> {
        let mut records = self.records.write().unwrap_or_else(|p| p.into_inner());
        // H-7：配额校验。替换已存在 id 不计入增长（覆盖），仅新 id 触发上限检查。
        let is_new = !records.contains_key(&record.id);
        if is_new && records.len() >= self.max_records {
            return Err(VectorError::QuotaExceeded {
                current: records.len(),
                limit: self.max_records,
            });
        }
        let id = record.id.clone();
        let previous = records.insert(id.clone(), record);
        // 落盘失败时回滚内存（2026-10-11 审计）：否则调用方收到 Err 而内存里已有
        // 该记录，内存与磁盘分叉——归档器据 Err 不推进水位、下次换新 id 重归档，
        // 进程内就会出现重复远记忆。
        if let Err(e) = self.persist_records(&records) {
            match previous {
                Some(old) => {
                    records.insert(id, old);
                }
                None => {
                    records.remove(&id);
                }
            }
            return Err(e);
        }
        Ok(())
    }

    fn search_by_vector(&self, query: &[f32], top_k: usize) -> Result<Vec<VectorHit>, VectorError> {
        self.search_by_vector_filtered(query, top_k, &MetadataFilter::default())
    }

    fn search_by_vector_filtered(
        &self,
        query: &[f32],
        top_k: usize,
        filter: &MetadataFilter,
    ) -> Result<Vec<VectorHit>, VectorError> {
        let records = self.records.read().unwrap_or_else(|p| p.into_inner());
        // H-7：维度不匹配改为返回错误（之前静默跳过 = 结果集缺条却不报错，损坏记录
        // 污染检索且无法排查）。第一条不匹配即短路返回，调用方可定位并修复损坏记录。
        // 只收集 (score, id) 再取 top-k：全量克隆每条记录（content/keywords/metadata）
        // 在配额上限（50k × ~4KB）下是 ~200MB 级瞬态分配（2026-09-01 全量审查修复）。
        //
        // 2026-10-11 审计：空向量记录是**设计内**的关键词专用记录（绿灯世界书条目、
        // 嵌入完成前的 RoundSummary），不是损坏数据——向量路径必须跳过它们。此前它们
        // 也被判 DimensionMismatch，导致只要库里有一条世界书条目，配置了 Embedder 的
        // 远记忆混合召回就整体失败（连关键词命中一起丢）。全零向量同样没有方向、
        // 无法定义余弦分数，按"无向量"跳过而不是误报维度错误。
        let mut scored: Vec<(f32, Id)> = Vec::with_capacity(records.len());
        for r in records.values().filter(|r| filter.accepts(r)) {
            if r.vector.is_empty() || r.vector.iter().all(|x| *x == 0.0) {
                continue;
            }
            let score = cosine_similarity(query, &r.vector).ok_or_else(|| {
                warn!(
                    "向量维度不匹配，查询 {} 维 vs 记录 id={} {} 维",
                    query.len(),
                    r.id.as_str(),
                    r.vector.len()
                );
                VectorError::DimensionMismatch {
                    expected: query.len(),
                    actual: r.vector.len(),
                    id: r.id.to_string(),
                }
            })?;
            scored.push((score, r.id.clone()));
        }

        // 按分数降序排序；同分按 id 升序，保证 top-k 结果稳定（D-05）
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.as_str().cmp(b.1.as_str()))
        });
        scored.truncate(top_k);
        // 二次查表克隆仅 top-k 条赢家
        let hits = scored
            .into_iter()
            .filter_map(|(score, id)| {
                records.get(&id).map(|r| VectorHit {
                    id: r.id.clone(),
                    content: r.content.clone(),
                    score,
                    kind: r.kind.clone(),
                    keywords: r.keywords.clone(),
                    metadata: r.metadata.clone(),
                })
            })
            .collect();
        Ok(hits)
    }

    fn search_by_keywords(
        &self,
        keywords: &[String],
        limit: usize,
    ) -> Result<Vec<VectorHit>, VectorError> {
        self.search_by_keywords_filtered(keywords, limit, &MetadataFilter::default())
    }

    fn search_by_keywords_filtered(
        &self,
        keywords: &[String],
        limit: usize,
        filter: &MetadataFilter,
    ) -> Result<Vec<VectorHit>, VectorError> {
        let records = self.records.read().unwrap_or_else(|p| p.into_inner());
        let keywords_lower: Vec<String> = keywords.iter().map(|k| k.to_lowercase()).collect();

        // D-05：HashMap 迭代序随机，此前 `truncate(limit)` 会返回随机的子集。
        // 先按"命中的关键词条数"降序、再按 id 升序排序，保证同输入同输出。
        let match_count = |r: &VectorRecord| -> usize {
            r.keywords
                .iter()
                .filter(|kw| {
                    let kw_lower = kw.to_lowercase();
                    keywords_lower.iter().any(|q| kw_lower.contains(q.as_str()))
                })
                .count()
        };
        let mut hits: Vec<(usize, VectorHit)> = records
            .values()
            .filter(|r| filter.accepts(r))
            .filter_map(|r| {
                let matched = match_count(r);
                if matched == 0 {
                    return None;
                }
                Some((
                    matched,
                    VectorHit {
                        id: r.id.clone(),
                        content: r.content.clone(),
                        // 关键词匹配无相似度分数（score 语义保持不变）
                        score: 1.0,
                        kind: r.kind.clone(),
                        keywords: r.keywords.clone(),
                        metadata: r.metadata.clone(),
                    },
                ))
            })
            .collect();

        hits.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.id.as_str().cmp(b.1.id.as_str()))
        });
        hits.truncate(limit);
        Ok(hits.into_iter().map(|(_, hit)| hit).collect())
    }

    fn delete(&self, id: &Id) -> Result<(), VectorError> {
        let mut records = self.records.write().unwrap_or_else(|p| p.into_inner());
        let Some(removed) = records.remove(id) else {
            return Ok(());
        };
        if let Err(e) = self.persist_records(&records) {
            records.insert(id.clone(), removed);
            return Err(e);
        }
        Ok(())
    }

    fn delete_by_campaign(&self, campaign_id: &Id) -> Result<usize, VectorError> {
        let target = serde_json::Value::String(campaign_id.to_string());
        self.delete_where(|r| r.metadata.get("campaign_id") == Some(&target))
    }

    fn delete_by_character(&self, character_id: &Id) -> Result<usize, VectorError> {
        let target = serde_json::Value::String(character_id.to_string());
        self.delete_where(|r| r.metadata.get("owner_character_id") == Some(&target))
    }

    fn upsert_batch(&self, records: Vec<VectorRecord>) -> Result<(), VectorError> {
        self.apply_changes(|_| false, records).map(|_| ())
    }

    fn contains(&self, id: &Id) -> bool {
        self.records
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id)
    }

    fn records_matching(&self, filter: &MetadataFilter) -> Vec<VectorRecord> {
        let records = self.records.read().unwrap_or_else(|p| p.into_inner());
        let mut out: Vec<VectorRecord> = records
            .values()
            .filter(|r| filter.accepts(r))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        out
    }

    fn replace_kind(
        &self,
        kind: VectorKind,
        records: Vec<VectorRecord>,
    ) -> Result<usize, VectorError> {
        if let Some(stray) = records.iter().find(|r| r.kind != kind) {
            return Err(VectorError::Index(format!(
                "replace_kind({kind:?}) 收到其它类型的记录 {}（{:?}）",
                stray.id, stray.kind
            )));
        }
        self.apply_changes(|r| r.kind == kind, records)
    }

    fn count(&self) -> usize {
        self.records.read().unwrap_or_else(|p| p.into_inner()).len()
    }
}

// ─── 查询切词 ──────────────────────────────────────────────────────────────

/// 从自然语言查询抽取检索 token（中英混合）。
///
/// - 英文/数字：按空白与标点切词，长度 ≥ 2
/// - 中文等非 ASCII：抽 2-gram，覆盖无空格连续文本
///
/// 与 `search_by_keywords*` 的「记录关键词包含查询 token」匹配语义配套；远记忆
/// 召回（app-memory）与 Agent `search_vectors` 工具共用，避免两套切词漂移。
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

// ─── 余弦相似度 ────────────────────────────────────────────────────────────

/// 余弦相似度（暴力计算）
fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return None;
    }

    Some(dot / (norm_a * norm_b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(id: &str, content: &str, keywords: Vec<&str>) -> VectorRecord {
        VectorRecord {
            id: Id::from_str(id),
            content: content.into(),
            vector: vec![1.0, 0.0, 0.0], // 简化向量
            keywords: keywords.into_iter().map(String::from).collect(),
            kind: VectorKind::ArchivedSummary,
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_brute_force_upsert_and_count() {
        let store = BruteForceStore::new();
        store
            .upsert(make_record("r1", "内容1", vec!["关键词"]))
            .unwrap();
        store
            .upsert(make_record("r2", "内容2", vec!["其他"]))
            .unwrap();
        assert_eq!(store.count(), 2);
    }

    #[test]
    fn test_brute_force_search_by_vector() {
        let store = BruteForceStore::new();
        store.upsert(make_record("r1", "相似", vec![])).unwrap();
        store
            .upsert(VectorRecord {
                id: Id::from_str("r2"),
                content: "不同".into(),
                vector: vec![0.0, 1.0, 0.0], // 正交
                keywords: vec![],
                kind: VectorKind::ArchivedSummary,
                metadata: HashMap::new(),
            })
            .unwrap();

        let hits = store.search_by_vector(&[1.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id.as_str(), "r1");
        assert!(hits[0].score > 0.99); // 几乎完全相似
    }

    #[test]
    fn test_brute_force_search_by_keywords() {
        let store = BruteForceStore::new();
        store
            .upsert(make_record("r1", "龙的故事", vec!["龙", "冒险"]))
            .unwrap();
        store
            .upsert(make_record("r2", "城市生活", vec!["城市", "日常"]))
            .unwrap();

        let hits = store.search_by_keywords(&["龙".into()], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id.as_str(), "r1");
    }

    #[test]
    fn test_brute_force_delete() {
        let store = BruteForceStore::new();
        store.upsert(make_record("r1", "内容", vec![])).unwrap();
        assert_eq!(store.count(), 1);

        store.delete(&Id::from_str("r1")).unwrap();
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b).unwrap() - 1.0).abs() < 0.001);

        let c = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &c).unwrap() - 0.0).abs() < 0.001);

        let d = vec![1.0, 1.0, 0.0];
        let score = cosine_similarity(&a, &d).unwrap();
        assert!((score - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
    }

    #[test]
    fn test_persistence() {
        let dir =
            std::env::temp_dir().join(format!("storyforge_test_vec_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vectors.json");

        // 写入
        {
            let store = BruteForceStore::with_persistence(path.clone());
            store
                .upsert(make_record("r1", "持久化测试", vec!["test"]))
                .unwrap();
        }

        // 重新加载
        {
            let store = BruteForceStore::with_persistence(path.clone());
            assert_eq!(store.count(), 1);
            let hits = store.search_by_keywords(&["test".into()], 10).unwrap();
            assert_eq!(hits.len(), 1);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_persistence_recovers_from_tmp_when_main_json_is_corrupt() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_vec_recover_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vectors.json");
        let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));

        let record = make_record("r1", "备份恢复", vec!["backup"]);
        let mut records = HashMap::new();
        records.insert(record.id.clone(), record);
        std::fs::write(&path, "{not valid json").unwrap();
        std::fs::write(&tmp_path, serde_json::to_string(&records).unwrap()).unwrap();

        let store = BruteForceStore::with_persistence(path.clone());

        assert_eq!(store.count(), 1);
        let hits = store.search_by_keywords(&["backup".into()], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id.as_str(), "r1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_persistence_copies_corrupt_main_json_when_no_backup_exists() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_vec_corrupt_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vectors.json");
        std::fs::write(&path, "{not valid json").unwrap();

        let store = BruteForceStore::with_persistence(path.clone());

        assert_eq!(store.count(), 0);
        assert!(path.with_extension("json.corrupt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_record_with_meta(
        id: &str,
        content: &str,
        keywords: Vec<&str>,
        character_id: &str,
        campaign_id: &str,
    ) -> VectorRecord {
        let mut meta = HashMap::new();
        meta.insert(
            "owner_character_id".into(),
            serde_json::Value::String(character_id.into()),
        );
        meta.insert(
            "campaign_id".into(),
            serde_json::Value::String(campaign_id.into()),
        );
        VectorRecord {
            id: Id::from_str(id),
            content: content.into(),
            vector: vec![1.0, 0.0, 0.0],
            keywords: keywords.into_iter().map(String::from).collect(),
            kind: VectorKind::CharacterKnowledge,
            metadata: meta,
        }
    }

    #[test]
    fn test_search_by_vector_filtered_by_character() {
        let store = BruteForceStore::new();
        // 林医生的知识
        store
            .upsert(make_record_with_meta(
                "k1",
                "林医生看到了爆炸",
                vec!["爆炸"],
                "char-lin",
                "camp-1",
            ))
            .unwrap();
        // 陈警官的知识
        store
            .upsert(make_record_with_meta(
                "k2",
                "陈警官调查现场",
                vec!["爆炸"],
                "char-chen",
                "camp-1",
            ))
            .unwrap();

        // 只查林医生的：应只返回 k1
        let filter = MetadataFilter::new()
            .for_character(&Id::from_str("char-lin"))
            .for_campaign(&Id::from_str("camp-1"));
        let hits = store
            .search_by_vector_filtered(&[1.0, 0.0, 0.0], 10, &filter)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id.as_str(), "k1");
    }

    #[test]
    fn test_search_by_keywords_filtered_by_campaign() {
        let store = BruteForceStore::new();
        store
            .upsert(make_record_with_meta(
                "k1",
                "爆炸事件",
                vec!["爆炸"],
                "char-lin",
                "camp-1",
            ))
            .unwrap();
        store
            .upsert(make_record_with_meta(
                "k2",
                "爆炸事件2",
                vec!["爆炸"],
                "char-lin",
                "camp-2",
            ))
            .unwrap();

        // 只查 camp-1 的
        let filter = MetadataFilter::new().for_campaign(&Id::from_str("camp-1"));
        let hits = store
            .search_by_keywords_filtered(&["爆炸".into()], 10, &filter)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id.as_str(), "k1");
    }

    #[test]
    fn test_delete_by_campaign() {
        let store = BruteForceStore::new();
        store
            .upsert(make_record_with_meta(
                "k1",
                "a",
                vec![],
                "char-lin",
                "camp-1",
            ))
            .unwrap();
        store
            .upsert(make_record_with_meta(
                "k2",
                "b",
                vec![],
                "char-chen",
                "camp-1",
            ))
            .unwrap();
        store
            .upsert(make_record_with_meta(
                "k3",
                "c",
                vec![],
                "char-lin",
                "camp-2",
            ))
            .unwrap();

        assert_eq!(store.count(), 3);
        let removed = store.delete_by_campaign(&Id::from_str("camp-1")).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn test_empty_filter_matches_all() {
        let store = BruteForceStore::new();
        store.upsert(make_record("r1", "a", vec![])).unwrap();
        store.upsert(make_record("r2", "b", vec![])).unwrap();

        let filter = MetadataFilter::new();
        let hits = store
            .search_by_vector_filtered(&[1.0, 0.0, 0.0], 10, &filter)
            .unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn test_delete_by_character() {
        let store = BruteForceStore::new();
        store
            .upsert(make_record_with_meta(
                "k1",
                "林医生的知识",
                vec![],
                "char-lin",
                "camp-1",
            ))
            .unwrap();
        store
            .upsert(make_record_with_meta(
                "k2",
                "陈警官的知识",
                vec![],
                "char-chen",
                "camp-1",
            ))
            .unwrap();
        store
            .upsert(make_record_with_meta(
                "k3",
                "林医生的另一条",
                vec![],
                "char-lin",
                "camp-2",
            ))
            .unwrap();

        assert_eq!(store.count(), 3);
        let removed = store
            .delete_by_character(&Id::from_str("char-lin"))
            .unwrap();
        assert_eq!(removed, 2);
        assert_eq!(store.count(), 1);
        // 只剩陈警官的
        let filter = MetadataFilter::new().for_character(&Id::from_str("char-chen"));
        let hits = store
            .search_by_vector_filtered(&[1.0, 0.0, 0.0], 10, &filter)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id.as_str(), "k2");
    }

    /// H-7：超出配额的新 id upsert 必须被拒（返回 QuotaExceeded）。
    #[test]
    fn test_upsert_rejects_new_id_beyond_quota() {
        let store = BruteForceStore::with_max_records(2);
        store.upsert(make_record("r1", "a", vec![])).unwrap();
        store.upsert(make_record("r2", "b", vec![])).unwrap();
        // 第三条新 id → 拒绝。
        let err = store
            .upsert(make_record("r3", "c", vec![]))
            .expect_err("超过配额的新 id 必须被拒");
        assert!(
            matches!(
                err,
                VectorError::QuotaExceeded {
                    current: 2,
                    limit: 2
                }
            ),
            "expected QuotaExceeded, got {err:?}"
        );
        assert_eq!(store.count(), 2);
    }

    /// H-7：替换已存在 id 不触发配额（覆盖不计入增长）。
    #[test]
    fn test_upsert_replacing_existing_id_does_not_trigger_quota() {
        let store = BruteForceStore::with_max_records(1);
        store.upsert(make_record("r1", "a", vec![])).unwrap();
        // 同 id 覆盖 → 允许。
        store
            .upsert(make_record("r1", "更新内容", vec![]))
            .expect("同 id 覆盖不应触发配额");
        assert_eq!(store.count(), 1);
    }

    /// H-7：维度不匹配必须返回 DimensionMismatch 错误（之前静默跳过）。
    #[test]
    fn test_search_returns_dimension_mismatch_error() {
        let store = BruteForceStore::new();
        // 记录是 3 维，用 2 维查询 → 不匹配。
        store.upsert(make_record("r1", "a", vec![])).unwrap();
        let err = store
            .search_by_vector(&[1.0, 0.0], 10)
            .expect_err("维度不匹配必须返回错误");
        assert!(
            matches!(
                err,
                VectorError::DimensionMismatch {
                    expected: 2,
                    actual: 3,
                    ..
                }
            ),
            "expected DimensionMismatch, got {err:?}"
        );
    }

    // ─── D-03：不可恢复损坏必须备份 + 冻结写入，不得静默固化空态 ──────────

    fn temp_store_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sf_vector_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("vectors.json")
    }

    #[test]
    fn unrecoverable_corruption_freezes_writes_and_keeps_backup() {
        let path = temp_store_path("corrupt");
        std::fs::write(&path, b"{ this is not json").unwrap();

        let store = BruteForceStore::with_persistence(path.clone());
        assert_eq!(store.count(), 0, "损坏文件按空库启动");

        // 1) 抢救副本必须存在，且内容与损坏原文件一致
        let backup = path.with_extension("json.corrupt");
        assert!(backup.exists(), "必须留下 .corrupt 抢救副本（D-03）");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "{ this is not json"
        );

        // 2) 写栅栏必须生效：下一次保存不得把空态固化到主文件
        assert!(
            storyforge_infra_util::write_fence::is_frozen(&path),
            "不可恢复损坏必须冻结写入（D-03）"
        );
        let err = store
            .upsert(make_record("r1", "新内容", vec![]))
            .expect_err("冻结后写入必须失败，而不是把空库写回主文件");
        assert!(
            matches!(err, VectorError::Io(ref e) if e.kind() == std::io::ErrorKind::PermissionDenied),
            "expected PermissionDenied(Io)，实际 {err:?}"
        );
        // 原文件内容未被覆盖
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ this is not json"
        );

        // 3) 冻结状态必须在健康面可见：哪份文件被隔离、为什么、怎么处理（Lead 裁定）
        let entries = storyforge_infra_util::write_fence::frozen_entries();
        let entry = entries
            .iter()
            .find(|e| e.path.contains("vectors.json"))
            .expect("冻结条目必须出现在 write_fence 诊断快照里");
        assert!(
            entry.reason.contains("无法解析"),
            "原因必须写明解析失败：{}",
            entry.reason
        );
        assert!(
            entry.reason.contains("corrupt"),
            "原因必须指出抢救副本位置：{}",
            entry.reason
        );
        let report = storyforge_infra_util::write_fence::storage_health_report();
        let (kind, detail) = report
            .iter()
            .find(|(_, d)| d.contains("vectors.json"))
            .expect("storage_health 报告必须包含被隔离的向量库");
        assert_eq!(kind, "write_fence_frozen");
        assert!(
            detail.contains("已被隔离") && detail.contains("处理建议"),
            "报告文案必须含隔离说明与人工处理步骤：{detail}"
        );

        // 清理全局栅栏状态，避免影响其它测试
        storyforge_infra_util::write_fence::unfreeze(&path);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn tmp_backup_recovery_does_not_freeze() {
        let path = temp_store_path("tmpbackup");
        std::fs::write(&path, b"{ broken").unwrap();
        // .tmp 里有合法内容 → 可恢复，不应冻结
        let mut good = HashMap::new();
        good.insert(
            Id::from_str("r-recovered"),
            make_record("r-recovered", "备份内容", vec!["备份"]),
        );
        std::fs::write(
            PathBuf::from(format!("{}.tmp", path.display())),
            serde_json::to_string(&good).unwrap(),
        )
        .unwrap();

        let store = BruteForceStore::with_persistence(path.clone());
        assert_eq!(store.count(), 1, "应从 .tmp 备份恢复");
        assert!(
            !storyforge_infra_util::write_fence::is_frozen(&path),
            "可恢复时不得冻结写入"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    // ─── D-05：搜索顺序必须确定 ───────────────────────────────────────────

    #[test]
    fn keyword_search_truncation_is_deterministic() {
        let store = BruteForceStore::new();
        // 6 条记录都命中同一关键词，limit=3：此前 HashMap 序随机 → 结果子集随机
        for i in 0..6 {
            store
                .upsert(make_record(&format!("r{i}"), "内容", vec!["龙"]))
                .unwrap();
        }
        let first: Vec<String> = store
            .search_by_keywords(&["龙".into()], 3)
            .unwrap()
            .into_iter()
            .map(|h| h.id.as_str().to_string())
            .collect();
        assert_eq!(
            first,
            vec!["r0", "r1", "r2"],
            "必须按 id 升序稳定截断（D-05）"
        );

        for _ in 0..5 {
            let again: Vec<String> = store
                .search_by_keywords(&["龙".into()], 3)
                .unwrap()
                .into_iter()
                .map(|h| h.id.as_str().to_string())
                .collect();
            assert_eq!(again, first, "同输入必须同输出（D-05）");
        }
    }

    #[test]
    fn keyword_search_ranks_more_matches_first() {
        let store = BruteForceStore::new();
        store
            .upsert(make_record("r-one", "单命中", vec!["龙"]))
            .unwrap();
        store
            .upsert(make_record("r-two", "双命中", vec!["龙", "冒险"]))
            .unwrap();
        let hits = store
            .search_by_keywords(&["龙".into(), "冒险".into()], 10)
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id.as_str(), "r-two", "命中更多关键词的排前面");
        assert_eq!(hits[0].score, 1.0, "score 语义保持不变");
    }

    #[test]
    fn vector_search_ties_break_by_id() {
        let store = BruteForceStore::new();
        // 三条相同向量 → 分数完全相同，top-2 必须稳定
        for i in 0..3 {
            store
                .upsert(make_record(&format!("r{i}"), "同分", vec![]))
                .unwrap();
        }
        let ids: Vec<String> = store
            .search_by_vector(&[1.0, 0.0, 0.0], 2)
            .unwrap()
            .into_iter()
            .map(|h| h.id.as_str().to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["r0", "r1"],
            "同分必须按 id 升序决定 top-k（D-05）"
        );
    }

    /// 2026-10-11 审计：关键词专用记录（空向量）与全零向量不是损坏数据，向量路径
    /// 必须跳过而不是让整次检索 DimensionMismatch。
    #[test]
    fn vector_search_skips_keyword_only_and_zero_vectors() {
        let store = BruteForceStore::new();
        store
            .upsert(make_record("embedded", "已嵌入", vec!["诊所"]))
            .unwrap();
        store
            .upsert(VectorRecord {
                id: Id::from_str("wi-keyword-only"),
                content: "绿灯世界书条目".into(),
                vector: vec![],
                keywords: vec!["诊所".into()],
                kind: VectorKind::WorldInfo,
                metadata: HashMap::new(),
            })
            .unwrap();
        store
            .upsert(VectorRecord {
                id: Id::from_str("zero"),
                content: "全零向量".into(),
                vector: vec![0.0, 0.0, 0.0],
                keywords: vec![],
                kind: VectorKind::ArchivedSummary,
                metadata: HashMap::new(),
            })
            .unwrap();

        let hits = store
            .search_by_vector(&[1.0, 0.0, 0.0], 10)
            .expect("关键词专用记录不得让向量检索失败");
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, vec!["embedded"]);

        // 非空且维度不同的记录仍是真正的维度错误（H-7 语义保持）。
        store
            .upsert(VectorRecord {
                id: Id::from_str("bad-dim"),
                content: "二维".into(),
                vector: vec![1.0, 0.0],
                keywords: vec![],
                kind: VectorKind::ArchivedSummary,
                metadata: HashMap::new(),
            })
            .unwrap();
        assert!(matches!(
            store.search_by_vector(&[1.0, 0.0, 0.0], 10),
            Err(VectorError::DimensionMismatch { actual: 2, .. })
        ));
    }

    /// 2026-10-11 审计：落盘失败必须回滚内存，否则 Err 返回后内存与磁盘分叉。
    #[test]
    fn persist_failure_rolls_back_in_memory_mutations() {
        let path = temp_store_path("rollback");
        let store = BruteForceStore::with_persistence(path.clone());
        let mut tagged = make_record("kept", "已落盘", vec!["k"]);
        tagged.metadata.insert(
            "campaign_id".into(),
            serde_json::Value::String("camp-1".into()),
        );
        store.upsert(tagged).unwrap();

        storyforge_infra_util::write_fence::freeze(&path);
        assert!(store.upsert(make_record("new", "新增", vec![])).is_err());
        assert_eq!(store.count(), 1, "新增失败不得残留在内存");
        assert!(
            store
                .upsert(make_record("kept", "覆盖", vec!["k"]))
                .is_err()
        );
        let kept = store.search_by_keywords(&["k".into()], 10).unwrap();
        assert_eq!(kept[0].content, "已落盘", "覆盖失败必须恢复旧值");
        assert!(store.delete(&Id::from_str("kept")).is_err());
        assert_eq!(store.count(), 1, "删除失败必须放回记录");
        assert!(store.delete_by_campaign(&Id::from_str("camp-1")).is_err());
        assert_eq!(store.count(), 1, "批量删除失败必须放回记录");
        storyforge_infra_util::write_fence::unfreeze(&path);

        assert_eq!(
            store.delete_by_campaign(&Id::from_str("camp-1")).unwrap(),
            1
        );
        assert_eq!(store.count(), 0);
        let reopened = BruteForceStore::with_persistence(path.clone());
        assert_eq!(reopened.count(), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    fn world_info(id: &str, content: &str) -> VectorRecord {
        VectorRecord {
            kind: VectorKind::WorldInfo,
            ..make_record(id, content, vec!["wi"])
        }
    }

    #[test]
    fn upsert_batch_persists_once_and_rolls_back_as_a_whole() {
        let path = temp_store_path("batch");
        let store = BruteForceStore::with_persistence(path.clone());
        store
            .upsert_batch(vec![
                make_record("a", "甲", vec!["k"]),
                make_record("b", "乙", vec!["k"]),
            ])
            .unwrap();
        assert_eq!(BruteForceStore::with_persistence(path.clone()).count(), 2);

        storyforge_infra_util::write_fence::freeze(&path);
        let err = store.upsert_batch(vec![
            make_record("a", "甲-改", vec!["k"]),
            make_record("c", "丙", vec!["k"]),
        ]);
        storyforge_infra_util::write_fence::unfreeze(&path);
        assert!(err.is_err());
        assert_eq!(store.count(), 2, "失败的批次不得留下新记录");
        let a = store
            .records_matching(&MetadataFilter::new())
            .into_iter()
            .find(|r| r.id.as_str() == "a")
            .unwrap();
        assert_eq!(a.content, "甲", "失败的批次必须恢复被覆盖的旧值");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn upsert_batch_enforces_quota_on_net_growth_only() {
        let store = BruteForceStore::with_max_records(2);
        store
            .upsert_batch(vec![
                make_record("a", "1", vec![]),
                make_record("b", "2", vec![]),
            ])
            .unwrap();
        assert!(matches!(
            store.upsert_batch(vec![make_record("c", "3", vec![])]),
            Err(VectorError::QuotaExceeded { .. })
        ));
        // 覆盖已有 id 不算增长。
        store
            .upsert_batch(vec![make_record("a", "1'", vec![])])
            .unwrap();
        assert_eq!(store.count(), 2);
    }

    #[test]
    fn replace_kind_swaps_only_that_kind() {
        let store = BruteForceStore::new();
        store.upsert(world_info("old-wi", "旧条目")).unwrap();
        store
            .upsert(make_record("summary", "摘要", vec!["k"]))
            .unwrap();

        let removed = store
            .replace_kind(VectorKind::WorldInfo, vec![world_info("new-wi", "新条目")])
            .unwrap();
        assert_eq!(removed, 1);
        let ids: Vec<String> = store
            .records_matching(&MetadataFilter::new())
            .into_iter()
            .map(|r| r.id.to_string())
            .collect();
        assert_eq!(ids, vec!["new-wi".to_string(), "summary".to_string()]);

        assert!(
            store
                .replace_kind(
                    VectorKind::WorldInfo,
                    vec![make_record("x", "类型不符", vec![])]
                )
                .is_err(),
            "混入其它类型的记录必须拒绝，避免误删"
        );
        assert_eq!(store.count(), 2);
    }

    #[test]
    fn records_matching_filters_by_metadata() {
        let store = BruteForceStore::new();
        let mut tagged = make_record("t", "带标签", vec![]);
        tagged
            .metadata
            .insert("campaign_id".into(), serde_json::json!("camp-1"));
        store.upsert(tagged).unwrap();
        store.upsert(make_record("u", "无标签", vec![])).unwrap();
        let hits =
            store.records_matching(&MetadataFilter::new().for_campaign(&Id::from_str("camp-1")));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id.as_str(), "t");
    }
}
