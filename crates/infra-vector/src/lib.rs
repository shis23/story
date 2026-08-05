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
                            tracing::error!("向量库 JSON 主文件和 .tmp 备份均损坏，文件: {}, 错误: {}. 已保存 .corrupt 备份", path.display(), e);
                            let _ = std::fs::copy(&path, path.with_extension("json.corrupt"));
                            HashMap::new()
                        })
                }),
                Err(e) => {
                    warn!("向量库文件读取失败({e})，尝试 .tmp 备份");
                    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
                    std::fs::read_to_string(&tmp)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_else(|| {
                            tracing::error!("向量库文件读取失败且无可用备份，文件: {}, IO 错误: {}. 已保存 .corrupt 备份", path.display(), e);
                            let _ = std::fs::copy(&path, path.with_extension("json.corrupt"));
                            HashMap::new()
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
}

impl Default for BruteForceStore {
    fn default() -> Self {
        Self::new()
    }
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
        records.insert(record.id.clone(), record);
        self.persist_records(&records)?;
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
        let mut scored: Vec<VectorHit> = records
            .values()
            .filter(|r| filter.accepts(r))
            .map(|r| {
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
                Ok(VectorHit {
                    id: r.id.clone(),
                    content: r.content.clone(),
                    score,
                    kind: r.kind.clone(),
                    keywords: r.keywords.clone(),
                    metadata: r.metadata.clone(),
                })
            })
            .collect::<Result<Vec<_>, VectorError>>()?;

        // 按分数降序排序
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(top_k);
        Ok(scored)
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

        let mut hits: Vec<VectorHit> = records
            .values()
            .filter(|r| filter.accepts(r))
            .filter(|r| {
                r.keywords.iter().any(|kw| {
                    let kw_lower = kw.to_lowercase();
                    keywords_lower.iter().any(|q| kw_lower.contains(q.as_str()))
                })
            })
            .map(|r| VectorHit {
                id: r.id.clone(),
                content: r.content.clone(),
                score: 1.0, // 关键词匹配无分数
                kind: r.kind.clone(),
                keywords: r.keywords.clone(),
                metadata: r.metadata.clone(),
            })
            .collect();

        hits.truncate(limit);
        Ok(hits)
    }

    fn delete(&self, id: &Id) -> Result<(), VectorError> {
        let mut records = self.records.write().unwrap_or_else(|p| p.into_inner());
        records.remove(id);
        self.persist_records(&records)?;
        Ok(())
    }

    fn delete_by_campaign(&self, campaign_id: &Id) -> Result<usize, VectorError> {
        let mut records = self.records.write().unwrap_or_else(|p| p.into_inner());
        let target = serde_json::Value::String(campaign_id.to_string());
        let before = records.len();
        records.retain(|_, r| r.metadata.get("campaign_id") != Some(&target));
        let removed = before - records.len();
        self.persist_records(&records)?;
        Ok(removed)
    }

    fn delete_by_character(&self, character_id: &Id) -> Result<usize, VectorError> {
        let mut records = self.records.write().unwrap_or_else(|p| p.into_inner());
        let target = serde_json::Value::String(character_id.to_string());
        let before = records.len();
        records.retain(|_, r| r.metadata.get("owner_character_id") != Some(&target));
        let removed = before - records.len();
        self.persist_records(&records)?;
        Ok(removed)
    }

    fn count(&self) -> usize {
        self.records.read().unwrap_or_else(|p| p.into_inner()).len()
    }
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
}
