//! Campaign / CharacterCard / CharacterInstance 持久化
//!
//! 分文件（对应 P1/P2/P3 + 世界书分层）：
//! - data/cards.json            —— CharacterCard（含 character_definitions）
//! - data/campaigns.json        —— Campaign
//! - data/instances.json        —— CharacterInstance（按 campaign_id 索引）
//! - data/knowledge.json        —— CharacterKnowledgeEntry（角色可见信息，P2 新增）
//! - data/tasks.json            —— StoryTask（叙事计划任务，P2 新增）
//! - data/round_summaries.json  —— RoundSummary（本轮剧情摘要，P2 新增）
//! - data/mvu_translations.json —— StoredMvuTranslation（MVU 五合一产物，P3 新增）
//! - data/campaign_world_info/{campaign_id}.json —— 本局世界书（卡模板只读，活动可写）
//!
//! 与现有 CharacterStore（扁平 Character）并存，向后兼容。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::{CharacterCard, RoleType};
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
use storyforge_domain::mvu_translation::MvuTranslation;
use storyforge_domain::story_task::StoryTask;
use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};

// ─── CharacterCard 存储 ────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredCard {
    pub card: CharacterCard,
    pub imported_at: String,
}

/// MVU 翻译存储（带 source_character_id 索引 + 分析时间）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredMvuTranslation {
    pub source_character_id: Id,
    pub character_name: String,
    pub translation: MvuTranslation,
    pub analyzed_at: String,
}

/// Gate 4 story-clock authority repair for a batch of JSON-loaded campaigns.
fn repair_story_clock_authority(campaigns: &mut [storyforge_domain::campaign::Campaign]) {
    for campaign in campaigns.iter_mut() {
        if campaign.repair_story_clock_authority() {
            tracing::warn!(
                campaign_id = %campaign.id,
                "campaign story_clock field diverged from variables authority; repaired from variables (JSON store)"
            );
        }
    }
}

pub const FORCE_RERUN_BLOCKED_BY_CAMPAIGN: &str =
    "这张角色卡已有游玩档，暂不支持重新识别；请先导入一份新卡再重跑识别。";

/// upsert 三态结果（Phase A 幂等重放用）。
///
/// - `Inserted`：ID 不存在，已插入。
/// - `AlreadyPresent`：ID 存在且 payload 完全一致，no-op（幂等重放）。
/// - `Conflict`：ID 存在但 payload 不一致（数据腐败或实现错误）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpsertResult {
    Inserted,
    AlreadyPresent,
    Conflict(String),
}

pub struct CampaignStore {
    cards_path: PathBuf,
    campaigns_path: PathBuf,
    instances_path: PathBuf,
    knowledge_path: PathBuf,
    tasks_path: PathBuf,
    summaries_path: PathBuf,
    mvu_path: PathBuf,
    /// 本局世界书目录（每活动一文件，避免 campaigns.json 被 400+ 条目撑爆）
    world_info_dir: PathBuf,
    /// 集合级缓存锁。写盘仍在对应集合锁内串行，避免锁外旧快照覆盖新快照。
    cards: Mutex<Vec<StoredCard>>,
    campaigns: Mutex<Vec<Campaign>>,
    instances: Mutex<Vec<CharacterInstance>>,
    knowledge: Mutex<Vec<CharacterKnowledgeEntry>>,
    tasks: Mutex<Vec<StoryTask>>,
    summaries: Mutex<Vec<RoundSummary>>,
    mvu: Mutex<Vec<StoredMvuTranslation>>,
    /// campaign_id → 本局世界书（惰性装入）
    world_info: Mutex<std::collections::HashMap<String, WorldInfoBook>>,
    /// When SQLite is the process authority, this legacy JSON store must not
    /// be read as a fallback or used as a secondary write target.
    json_access_disabled: AtomicBool,
}

impl CampaignStore {
    pub fn new(data_dir: &Path) -> Self {
        let cards_path = data_dir.join("cards.json");
        let campaigns_path = data_dir.join("campaigns.json");
        let instances_path = data_dir.join("instances.json");
        let knowledge_path = data_dir.join("knowledge.json");
        let tasks_path = data_dir.join("tasks.json");
        let summaries_path = data_dir.join("round_summaries.json");
        let mvu_path = data_dir.join("mvu_translations.json");
        let world_info_dir = data_dir.join("campaign_world_info");
        let _ = std::fs::create_dir_all(&world_info_dir);

        Self {
            cards: Mutex::new(load_or_default(&cards_path)),
            campaigns: Mutex::new(load_or_default(&campaigns_path)),
            instances: Mutex::new(load_or_default(&instances_path)),
            knowledge: Mutex::new(load_or_default(&knowledge_path)),
            tasks: Mutex::new(load_or_default(&tasks_path)),
            summaries: Mutex::new(load_or_default(&summaries_path)),
            mvu: Mutex::new(load_or_default(&mvu_path)),
            world_info: Mutex::new(std::collections::HashMap::new()),
            json_access_disabled: AtomicBool::new(false),
            cards_path,
            campaigns_path,
            instances_path,
            knowledge_path,
            tasks_path,
            summaries_path,
            mvu_path,
            world_info_dir,
        }
    }

    /// Construct a non-I/O sentinel for an SQLite-authoritative process.
    /// Reads return no JSON-derived data and every write fails closed.
    pub fn disabled() -> Self {
        Self {
            cards_path: PathBuf::new(),
            campaigns_path: PathBuf::new(),
            instances_path: PathBuf::new(),
            knowledge_path: PathBuf::new(),
            tasks_path: PathBuf::new(),
            summaries_path: PathBuf::new(),
            mvu_path: PathBuf::new(),
            world_info_dir: PathBuf::new(),
            cards: Mutex::new(Vec::new()),
            campaigns: Mutex::new(Vec::new()),
            world_info: Mutex::new(std::collections::HashMap::new()),
            instances: Mutex::new(Vec::new()),
            knowledge: Mutex::new(Vec::new()),
            tasks: Mutex::new(Vec::new()),
            summaries: Mutex::new(Vec::new()),
            mvu: Mutex::new(Vec::new()),
            json_access_disabled: AtomicBool::new(true),
        }
    }

    /// Permanently close this process-local JSON handle after an opt-in
    /// backend has become authoritative. This only protects against an
    /// accidental lazy initialization before backend resolution.
    pub fn disable_json_access(&self) {
        self.json_access_disabled.store(true, Ordering::Release);
    }

    fn json_access_disabled(&self) -> bool {
        self.json_access_disabled.load(Ordering::Acquire)
    }

    fn ensure_json_write_allowed(&self) -> Result<(), String> {
        if self.json_access_disabled() {
            return Err(
                "legacy JSON CampaignStore is disabled while SQLite is authoritative".into(),
            );
        }
        Ok(())
    }

    /// Directory that owns the JSON files for this store (for reload/verify).
    pub fn data_dir(&self) -> Option<&Path> {
        if self.json_access_disabled() {
            return None;
        }
        self.cards_path.parent()
    }

    // ─── CharacterCard CRUD ───────────────────────────────────────────────

    pub fn list_cards(&self) -> Vec<StoredCard> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        self.cards.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn get_card(&self, id: &Id) -> Option<StoredCard> {
        if self.json_access_disabled() {
            return None;
        }
        self.cards
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|c| c.card.id == *id)
            .cloned()
    }

    pub fn get_card_by_source(&self, source_character_id: &Id) -> Option<StoredCard> {
        if self.json_access_disabled() {
            return None;
        }
        self.cards
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|c| c.card.source_character_id == *source_character_id)
            .cloned()
    }

    pub fn save_card(&self, card: CharacterCard) -> Result<StoredCard, String> {
        self.ensure_json_write_allowed()?;
        let mut cards = self.cards.lock().unwrap_or_else(|p| p.into_inner());
        // 同 source_character_id 去重（重跑识别时覆盖）
        cards.retain(|c| c.card.source_character_id != card.source_character_id);
        let stored = StoredCard {
            card,
            imported_at: chrono::Utc::now().to_rfc3339(),
        };
        cards.push(stored.clone());
        persist(&self.cards_path, &cards)?;
        Ok(stored)
    }

    pub fn save_card_if_no_campaigns(&self, card: CharacterCard) -> Result<StoredCard, String> {
        self.ensure_json_write_allowed()?;
        let mut cards = self.cards.lock().unwrap_or_else(|p| p.into_inner());
        let campaigns = self.campaigns.lock().unwrap_or_else(|p| p.into_inner());

        let source_character_id = card.source_character_id.clone();
        let mut replaced_card_ids = vec![card.id.clone()];
        for existing in cards
            .iter()
            .filter(|c| c.card.source_character_id == source_character_id)
        {
            if !replaced_card_ids.contains(&existing.card.id) {
                replaced_card_ids.push(existing.card.id.clone());
            }
        }

        if campaigns
            .iter()
            .any(|campaign| replaced_card_ids.contains(&campaign.card_id))
        {
            return Err(FORCE_RERUN_BLOCKED_BY_CAMPAIGN.to_string());
        }

        cards.retain(|c| c.card.source_character_id != source_character_id);
        let stored = StoredCard {
            card,
            imported_at: chrono::Utc::now().to_rfc3339(),
        };
        cards.push(stored.clone());
        persist(&self.cards_path, &cards)?;
        Ok(stored)
    }

    pub fn update_card(&self, card: CharacterCard) -> Result<Option<StoredCard>, String> {
        self.ensure_json_write_allowed()?;
        let mut cards = self.cards.lock().unwrap_or_else(|p| p.into_inner());
        let stored = StoredCard {
            card,
            imported_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Some(idx) = cards.iter().position(|c| c.card.id == stored.card.id) {
            cards[idx] = stored.clone();
            persist(&self.cards_path, &cards)?;
            Ok(Some(stored))
        } else {
            Ok(None)
        }
    }

    pub fn delete_card(&self, id: &Id) -> Result<bool, String> {
        self.ensure_json_write_allowed()?;
        let mut cards = self.cards.lock().unwrap_or_else(|p| p.into_inner());
        let mut campaigns = self.campaigns.lock().unwrap_or_else(|p| p.into_inner());
        let mut instances = self.instances.lock().unwrap_or_else(|p| p.into_inner());
        let mut knowledge = self.knowledge.lock().unwrap_or_else(|p| p.into_inner());
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        let mut summaries = self.summaries.lock().unwrap_or_else(|p| p.into_inner());
        let mut mvu = self.mvu.lock().unwrap_or_else(|p| p.into_inner());

        let before = cards.len();
        // 先记下要删的卡的 source_character_id（用于级联删 MVU 翻译）
        let source_ids: Vec<Id> = cards
            .iter()
            .filter(|c| c.card.id == *id)
            .map(|c| c.card.source_character_id.clone())
            .collect();
        cards.retain(|c| c.card.id != *id);
        let changed = cards.len() != before;
        if changed {
            persist(&self.cards_path, &cards)?;
            // 级联删除：该卡的 campaign + instances
            let camp_ids: Vec<Id> = campaigns
                .iter()
                .filter(|c| c.card_id == *id)
                .map(|c| c.id.clone())
                .collect();
            for camp_id in &camp_ids {
                campaigns.retain(|c| c.id != *camp_id);
                instances.retain(|i| i.campaign_id != *camp_id);
                knowledge.retain(|k| k.campaign_id != *camp_id);
                tasks.retain(|t| t.campaign_id != *camp_id);
                summaries.retain(|s| s.campaign_id != *camp_id);
            }
            persist(&self.campaigns_path, &campaigns)?;
            persist(&self.instances_path, &instances)?;
            persist(&self.knowledge_path, &knowledge)?;
            persist(&self.tasks_path, &tasks)?;
            persist(&self.summaries_path, &summaries)?;
            // 级联删除：该卡的 MVU 翻译
            for source_id in &source_ids {
                mvu.retain(|m| m.source_character_id != *source_id);
            }
            persist(&self.mvu_path, &mvu)?;
        }
        Ok(changed)
    }

    // ─── Campaign CRUD ────────────────────────────────────────────────────

    pub fn list_campaigns(&self) -> Vec<Campaign> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        let mut campaigns: Vec<Campaign> = self
            .campaigns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        repair_story_clock_authority(&mut campaigns);
        campaigns
    }

    pub fn list_campaigns_of_card(&self, card_id: &Id) -> Vec<Campaign> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        let mut campaigns: Vec<Campaign> = self
            .campaigns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|c| c.card_id == *card_id)
            .cloned()
            .collect();
        repair_story_clock_authority(&mut campaigns);
        campaigns
    }

    pub fn get_campaign(&self, id: &Id) -> Option<Campaign> {
        if self.json_access_disabled() {
            return None;
        }
        let mut campaign: Option<Campaign> = self
            .campaigns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|c| c.id == *id)
            .cloned();
        if let Some(c) = campaign.as_mut()
            && c.repair_story_clock_authority()
        {
            tracing::warn!(
                campaign_id = %c.id,
                "campaign story_clock field diverged from variables authority; repaired from variables (JSON store)"
            );
        }
        campaign
    }

    pub fn save_campaign(&self, campaign: Campaign) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        let mut campaigns = self.campaigns.lock().unwrap_or_else(|p| p.into_inner());
        campaigns.retain(|c| c.id != campaign.id);
        campaigns.push(campaign);
        persist(&self.campaigns_path, &campaigns)
    }

    pub fn create_campaign_with_instances(
        &self,
        campaign: Campaign,
    ) -> Result<(StoredCard, Campaign, usize), String> {
        self.ensure_json_write_allowed()?;
        let cards = self.cards.lock().unwrap_or_else(|p| p.into_inner());
        let mut campaigns = self.campaigns.lock().unwrap_or_else(|p| p.into_inner());
        let mut instances = self.instances.lock().unwrap_or_else(|p| p.into_inner());

        let stored = cards
            .iter()
            .find(|c| c.card.id == campaign.card_id)
            .cloned()
            .ok_or_else(|| format!("card not found: {}", campaign.card_id))?;

        let mut next_campaigns = campaigns.clone();
        next_campaigns.retain(|c| c.id != campaign.id);
        next_campaigns.push(campaign.clone());

        let mut next_instances = instances.clone();
        let instance_len_before_retain = next_instances.len();
        next_instances.retain(|i| i.campaign_id != campaign.id);
        let removed_existing_instances = next_instances.len() != instance_len_before_retain;

        let mut instance_count = 0;
        for def in &stored.card.character_definitions {
            if matches!(def.role_type, RoleType::Protagonist | RoleType::Supporting) {
                next_instances.push(CharacterInstance::from_definition(campaign.id.clone(), def));
                instance_count += 1;
            }
        }
        let instances_changed = removed_existing_instances || instance_count > 0;

        if instances_changed {
            persist(&self.instances_path, &next_instances)?;
        }
        if let Err(err) = persist(&self.campaigns_path, &next_campaigns) {
            if instances_changed {
                let _ = persist(&self.instances_path, &instances);
            }
            return Err(err);
        }

        if instances_changed {
            *instances = next_instances;
        }
        *campaigns = next_campaigns;

        Ok((stored, campaign, instance_count))
    }

    pub fn update_campaign(&self, campaign: Campaign) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        let mut campaigns = self.campaigns.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = campaigns.iter().position(|c| c.id == campaign.id) {
            // 先 persist 再写内存，避免磁盘失败时内存已清空 epoch/marker
            let mut next = campaigns.clone();
            next[idx] = campaign;
            persist(&self.campaigns_path, &next)?;
            *campaigns = next;
        }
        Ok(())
    }

    pub fn delete_campaign(&self, id: &Id) -> Result<bool, String> {
        self.ensure_json_write_allowed()?;
        // World-info reads/writes and Campaign deletion use the same outer lock.
        // The lock is acquired before checking Campaign existence so a writer
        // cannot pass that check and recreate a file after deletion.
        let mut world_info = self.world_info.lock().unwrap_or_else(|p| p.into_inner());
        let mut campaigns = self.campaigns.lock().unwrap_or_else(|p| p.into_inner());
        let mut instances = self.instances.lock().unwrap_or_else(|p| p.into_inner());
        let mut knowledge = self.knowledge.lock().unwrap_or_else(|p| p.into_inner());
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        let mut summaries = self.summaries.lock().unwrap_or_else(|p| p.into_inner());

        let changed = campaigns.iter().any(|campaign| campaign.id == *id);
        if !changed {
            return Ok(false);
        }

        let next_campaigns: Vec<_> = campaigns
            .iter()
            .filter(|campaign| campaign.id != *id)
            .cloned()
            .collect();
        let next_instances: Vec<_> = instances
            .iter()
            .filter(|instance| instance.campaign_id != *id)
            .cloned()
            .collect();
        let next_knowledge: Vec<_> = knowledge
            .iter()
            .filter(|entry| entry.campaign_id != *id)
            .cloned()
            .collect();
        let next_tasks: Vec<_> = tasks
            .iter()
            .filter(|task| task.campaign_id != *id)
            .cloned()
            .collect();
        let next_summaries: Vec<_> = summaries
            .iter()
            .filter(|summary| summary.campaign_id != *id)
            .cloned()
            .collect();

        // Persist all JSON files before mutating any in-memory cache. If a later
        // file fails, restore every file already written so deletion remains
        // retryable instead of leaving a half-deleted aggregate.
        let original_campaigns_file = snapshot_path(&self.campaigns_path)?;
        let original_instances_file = snapshot_path(&self.instances_path)?;
        let original_knowledge_file = snapshot_path(&self.knowledge_path)?;
        let original_tasks_file = snapshot_path(&self.tasks_path)?;
        let original_summaries_file = snapshot_path(&self.summaries_path)?;
        for (path, snapshot) in [
            (&self.campaigns_path, original_campaigns_file),
            (&self.instances_path, original_instances_file),
            (&self.knowledge_path, original_knowledge_file),
            (&self.tasks_path, original_tasks_file),
            (&self.summaries_path, original_summaries_file),
        ] {
            ensure_regular_or_missing(path, snapshot)?;
        }
        let world_info_path = self.world_info_path(id);
        let original_world_info_file = snapshot_path(&world_info_path)?;
        ensure_regular_or_missing(&world_info_path, original_world_info_file)?;
        let original_world_info = match original_world_info_file {
            PathSnapshot::RegularFile => Some(
                std::fs::read_to_string(&world_info_path)
                    .map_err(|error| format!("读取本局世界书以备删除回滚失败: {error}"))?,
            ),
            PathSnapshot::Missing => None,
            PathSnapshot::Other => unreachable!("validated above"),
        };

        let write_result = (|| {
            persist(&self.campaigns_path, &next_campaigns)?;
            persist(&self.instances_path, &next_instances)?;
            persist(&self.knowledge_path, &next_knowledge)?;
            persist(&self.tasks_path, &next_tasks)?;
            persist(&self.summaries_path, &next_summaries)?;
            match snapshot_path(&world_info_path)? {
                current if current == original_world_info_file => {
                    if current == PathSnapshot::RegularFile {
                        std::fs::remove_file(&world_info_path)
                            .map_err(|error| format!("删除本局世界书失败: {error}"))?;
                    }
                }
                PathSnapshot::Other => {
                    return Err(format!(
                        "本局世界书路径不是普通文件，拒绝删除: {}",
                        world_info_path.display()
                    ));
                }
                current => {
                    return Err(format!(
                        "本局世界书路径在删除期间发生变化 ({current:?}): {}",
                        world_info_path.display()
                    ));
                }
            }
            Ok::<(), String>(())
        })();

        if let Err(error) = write_result {
            let mut rollback_errors = Vec::new();
            macro_rules! restore {
                ($path:expr, $existed:expr, $original:expr) => {
                    if let Err(rollback_error) = restore_json_file($path, $existed, $original) {
                        rollback_errors.push(rollback_error);
                    }
                };
            }
            restore!(&self.campaigns_path, original_campaigns_file, &campaigns);
            restore!(&self.instances_path, original_instances_file, &instances);
            restore!(&self.knowledge_path, original_knowledge_file, &knowledge);
            restore!(&self.tasks_path, original_tasks_file, &tasks);
            restore!(&self.summaries_path, original_summaries_file, &summaries);
            if let Err(rollback_error) = restore_world_info_file(
                &world_info_path,
                original_world_info_file,
                original_world_info.as_deref(),
            ) {
                rollback_errors.push(rollback_error);
            }

            return if rollback_errors.is_empty() {
                Err(error)
            } else {
                Err(format!(
                    "{error}; 删除回滚也失败，数据需要恢复: {}",
                    rollback_errors.join("; ")
                ))
            };
        }

        *campaigns = next_campaigns;
        *instances = next_instances;
        *knowledge = next_knowledge;
        *tasks = next_tasks;
        *summaries = next_summaries;
        world_info.remove(id.as_str());

        Ok(true)
    }

    // ─── Campaign 本局世界书（卡模板只读；活动可写）──────────────────────

    fn world_info_path(&self, campaign_id: &Id) -> PathBuf {
        self.world_info_dir
            .join(format!("{}.json", campaign_id.as_str()))
    }

    fn read_world_info_file(&self, campaign_id: &Id) -> Result<WorldInfoBook, String> {
        let path = self.world_info_path(campaign_id);
        if !path.exists() {
            return Ok(empty_world_info_book());
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| format!("read campaign world_info failed: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("parse campaign world_info failed: {e}"))
    }

    /// Apply one read-modify-write while retaining the world-info mutex across
    /// the file write. Shells can be mounted more than once for a Campaign;
    /// this makes independently delivered bridge updates atomic.
    fn mutate_world_info<T, F>(
        &self,
        campaign_id: &Id,
        mutate: F,
    ) -> Result<(T, WorldInfoBook), String>
    where
        F: FnOnce(&mut WorldInfoBook) -> Result<T, String>,
    {
        self.ensure_json_write_allowed()?;
        let mut map = self.world_info.lock().unwrap_or_else(|p| p.into_inner());
        if self.get_campaign(campaign_id).is_none() {
            return Err(format!("campaign not found: {}", campaign_id.as_str()));
        }
        let _ = std::fs::create_dir_all(&self.world_info_dir);
        let path = self.world_info_path(campaign_id);
        let mut book = match map.get(campaign_id.as_str()) {
            Some(book) => book.clone(),
            None => self.read_world_info_file(campaign_id)?,
        };
        let result = mutate(&mut book)?;
        storyforge_infra_util::atomic_write_json(&path, &book)
            .map_err(|e| format!("persist campaign world_info failed: {e}"))?;
        map.insert(campaign_id.as_str().to_string(), book.clone());
        Ok((result, book))
    }

    /// 读取本局世界书。文件不存在返回空书（调用方应 ensure/copy）。
    pub fn get_world_info(&self, campaign_id: &Id) -> Result<WorldInfoBook, String> {
        if self.json_access_disabled() {
            return Err(
                "legacy JSON CampaignStore is disabled while SQLite is authoritative".into(),
            );
        }
        let mut map = self.world_info.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(book) = map.get(campaign_id.as_str()) {
            return Ok(book.clone());
        }
        let book = self.read_world_info_file(campaign_id)?;
        map.insert(campaign_id.as_str().to_string(), book.clone());
        Ok(book)
    }

    /// 整本替换本局世界书并落盘。
    pub fn set_world_info(&self, campaign_id: &Id, book: WorldInfoBook) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        let mut map = self.world_info.lock().unwrap_or_else(|p| p.into_inner());
        if self.get_campaign(campaign_id).is_none() {
            return Err(format!("campaign not found: {}", campaign_id.as_str()));
        }
        let _ = std::fs::create_dir_all(&self.world_info_dir);
        let path = self.world_info_path(campaign_id);
        storyforge_infra_util::atomic_write_json(&path, &book)
            .map_err(|e| format!("persist campaign world_info failed: {e}"))?;
        map.insert(campaign_id.as_str().to_string(), book);
        Ok(())
    }

    /// 从卡模板书深拷贝到本局（开档 / 惰性迁移）。已有非空本局书则跳过。
    pub fn ensure_world_info_from_book(
        &self,
        campaign_id: &Id,
        template: &WorldInfoBook,
    ) -> Result<WorldInfoBook, String> {
        let mut book = template.clone();
        // 标记条目来自卡模板（extensions 上挂 source，便于 UI 区分 user 新增）
        for entry in &mut book.entries {
            if entry.extensions.is_null() {
                entry.extensions = serde_json::json!({ "sf_source": "card" });
            } else if let Some(obj) = entry.extensions.as_object_mut() {
                obj.entry("sf_source")
                    .or_insert_with(|| serde_json::json!("card"));
            }
        }
        let (_, persisted) = self.mutate_world_info(campaign_id, |existing| {
            if existing.entries.is_empty() {
                *existing = book;
            }
            Ok(())
        })?;
        Ok(persisted)
    }

    pub fn add_world_info_entry(
        &self,
        campaign_id: &Id,
        mut entry: WorldInfoEntry,
    ) -> Result<usize, String> {
        if entry.extensions.is_null() {
            entry.extensions = serde_json::json!({ "sf_source": "user" });
        } else if let Some(obj) = entry.extensions.as_object_mut() {
            obj.entry("sf_source")
                .or_insert_with(|| serde_json::json!("user"));
        }
        let (idx, _) = self.mutate_world_info(campaign_id, |book| {
            book.entries.push(entry);
            Ok(book.entries.len() - 1)
        })?;
        Ok(idx)
    }

    pub fn update_world_info_entry(
        &self,
        campaign_id: &Id,
        entry_index: usize,
        entry: WorldInfoEntry,
    ) -> Result<(), String> {
        self.mutate_world_info(campaign_id, |book| {
            if entry_index >= book.entries.len() {
                return Err(format!("世界书条目索引越界: {entry_index}"));
            }
            book.entries[entry_index] = entry;
            Ok(())
        })?;
        Ok(())
    }

    pub fn delete_world_info_entry(
        &self,
        campaign_id: &Id,
        entry_index: usize,
    ) -> Result<(), String> {
        self.mutate_world_info(campaign_id, |book| {
            if entry_index >= book.entries.len() {
                return Err(format!("世界书条目索引越界: {entry_index}"));
            }
            book.entries.remove(entry_index);
            Ok(())
        })?;
        Ok(())
    }

    pub fn set_world_info_route(
        &self,
        campaign_id: &Id,
        entry_index: usize,
        route: LoreRoute,
    ) -> Result<(), String> {
        self.mutate_world_info(campaign_id, |book| {
            if entry_index >= book.entries.len() {
                return Err(format!("世界书条目索引越界: {entry_index}"));
            }
            book.entries[entry_index].route = route;
            book.entries[entry_index].disabled =
                matches!(book.entries[entry_index].route, LoreRoute::Disabled);
            Ok(())
        })?;
        Ok(())
    }

    /// Toggle one entry without letting parallel card-shell callbacks replace
    /// a sibling entry with a stale full-book snapshot.
    pub fn set_world_info_entry_enabled(
        &self,
        campaign_id: &Id,
        entry_index: usize,
        enabled: bool,
    ) -> Result<WorldInfoBook, String> {
        let (_, book) = self.mutate_world_info(campaign_id, |book| {
            let entry = book
                .entries
                .get_mut(entry_index)
                .ok_or_else(|| format!("世界书条目索引越界: {entry_index}"))?;
            entry.set_enabled(enabled)
        })?;
        Ok(book)
    }

    // ─── CharacterInstance CRUD ───────────────────────────────────────────

    pub fn list_all_instances(&self) -> Vec<CharacterInstance> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        self.instances
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn list_instances(&self, campaign_id: &Id) -> Vec<CharacterInstance> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        self.instances
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|i| i.campaign_id == *campaign_id)
            .cloned()
            .collect()
    }

    pub fn get_instance(&self, campaign_id: &Id, instance_id: &Id) -> Option<CharacterInstance> {
        if self.json_access_disabled() {
            return None;
        }
        self.instances
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|i| i.campaign_id == *campaign_id && i.id == *instance_id)
            .cloned()
    }

    pub fn add_instance(&self, instance: CharacterInstance) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        let mut instances = self.instances.lock().unwrap_or_else(|p| p.into_inner());
        instances.retain(|i| i.id != instance.id);
        instances.push(instance);
        persist(&self.instances_path, &instances)
    }

    pub fn update_instance(&self, instance: CharacterInstance) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        let mut instances = self.instances.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = instances.iter().position(|i| i.id == instance.id) {
            instances[idx] = instance;
            persist(&self.instances_path, &instances)?;
        }
        Ok(())
    }

    // ─── CharacterKnowledge CRUD（P2 新增）─────────────────────────────────

    pub fn list_all_knowledge(&self) -> Vec<CharacterKnowledgeEntry> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        self.knowledge
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// 查某 campaign 下所有角色的知识条目
    pub fn list_knowledge(&self, campaign_id: &Id) -> Vec<CharacterKnowledgeEntry> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        self.knowledge
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|k| k.campaign_id == *campaign_id)
            .cloned()
            .collect()
    }

    /// 查某 campaign 下某角色的知识条目
    pub fn list_knowledge_of(
        &self,
        campaign_id: &Id,
        character_id: &Id,
    ) -> Vec<CharacterKnowledgeEntry> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        self.knowledge
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|k| k.campaign_id == *campaign_id && k.character_id == *character_id)
            .cloned()
            .collect()
    }

    /// 批量追加知识条目（后处理 Agent 产出后调用）
    pub fn add_knowledge(&self, entries: Vec<CharacterKnowledgeEntry>) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        if entries.is_empty() {
            return Ok(());
        }
        let mut knowledge = self.knowledge.lock().unwrap_or_else(|p| p.into_inner());
        knowledge.extend(entries);
        persist(&self.knowledge_path, &knowledge)
    }

    /// 删除单条知识条目（按 id），返回是否找到并删除
    pub fn delete_knowledge(&self, knowledge_id: &Id) -> Result<bool, String> {
        self.ensure_json_write_allowed()?;
        let mut knowledge = self.knowledge.lock().unwrap_or_else(|p| p.into_inner());
        let before = knowledge.len();
        knowledge.retain(|k| k.id != *knowledge_id);
        let changed = knowledge.len() != before;
        if changed {
            persist(&self.knowledge_path, &knowledge)?;
        }
        Ok(changed)
    }

    // ─── StoryTask CRUD（P2 新增）──────────────────────────────────────────

    pub fn list_all_tasks(&self) -> Vec<StoryTask> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        self.tasks.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// 查某 campaign 下所有任务（按 status 筛选：传 None 返回全部）
    pub fn list_tasks(&self, campaign_id: &Id) -> Vec<StoryTask> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        self.tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|t| t.campaign_id == *campaign_id)
            .cloned()
            .collect()
    }

    pub fn get_task(&self, task_id: &Id) -> Option<StoryTask> {
        if self.json_access_disabled() {
            return None;
        }
        self.tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|t| t.id == *task_id)
            .cloned()
    }

    /// 新建任务（用户规划或后处理抽取）
    pub fn add_task(&self, task: StoryTask) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        tasks.retain(|t| t.id != task.id);
        tasks.push(task);
        persist(&self.tasks_path, &tasks)
    }

    /// 更新任务（状态变化 / 注入记录 / 标完成）
    pub fn update_task(&self, task: StoryTask) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = tasks.iter().position(|t| t.id == task.id) {
            tasks[idx] = task;
            persist(&self.tasks_path, &tasks)?;
        }
        Ok(())
    }

    /// 删除任务
    pub fn delete_task(&self, task_id: &Id) -> Result<bool, String> {
        self.ensure_json_write_allowed()?;
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        let before = tasks.len();
        tasks.retain(|t| t.id != *task_id);
        let changed = tasks.len() != before;
        if changed {
            persist(&self.tasks_path, &tasks)?;
        }
        Ok(changed)
    }

    // ─── RoundSummary CRUD（P2 新增）───────────────────────────────────────

    pub fn list_all_summaries(&self) -> Vec<RoundSummary> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        self.summaries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// 查某 campaign 的所有本轮摘要（按 turn 升序）
    pub fn list_summaries(&self, campaign_id: &Id) -> Vec<RoundSummary> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        let mut out: Vec<_> = self
            .summaries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|s| s.campaign_id == *campaign_id)
            .cloned()
            .collect();
        out.sort_by_key(|s| s.turn);
        out
    }

    /// 追加一条本轮摘要（剧情总结 Agent 产出后调用）
    pub fn add_summary(&self, summary: RoundSummary) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        let mut summaries = self.summaries.lock().unwrap_or_else(|p| p.into_inner());
        summaries.retain(|s| !(s.campaign_id == summary.campaign_id && s.turn == summary.turn));
        summaries.push(summary);
        persist(&self.summaries_path, &summaries)
    }

    // ─── Phase A: 三态 upsert（TurnCommit 幂等重放用）─────────────────────

    /// 比较两个值的 payload 是否完全一致（用于幂等重放的 no-op 判定）。
    ///
    /// 用 serde_json 规范化比较，避免浮点精度或字段顺序差异导致误判。
    fn payloads_match<T: serde::Serialize>(a: &T, b: &T) -> bool {
        serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
    }

    /// 三态 upsert 知识条目（按 entry.id）。
    ///
    /// Phase A 幂等重放：重放时复用同一批 entry_id，payload 一致则 no-op。
    pub fn upsert_knowledge(&self, entry: CharacterKnowledgeEntry) -> Result<UpsertResult, String> {
        self.ensure_json_write_allowed()?;
        let mut knowledge = self.knowledge.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = knowledge.iter().position(|k| k.id == entry.id) {
            if Self::payloads_match(&knowledge[idx], &entry) {
                return Ok(UpsertResult::AlreadyPresent);
            }
            return Ok(UpsertResult::Conflict(format!(
                "knowledge entry id={} 已存在但 payload 不一致",
                entry.id
            )));
        }
        knowledge.push(entry);
        persist(&self.knowledge_path, &knowledge)?;
        Ok(UpsertResult::Inserted)
    }

    /// 三态 upsert 任务（按 task.id）。
    pub fn upsert_task(&self, task: StoryTask) -> Result<UpsertResult, String> {
        self.ensure_json_write_allowed()?;
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = tasks.iter().position(|t| t.id == task.id) {
            if Self::payloads_match(&tasks[idx], &task) {
                return Ok(UpsertResult::AlreadyPresent);
            }
            return Ok(UpsertResult::Conflict(format!(
                "task id={} 已存在但 payload 不一致",
                task.id
            )));
        }
        tasks.push(task);
        persist(&self.tasks_path, &tasks)?;
        Ok(UpsertResult::Inserted)
    }

    /// 三态 upsert 本轮摘要（按 campaign_id + turn 幂等键，payload 比较 content）。
    pub fn upsert_summary(&self, summary: RoundSummary) -> Result<UpsertResult, String> {
        self.ensure_json_write_allowed()?;
        let mut summaries = self.summaries.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = summaries
            .iter()
            .position(|s| s.campaign_id == summary.campaign_id && s.turn == summary.turn)
        {
            if Self::payloads_match(&summaries[idx], &summary) {
                return Ok(UpsertResult::AlreadyPresent);
            }
            return Ok(UpsertResult::Conflict(format!(
                "summary campaign={}? turn={} 已存在但 payload 不一致",
                summary.campaign_id, summary.turn
            )));
        }
        summaries.push(summary);
        persist(&self.summaries_path, &summaries)?;
        Ok(UpsertResult::Inserted)
    }

    /// 按 summary.id 更新（covered_by 折叠 / 字段修补）。
    pub fn update_summary_by_id(&self, summary: RoundSummary) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        let mut summaries = self.summaries.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = summaries.iter().position(|s| s.id == summary.id) {
            summaries[idx] = summary;
            return persist(&self.summaries_path, &summaries);
        }
        Err(format!("summary id={} 不存在", summary.id))
    }

    /// 插入 stage 纪要（B/C）：按 id 去重，不按 turn 覆盖 leaf A。
    pub fn insert_stage_summary(&self, summary: RoundSummary) -> Result<UpsertResult, String> {
        self.ensure_json_write_allowed()?;
        let mut summaries = self.summaries.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = summaries.iter().position(|s| s.id == summary.id) {
            if Self::payloads_match(&summaries[idx], &summary) {
                return Ok(UpsertResult::AlreadyPresent);
            }
            return Ok(UpsertResult::Conflict(format!(
                "stage summary id={} 已存在但 payload 不一致",
                summary.id
            )));
        }
        summaries.push(summary);
        persist(&self.summaries_path, &summaries)?;
        Ok(UpsertResult::Inserted)
    }

    /// 发布压缩结果：写 parent B/C + children.covered_by，bump chronicle_revision，清空 epoch。
    ///
    /// 半提交协议（整段在 `with_campaign_lock` 内，避免覆盖 Accept/Meta 并发写）：
    /// 1) 基于**锁内最新** Campaign 写 `pending_compress_publication`
    /// 2) summaries clone→persist→写回
    /// 3) 校验 parent 存在且 child→parent 覆盖成立后，完成 metadata 并清 marker
    ///
    /// heal 只在校验通过时完成；校验失败保留 marker。
    pub fn publish_compress_result(
        &self,
        campaign_id: &Id,
        parents: &[RoundSummary],
        child_covered_by: &[(Id, Id)],
    ) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        use storyforge_domain::chronicle::PendingCompressPublication;

        crate::turn_coordinator::with_campaign_lock(|| {
            let base_rev = self
                .get_campaign(campaign_id)
                .map(|c| c.chronicle_revision)
                .unwrap_or(0);
            let parent_ids: Vec<Id> = parents.iter().map(|p| p.id.clone()).collect();
            let pending =
                PendingCompressPublication::new(base_rev, parent_ids, child_covered_by.to_vec());

            // 1) intent（锁内最新 campaign）
            let mut camp = self.get_campaign(campaign_id).ok_or_else(|| {
                crate::turn_coordinator::CommitError::CampaignNotFound(campaign_id.clone())
            })?;
            camp.pending_compress_publication = Some(pending);
            self.update_campaign(camp)
                .map_err(crate::turn_coordinator::CommitError::Storage)?;

            // 2) summaries
            {
                let mut summaries = self.summaries.lock().unwrap_or_else(|p| p.into_inner());
                let mut next = summaries.clone();
                for (child_id, parent_id) in child_covered_by {
                    if let Some(s) = next.iter_mut().find(|s| s.id == *child_id) {
                        s.covered_by = Some(parent_id.clone());
                    }
                }
                for parent in parents {
                    if let Some(idx) = next.iter().position(|s| s.id == parent.id) {
                        if !Self::payloads_match(&next[idx], parent) {
                            return Err(crate::turn_coordinator::CommitError::Storage(format!(
                                "stage summary id={} conflict",
                                parent.id
                            )));
                        }
                    } else {
                        next.push(parent.clone());
                    }
                }
                persist(&self.summaries_path, &next)
                    .map_err(crate::turn_coordinator::CommitError::Storage)?;
                *summaries = next;
            }

            // 3) 校验后完成
            self.complete_compress_publication_inner(campaign_id)
                .map_err(crate::turn_coordinator::CommitError::Storage)?;
            Ok(())
        })
        .map_err(|e| e.to_string())
    }

    /// 若存在 pending_compress_publication，在锁内校验后完成 bump/clear；幂等。
    pub fn heal_compress_publication_metadata(&self, campaign_id: &Id) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        crate::turn_coordinator::with_campaign_lock(|| {
            self.complete_compress_publication_inner(campaign_id)
                .map_err(crate::turn_coordinator::CommitError::Storage)?;
            Ok(())
        })
        .map_err(|e| e.to_string())
    }

    fn complete_compress_publication_inner(&self, campaign_id: &Id) -> Result<(), String> {
        let Some(mut camp) = self.get_campaign(campaign_id) else {
            return Ok(());
        };
        let Some(pending) = camp.pending_compress_publication.clone() else {
            // 无 marker：兼容旧路径——若仍有 epoch 且已有 covered+stage，仍尝试失效 epoch
            if camp.context_epoch.is_some() {
                let summaries = self.list_summaries(campaign_id);
                let has_stage = summaries.iter().any(|s| s.level > 0);
                let has_covered = summaries.iter().any(|s| s.covered_by.is_some());
                if has_stage && has_covered {
                    camp.bump_chronicle_revision();
                    camp.context_epoch = None;
                    return self.update_campaign(camp);
                }
            }
            return Ok(());
        };

        // 完成前校验：parent 存在 + child→parent 覆盖
        if let Err(reason) = self.verify_pending_publication(campaign_id, &pending) {
            return Err(format!(
                "pending publication incomplete, keep marker: {reason}"
            ));
        }

        // 仅当 revision 尚未越过 base 时 bump，避免重复 heal 连涨
        if camp.chronicle_revision <= pending.base_chronicle_revision {
            camp.bump_chronicle_revision();
        }
        camp.context_epoch = None;
        camp.pending_compress_publication = None;
        self.update_campaign(camp)
    }

    fn verify_pending_publication(
        &self,
        campaign_id: &Id,
        pending: &storyforge_domain::chronicle::PendingCompressPublication,
    ) -> Result<(), String> {
        let summaries = self.list_summaries(campaign_id);
        for pid in &pending.parent_ids {
            if !summaries.iter().any(|s| s.id == *pid) {
                return Err(format!("missing parent {pid}"));
            }
        }
        // 新格式：child_covered_by 对
        if !pending.child_covered_by.is_empty() {
            for (child_id, parent_id) in &pending.child_covered_by {
                let Some(child) = summaries.iter().find(|s| s.id == *child_id) else {
                    return Err(format!("missing child {child_id}"));
                };
                if child.covered_by.as_ref() != Some(parent_id) {
                    return Err(format!(
                        "child {child_id} covered_by mismatch (want {parent_id})"
                    ));
                }
            }
            return Ok(());
        }
        // 旧格式兼容：仅 child_ids 存在且已有任意 covered_by
        for cid in &pending.child_ids {
            let Some(child) = summaries.iter().find(|s| s.id == *cid) else {
                return Err(format!("missing child {cid}"));
            };
            if child.covered_by.is_none() {
                return Err(format!("child {cid} not covered yet"));
            }
        }
        Ok(())
    }

    /// 需要 heal：存在 pending marker，或（兼容）epoch 未失效且已有 stage/covered。
    pub fn needs_compress_metadata_heal(&self, campaign_id: &Id) -> bool {
        if self.json_access_disabled() {
            return false;
        }
        let Some(camp) = self.get_campaign(campaign_id) else {
            return false;
        };
        if camp.pending_compress_publication.is_some() {
            return true;
        }
        if camp.context_epoch.is_none() {
            return false;
        }
        let summaries = self.list_summaries(campaign_id);
        let has_stage = summaries.iter().any(|s| s.level > 0);
        let has_covered = summaries.iter().any(|s| s.covered_by.is_some());
        has_stage && has_covered
    }

    // ─── MVU 翻译存储（P3 新增）──────────────────────────────────────────

    /// 列所有 MVU 翻译
    pub fn list_all_mvu(&self) -> Vec<StoredMvuTranslation> {
        if self.json_access_disabled() {
            return Vec::new();
        }
        self.mvu.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// 查某角色卡的 MVU 翻译
    pub fn get_mvu(&self, source_character_id: &Id) -> Option<StoredMvuTranslation> {
        if self.json_access_disabled() {
            return None;
        }
        self.mvu
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|m| m.source_character_id == *source_character_id)
            .cloned()
    }

    /// 保存/覆盖某角色卡的 MVU 翻译（按 source_character_id 去重）
    pub fn save_mvu(&self, stored: StoredMvuTranslation) -> Result<(), String> {
        self.ensure_json_write_allowed()?;
        let mut mvu = self.mvu.lock().unwrap_or_else(|p| p.into_inner());
        mvu.retain(|m| m.source_character_id != stored.source_character_id);
        mvu.push(stored);
        persist(&self.mvu_path, &mvu)
    }

    /// 删某角色卡的 MVU 翻译（删卡时级联）
    pub fn delete_mvu(&self, source_character_id: &Id) -> Result<bool, String> {
        self.ensure_json_write_allowed()?;
        let mut mvu = self.mvu.lock().unwrap_or_else(|p| p.into_inner());
        let before = mvu.len();
        mvu.retain(|m| m.source_character_id != *source_character_id);
        let changed = mvu.len() != before;
        if changed {
            persist(&self.mvu_path, &mvu)?;
        }
        Ok(changed)
    }
}

// ─── 持久化辅助 ─────────────────────────────────────────────────────────────

fn load_or_default<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    crate::storage::json_store::load_json_with_tmp_backup_or_default(
        path,
        |error| {
            tracing::warn!(
                "Failed to load {}; trying .tmp backup: {error}",
                path.display()
            );
        },
        |path, error| {
            tracing::error!(
                "Failed to load {} and .tmp recovery was unavailable; copied .corrupt backup: {error}",
                path.display()
            );
        },
    )
}

pub(crate) fn persist<T: serde::Serialize>(path: &Path, data: &[T]) -> Result<(), String> {
    storyforge_infra_util::atomic_write_json(path, data).map_err(|e| {
        let msg = format!("持久化失败 {}: {e}", path.display());
        tracing::error!("{msg}");
        msg
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathSnapshot {
    Missing,
    RegularFile,
    Other,
}

fn snapshot_path(path: &Path) -> Result<PathSnapshot, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(PathSnapshot::RegularFile),
        Ok(_) => Ok(PathSnapshot::Other),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PathSnapshot::Missing),
        Err(error) => Err(format!("读取路径状态失败 {}: {error}", path.display())),
    }
}

fn ensure_regular_or_missing(path: &Path, snapshot: PathSnapshot) -> Result<(), String> {
    if snapshot == PathSnapshot::Other {
        return Err(format!("路径不是普通文件，拒绝删除: {}", path.display()));
    }
    Ok(())
}

fn restore_json_file<T: serde::Serialize>(
    path: &Path,
    snapshot: PathSnapshot,
    data: &[T],
) -> Result<(), String> {
    match snapshot {
        PathSnapshot::RegularFile => persist(path, data),
        PathSnapshot::Missing => remove_created_file_if_present(path),
        PathSnapshot::Other => Err(format!(
            "拒绝回滚原本为非普通文件的路径: {}",
            path.display()
        )),
    }
}

fn restore_world_info_file(
    path: &Path,
    snapshot: PathSnapshot,
    raw: Option<&str>,
) -> Result<(), String> {
    match (snapshot, raw) {
        (PathSnapshot::RegularFile, Some(raw)) => {
            storyforge_infra_util::atomic_write_json_str(path, raw)
                .map_err(|error| format!("恢复本局世界书失败 {}: {error}", path.display()))
        }
        (PathSnapshot::Missing, None) => remove_created_file_if_present(path),
        (PathSnapshot::Other, _) => Err(format!(
            "拒绝回滚原本为非普通文件的路径: {}",
            path.display()
        )),
        _ => Err(format!("本局世界书原始快照不一致: {}", path.display())),
    }
}

/// Roll back only a regular file created by this operation. Never recurse into
/// a directory or follow a symlink that appeared after the snapshot.
fn remove_created_file_if_present(path: &Path) -> Result<(), String> {
    match snapshot_path(path)? {
        PathSnapshot::Missing => Ok(()),
        PathSnapshot::RegularFile => std::fs::remove_file(path)
            .map_err(|error| format!("删除回滚文件失败 {}: {error}", path.display())),
        PathSnapshot::Other => Err(format!("回滚拒绝删除非普通文件路径: {}", path.display())),
    }
}

fn empty_world_info_book() -> WorldInfoBook {
    WorldInfoBook {
        entries: Vec::new(),
        source: storyforge_domain::Source::Native,
        metadata: Default::default(),
    }
}

// ─── 测试 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::character::{
        CharacterCard, CharacterDefinition, CharacterExtractionStatus, RoleType,
    };
    use storyforge_domain::variables::default_character_variables;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sf-campaign-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_card() -> CharacterCard {
        let mut card = CharacterCard {
            id: Id::from_str("card-1"),
            name: "测试卡".into(),
            source_character_id: Id::from_str("src-1"),
            character_definitions: vec![],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::Value::Null,
            extraction_status: CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        let def = CharacterDefinition {
            id: Id::from_str("def-1"),
            card_id: card.id.clone(),
            name: "林医生".into(),
            persona_prompt: "外科医生".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        card.character_definitions.push(def);
        card
    }

    #[test]
    fn test_campaigns_loads_tmp_backup_when_main_json_invalid() {
        let dir = temp_dir();
        let path = dir.join("campaigns.json");
        let campaign = Campaign::new(Id::from_str("card-1"), "tmp recovery".to_string());
        std::fs::write(&path, "{ invalid").unwrap();
        std::fs::write(
            path.with_extension("json.tmp"),
            serde_json::to_string(&vec![campaign.clone()]).unwrap(),
        )
        .unwrap();

        let store = CampaignStore::new(&dir);
        let loaded = store.list_campaigns();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, campaign.id);
        assert_eq!(loaded[0].card_id, campaign.card_id);
        assert!(!path.with_extension("json.corrupt").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_campaigns_copies_corrupt_backup_when_main_and_tmp_are_invalid() {
        let dir = temp_dir();
        let path = dir.join("campaigns.json");
        let original = "{ invalid";
        std::fs::write(&path, original).unwrap();
        std::fs::write(path.with_extension("json.tmp"), "{ also invalid").unwrap();

        let store = CampaignStore::new(&dir);

        assert!(store.list_campaigns().is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.corrupt")).unwrap(),
            original
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_card_save_get_list() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let card = make_card();
        let stored = store.save_card(card.clone()).unwrap();
        assert!(!stored.imported_at.is_empty());

        let got = store.get_card(&Id::from_str("card-1")).unwrap();
        assert_eq!(got.card.name, "测试卡");
        assert_eq!(got.card.character_definitions.len(), 1);

        let all = store.list_cards();
        assert_eq!(all.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_campaign_world_info_copy_edit_does_not_touch_missing_file_until_set() {
        use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};

        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        store.save_card(make_card()).unwrap();
        let camp = Campaign::new(Id::from_str("card-1"), "wi-play");
        let camp_id = camp.id.clone();
        store.create_campaign_with_instances(camp).unwrap();

        let template = WorldInfoBook {
            entries: vec![WorldInfoEntry {
                st_id: Some(1),
                keys: vec!["圣都".into()],
                secondary_keys: vec![],
                content: "梵尼亚".into(),
                constant: true,
                selective: false,
                selective_logic: Default::default(),
                disabled: false,
                position: 0,
                depth: 2,
                order: 100,
                route: LoreRoute::Constant,
                extensions: serde_json::json!({}),
                extra: Default::default(),
            }],
            source: storyforge_domain::Source::ImportedFromST,
            metadata: Default::default(),
        };
        let book = store
            .ensure_world_info_from_book(&camp_id, &template)
            .unwrap();
        assert_eq!(book.entries.len(), 1);
        assert_eq!(
            book.entries[0]
                .extensions
                .get("sf_source")
                .and_then(|v| v.as_str()),
            Some("card")
        );

        // 二次 ensure 不覆盖用户已有书
        let mut user_book = book.clone();
        user_book.entries[0].content = "用户改过".into();
        store.set_world_info(&camp_id, user_book).unwrap();
        let again = store
            .ensure_world_info_from_book(&camp_id, &template)
            .unwrap();
        assert_eq!(again.entries[0].content, "用户改过");

        store
            .add_world_info_entry(
                &camp_id,
                WorldInfoEntry {
                    st_id: None,
                    keys: vec!["测试地点".into()],
                    secondary_keys: vec![],
                    content: "本局新增".into(),
                    constant: false,
                    selective: true,
                    selective_logic: Default::default(),
                    disabled: false,
                    position: 0,
                    depth: 2,
                    order: 50,
                    route: LoreRoute::Selective,
                    extensions: serde_json::json!({}),
                    extra: Default::default(),
                },
            )
            .unwrap();
        let listed = store.get_world_info(&camp_id).unwrap();
        assert_eq!(listed.entries.len(), 2);
        assert!(store.world_info_path(&camp_id).exists());

        assert!(store.delete_campaign(&camp_id).unwrap());
        assert!(!store.world_info_path(&camp_id).exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_campaign_rolls_back_all_json_files_when_a_cascade_write_fails() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let campaign = Campaign::new(Id::from_str("rollback-campaign"), "rollback");
        let campaign_id = campaign.id.clone();
        store.save_campaign(campaign.clone()).unwrap();
        let instance = CharacterInstance::temporary(campaign_id.clone(), "Lin");
        store.add_instance(instance.clone()).unwrap();

        // A pre-existing directory at the target path must be treated as an
        // unsupported path, never as a missing file that rollback may remove.
        std::fs::create_dir(&store.knowledge_path).unwrap();
        let sentinel = store.knowledge_path.join("must-survive.txt");
        std::fs::write(&sentinel, b"pre-existing directory").unwrap();
        let error = store
            .delete_campaign(&campaign_id)
            .expect_err("cascade write failure should be surfaced");
        assert!(error.contains("不是普通文件"));
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"pre-existing directory");
        assert!(store.get_campaign(&campaign_id).is_some());
        assert_eq!(store.list_instances(&campaign_id).len(), 1);

        let reloaded = CampaignStore::new(&dir);
        assert!(reloaded.get_campaign(&campaign_id).is_some());
        assert_eq!(reloaded.list_instances(&campaign_id).len(), 1);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn concurrent_world_info_enabled_updates_keep_both_entry_changes() {
        use std::sync::{Arc, Barrier};
        use std::thread;
        use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};

        let dir = temp_dir();
        let store = Arc::new(CampaignStore::new(&dir));
        store.save_card(make_card()).unwrap();
        let campaign = Campaign::new(Id::from_str("card-1"), "world-info-atomic");
        let campaign_id = campaign.id.clone();
        store.create_campaign_with_instances(campaign).unwrap();

        let entry = WorldInfoEntry {
            st_id: None,
            keys: vec!["atomic".into()],
            secondary_keys: vec![],
            content: "atomic lore".into(),
            constant: false,
            selective: true,
            selective_logic: Default::default(),
            disabled: false,
            position: 0,
            depth: 2,
            order: 100,
            route: LoreRoute::Selective,
            extensions: serde_json::json!({}),
            extra: Default::default(),
        };
        store
            .set_world_info(
                &campaign_id,
                WorldInfoBook {
                    entries: vec![entry.clone(), entry],
                    source: storyforge_domain::Source::Native,
                    metadata: Default::default(),
                },
            )
            .unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let first_store = Arc::clone(&store);
        let first_id = campaign_id.clone();
        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            first_barrier.wait();
            first_store
                .set_world_info_entry_enabled(&first_id, 0, false)
                .unwrap();
        });
        let second_store = Arc::clone(&store);
        let second_id = campaign_id.clone();
        let second_barrier = Arc::clone(&barrier);
        let second = thread::spawn(move || {
            second_barrier.wait();
            second_store
                .set_world_info_entry_enabled(&second_id, 1, false)
                .unwrap();
        });
        first.join().unwrap();
        second.join().unwrap();

        let persisted = store.get_world_info(&campaign_id).unwrap();
        assert!(persisted.entries.iter().all(|entry| entry.disabled));

        drop(store);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn campaign_delete_serializes_with_world_info_write_and_leaves_no_orphan() {
        use std::sync::{Arc, Barrier};
        use std::thread;
        use storyforge_domain::world_info::{LoreRoute, WorldInfoEntry};

        let dir = temp_dir();
        let store = Arc::new(CampaignStore::new(&dir));
        store.save_card(make_card()).unwrap();
        let campaign = Campaign::new(Id::from_str("card-1"), "world-info-delete-race");
        let campaign_id = campaign.id.clone();
        store.create_campaign_with_instances(campaign).unwrap();

        let write_entered = Arc::new(Barrier::new(2));
        let release_write = Arc::new(Barrier::new(2));
        let writer_store = Arc::clone(&store);
        let writer_id = campaign_id.clone();
        let writer_entered = Arc::clone(&write_entered);
        let writer_release = Arc::clone(&release_write);
        let writer = thread::spawn(move || {
            writer_store
                .mutate_world_info(&writer_id, |book| {
                    writer_entered.wait();
                    writer_release.wait();
                    book.entries.push(WorldInfoEntry {
                        st_id: None,
                        keys: vec!["race".into()],
                        secondary_keys: vec![],
                        content: "must not survive deletion".into(),
                        constant: false,
                        selective: true,
                        selective_logic: Default::default(),
                        disabled: false,
                        position: 0,
                        depth: 2,
                        order: 100,
                        route: LoreRoute::Selective,
                        extensions: serde_json::json!({}),
                        extra: Default::default(),
                    });
                    Ok(())
                })
                .unwrap();
        });

        write_entered.wait();
        let delete_store = Arc::clone(&store);
        let delete_id = campaign_id.clone();
        let delete = thread::spawn(move || delete_store.delete_campaign(&delete_id));
        release_write.wait();

        writer.join().unwrap();
        assert!(delete.join().unwrap().unwrap());
        assert!(!store.world_info_path(&campaign_id).exists());
        assert!(
            store
                .get_world_info(&campaign_id)
                .unwrap()
                .entries
                .is_empty()
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_card_save_dedup_by_source() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let mut card = make_card();
        store.save_card(card.clone()).unwrap();
        // 重跑（同名新 id），应覆盖而非累积
        card.id = Id::from_str("card-2");
        store.save_card(card).unwrap();
        assert_eq!(store.list_cards().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disabled_store_never_exposes_or_writes_legacy_json_data() {
        let store = CampaignStore::disabled();
        let campaign = Campaign::new(Id::from_str("card-1"), "sqlite authority".to_string());

        assert!(store.list_campaigns().is_empty());
        assert!(store.get_campaign(&campaign.id).is_none());
        assert!(store.save_campaign(campaign).is_err());
        assert!(store.data_dir().is_none());
    }

    #[test]
    fn test_campaign_and_instances_cascade_delete() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);

        // 建 card
        store.save_card(make_card()).unwrap();

        // 建 campaign + instances
        let camp_id = Id::from_str("camp-1");
        let camp = Campaign::new(Id::from_str("card-1"), "新档".to_string());
        let _ = camp_id;
        store.save_campaign(camp.clone()).unwrap();

        let inst = CharacterInstance::from_definition(
            camp.id.clone(),
            &make_card().character_definitions[0],
        );
        store.add_instance(inst.clone()).unwrap();

        assert_eq!(store.list_campaigns().len(), 1);
        assert_eq!(store.list_instances(&camp.id).len(), 1);

        // 删 card 级联删 campaign + instances
        assert!(store.delete_card(&Id::from_str("card-1")).unwrap());
        assert_eq!(store.list_campaigns().len(), 0);
        assert_eq!(store.list_all_instances().len(), 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_guarded_card_save_refuses_campaign_on_replaced_source() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let mut card = make_card();
        card.character_definitions[0].id = Id::from_str("old-def");
        store.save_card(card.clone()).unwrap();

        let camp = Campaign::new(card.id.clone(), "guarded");
        store.save_campaign(camp).unwrap();

        let mut rerun_card = card.clone();
        rerun_card.id = Id::from_str("new-card-id");
        rerun_card.character_definitions[0].id = Id::from_str("new-def");

        let err = store.save_card_if_no_campaigns(rerun_card).unwrap_err();
        assert_eq!(err, FORCE_RERUN_BLOCKED_BY_CAMPAIGN);

        let stored = store.get_card(&card.id).unwrap();
        assert_eq!(
            stored.card.character_definitions[0].id,
            Id::from_str("old-def")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_create_campaign_with_instances_uses_current_card_definitions() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let mut card = make_card();
        card.character_definitions[0].id = Id::from_str("old-def");
        store.save_card(card.clone()).unwrap();

        let mut current_card = card.clone();
        current_card.character_definitions[0].id = Id::from_str("current-def");
        current_card.character_definitions[0].name = "Current".into();
        store.save_card_if_no_campaigns(current_card).unwrap();

        let campaign = Campaign::new(card.id.clone(), "current definitions");
        let (_, campaign, instance_count) = store.create_campaign_with_instances(campaign).unwrap();

        assert_eq!(instance_count, 1);
        let instances = store.list_instances(&campaign.id);
        assert_eq!(instances.len(), 1);
        assert_eq!(
            instances[0].definition_id.as_ref(),
            Some(&Id::from_str("current-def"))
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_create_campaign_with_instances_does_not_commit_memory_on_instance_persist_failure() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        store.save_card(make_card()).unwrap();
        std::fs::create_dir_all(&store.instances_path).unwrap();

        let campaign = Campaign::new(Id::from_str("card-1"), "blocked instances");
        let err = store.create_campaign_with_instances(campaign).unwrap_err();

        assert!(
            err.contains("instances.json"),
            "expected instances persist failure, got {err}"
        );
        assert!(store.list_campaigns().is_empty());
        assert!(store.list_all_instances().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_persistence_reload() {
        let dir = temp_dir();
        {
            let store = CampaignStore::new(&dir);
            store.save_card(make_card()).unwrap();
        }
        // 新 store 实例从同一目录加载
        let store2 = CampaignStore::new(&dir);
        assert_eq!(store2.list_cards().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_update_instance() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let camp = Campaign::new(Id::from_str("card-1"), "x".to_string());
        let mut inst = CharacterInstance::from_definition(
            camp.id.clone(),
            &make_card().character_definitions[0],
        );
        store.save_campaign(camp.clone()).unwrap();
        store.add_instance(inst.clone()).unwrap();

        inst.set_variable("hp", serde_json::json!(50), 3);
        store.update_instance(inst.clone()).unwrap();

        let got = store.get_instance(&inst.campaign_id, &inst.id).unwrap();
        let hp = got.get_variable("hp").unwrap();
        assert_eq!(hp.as_i64(), Some(50));

        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── P2 新增：knowledge / tasks / summaries 持久化测试 ──────────────────

    #[test]
    fn test_knowledge_add_and_query() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let camp_id = Id::from_str("camp-k");
        let char_id = Id::from_str("char-1");
        let e1 =
            CharacterKnowledgeEntry::witnessed(camp_id.clone(), char_id.clone(), "看到尸体", 1);
        let e2 =
            CharacterKnowledgeEntry::backstory(camp_id.clone(), char_id.clone(), "我是外科医生");
        store.add_knowledge(vec![e1.clone(), e2.clone()]).unwrap();

        assert_eq!(store.list_knowledge(&camp_id).len(), 2);
        assert_eq!(store.list_knowledge_of(&camp_id, &char_id).len(), 2);
        // 另一角色查不到
        assert!(
            store
                .list_knowledge_of(&camp_id, &Id::from_str("char-other"))
                .is_empty()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_delete_knowledge() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let camp_id = Id::from_str("camp-dk");
        let char_id = Id::from_str("char-1");
        let e1 =
            CharacterKnowledgeEntry::witnessed(camp_id.clone(), char_id.clone(), "看到尸体", 1);
        let e2 =
            CharacterKnowledgeEntry::backstory(camp_id.clone(), char_id.clone(), "我是外科医生");
        let e1_id = e1.id.clone();
        store.add_knowledge(vec![e1, e2]).unwrap();
        assert_eq!(store.list_knowledge(&camp_id).len(), 2);

        // 删除 e1
        assert!(store.delete_knowledge(&e1_id).unwrap());
        assert_eq!(store.list_knowledge(&camp_id).len(), 1);

        // 再删返回 false
        assert!(!store.delete_knowledge(&e1_id).unwrap());

        // 持久化验证
        let store2 = CampaignStore::new(&dir);
        assert_eq!(store2.list_knowledge(&camp_id).len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_task_crud() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let camp_id = Id::from_str("camp-t");
        let task = StoryTask::user_planned(
            camp_id.clone(),
            "复仇",
            "老王复仇",
            vec![storyforge_domain::story_task::TaskTrigger::TurnReminder { at_turn: 10 }],
            1,
        );
        let task_id = task.id.clone();
        store.add_task(task).unwrap();
        assert_eq!(store.list_tasks(&camp_id).len(), 1);

        // 更新：标完成
        let mut got = store.get_task(&task_id).unwrap();
        got.complete();
        store.update_task(got).unwrap();
        assert_eq!(
            store.get_task(&task_id).unwrap().status,
            storyforge_domain::story_task::TaskStatus::Completed
        );

        // 删除
        assert!(store.delete_task(&task_id).unwrap());
        assert!(store.get_task(&task_id).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_summary_add_dedup_by_turn_and_sorted() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let camp_id = Id::from_str("camp-s");
        let conv = Id::from_str("conv-1");

        store
            .add_summary(RoundSummary::new(
                camp_id.clone(),
                conv.clone(),
                2,
                "第二轮".into(),
            ))
            .unwrap();
        store
            .add_summary(RoundSummary::new(
                camp_id.clone(),
                conv.clone(),
                1,
                "第一轮".into(),
            ))
            .unwrap();
        // 同 turn 覆盖
        store
            .add_summary(RoundSummary::new(
                camp_id.clone(),
                conv.clone(),
                1,
                "第一轮（重写）".into(),
            ))
            .unwrap();

        let list = store.list_summaries(&camp_id);
        assert_eq!(list.len(), 2); // turn 1 和 turn 2
        assert_eq!(list[0].turn, 1); // 升序
        assert_eq!(list[1].turn, 2);
        assert_eq!(list[0].content, "第一轮（重写）"); // 覆盖生效
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_campaign_delete_cascades_to_p2_collections() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        store.save_card(make_card()).unwrap(); // 保证 card-1 存在

        // 建 campaign（Campaign::new 内部分配 id）
        let camp = Campaign::new(Id::from_str("card-1"), "cascade");
        let camp_id = camp.id.clone();
        store.save_campaign(camp).unwrap();

        // 塞三类 P2 数据
        store
            .add_knowledge(vec![CharacterKnowledgeEntry::witnessed(
                camp_id.clone(),
                Id::from_str("c1"),
                "x",
                1,
            )])
            .unwrap();
        store
            .add_task(StoryTask::user_planned(
                camp_id.clone(),
                "t",
                "d",
                vec![],
                1,
            ))
            .unwrap();
        store
            .add_summary(RoundSummary::new(
                camp_id.clone(),
                Id::from_str("conv"),
                1,
                "s".into(),
            ))
            .unwrap();

        assert_eq!(store.list_knowledge(&camp_id).len(), 1);
        assert_eq!(store.list_tasks(&camp_id).len(), 1);
        assert_eq!(store.list_summaries(&camp_id).len(), 1);

        // 删 campaign 级联清掉三类
        assert!(store.delete_campaign(&camp_id).unwrap());
        assert!(store.list_knowledge(&camp_id).is_empty());
        assert!(store.list_tasks(&camp_id).is_empty());
        assert!(store.list_summaries(&camp_id).is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_parallel_collection_writes_round_trip() {
        let dir = temp_dir();
        let store = std::sync::Arc::new(CampaignStore::new(&dir));
        store.save_card(make_card()).unwrap();
        let camp = Campaign::new(Id::from_str("card-1"), "parallel");
        let camp_id = camp.id.clone();
        store.save_campaign(camp).unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(5));
        let mut handles = Vec::new();

        {
            let store = store.clone();
            let barrier = barrier.clone();
            let camp_id = camp_id.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..20 {
                    store
                        .add_knowledge(vec![CharacterKnowledgeEntry::witnessed(
                            camp_id.clone(),
                            Id::from_str("inst-parallel"),
                            format!("knowledge-{i}"),
                            i + 1,
                        )])
                        .unwrap();
                }
            }));
        }

        {
            let store = store.clone();
            let barrier = barrier.clone();
            let camp_id = camp_id.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..20 {
                    store
                        .add_task(StoryTask::user_planned(
                            camp_id.clone(),
                            format!("task-{i}"),
                            "parallel task",
                            vec![],
                            i + 1,
                        ))
                        .unwrap();
                }
            }));
        }

        {
            let store = store.clone();
            let barrier = barrier.clone();
            let camp_id = camp_id.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..20 {
                    store
                        .add_summary(RoundSummary::new(
                            camp_id.clone(),
                            Id::from_str("conv-parallel"),
                            i + 1,
                            format!("summary-{i}"),
                        ))
                        .unwrap();
                }
            }));
        }

        {
            let store = store.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..20 {
                    store
                        .save_mvu(make_mvu(&format!("src-parallel-{i}"), "parallel"))
                        .unwrap();
                }
            }));
        }

        {
            let store = store.clone();
            let barrier = barrier.clone();
            let camp_id = camp_id.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..20 {
                    let _ = store.list_cards();
                    let _ = store.list_campaigns();
                    let _ = store.list_knowledge(&camp_id);
                    let _ = store.list_tasks(&camp_id);
                    let _ = store.list_summaries(&camp_id);
                    let _ = store.list_all_mvu();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let reloaded = CampaignStore::new(&dir);
        assert_eq!(reloaded.list_knowledge(&camp_id).len(), 20);
        assert_eq!(reloaded.list_tasks(&camp_id).len(), 20);
        assert_eq!(reloaded.list_summaries(&camp_id).len(), 20);
        assert_eq!(reloaded.list_all_mvu().len(), 20);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[ignore = "发布前手动压测同步 JSON I/O：cargo test -p storyforge --lib pressure_sync_json_io -- --ignored --nocapture"]
    fn pressure_sync_json_io_across_collections() {
        let writes_per_collection = std::env::var("SF_STORE_PRESSURE_WRITES")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(120);

        let dir = temp_dir();
        let store = std::sync::Arc::new(CampaignStore::new(&dir));
        store.save_card(make_card()).unwrap();
        let camp = Campaign::new(Id::from_str("card-1"), "pressure");
        let camp_id = camp.id.clone();
        store.save_campaign(camp).unwrap();

        let timings: std::sync::Arc<std::sync::Mutex<Vec<(&'static str, u128)>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let total_start = std::time::Instant::now();
        let mut handles = Vec::new();

        {
            let store = store.clone();
            let timings = timings.clone();
            let barrier = barrier.clone();
            let camp_id = camp_id.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..writes_per_collection {
                    let start = std::time::Instant::now();
                    store
                        .add_knowledge(vec![CharacterKnowledgeEntry::witnessed(
                            camp_id.clone(),
                            Id::from_str("inst-pressure"),
                            format!("pressure knowledge {i}"),
                            i + 1,
                        )])
                        .unwrap();
                    record_store_timing(&timings, "knowledge", start.elapsed());
                }
            }));
        }

        {
            let store = store.clone();
            let timings = timings.clone();
            let barrier = barrier.clone();
            let camp_id = camp_id.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..writes_per_collection {
                    let start = std::time::Instant::now();
                    store
                        .add_task(StoryTask::user_planned(
                            camp_id.clone(),
                            format!("pressure task {i}"),
                            "pressure task",
                            vec![],
                            i + 1,
                        ))
                        .unwrap();
                    record_store_timing(&timings, "tasks", start.elapsed());
                }
            }));
        }

        {
            let store = store.clone();
            let timings = timings.clone();
            let barrier = barrier.clone();
            let camp_id = camp_id.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..writes_per_collection {
                    let start = std::time::Instant::now();
                    store
                        .add_summary(RoundSummary::new(
                            camp_id.clone(),
                            Id::from_str("conv-pressure"),
                            i + 1,
                            format!("pressure summary {i}"),
                        ))
                        .unwrap();
                    record_store_timing(&timings, "summaries", start.elapsed());
                }
            }));
        }

        {
            let store = store.clone();
            let timings = timings.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..writes_per_collection {
                    let start = std::time::Instant::now();
                    store
                        .save_mvu(make_mvu(&format!("src-pressure-{i}"), "pressure"))
                        .unwrap();
                    record_store_timing(&timings, "mvu", start.elapsed());
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let elapsed_ms = total_start.elapsed().as_millis();
        let reloaded = CampaignStore::new(&dir);
        assert_eq!(
            reloaded.list_knowledge(&camp_id).len(),
            writes_per_collection as usize
        );
        assert_eq!(
            reloaded.list_tasks(&camp_id).len(),
            writes_per_collection as usize
        );
        assert_eq!(
            reloaded.list_summaries(&camp_id).len(),
            writes_per_collection as usize
        );
        assert_eq!(
            reloaded.list_all_mvu().len(),
            writes_per_collection as usize
        );

        let timings = timings.lock().unwrap_or_else(|p| p.into_inner());
        eprintln!(
            "CampaignStore pressure: writes_per_collection={writes_per_collection}, total_elapsed_ms={elapsed_ms}"
        );
        for label in ["knowledge", "tasks", "summaries", "mvu"] {
            let samples: Vec<u128> = timings
                .iter()
                .filter_map(|(sample_label, micros)| (*sample_label == label).then_some(*micros))
                .collect();
            eprintln!("{}", format_store_timing(label, &samples));
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    fn record_store_timing(
        timings: &std::sync::Arc<std::sync::Mutex<Vec<(&'static str, u128)>>>,
        label: &'static str,
        elapsed: std::time::Duration,
    ) {
        timings
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((label, elapsed.as_micros()));
    }

    fn format_store_timing(label: &str, samples: &[u128]) -> String {
        if samples.is_empty() {
            return format!("{label}: no samples");
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let p50 = sorted[sorted.len() / 2];
        let p95 = sorted[(sorted.len() - 1) * 95 / 100];
        let max = sorted[sorted.len() - 1];
        format!(
            "{label}: samples={} p50={}us p95={}us max={}us",
            sorted.len(),
            p50,
            p95,
            max
        )
    }

    fn make_mvu(source_id: &str, name: &str) -> StoredMvuTranslation {
        StoredMvuTranslation {
            source_character_id: Id::from_str(source_id),
            character_name: name.into(),
            translation: storyforge_domain::mvu_translation::MvuTranslation::pure_data_fallback(
                vec![],
            ),
            analyzed_at: "2026-06-16T00:00:00Z".into(),
        }
    }

    #[test]
    fn test_mvu_save_get_list_delete() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);

        // 空
        assert!(store.list_all_mvu().is_empty());
        assert!(store.get_mvu(&Id::from_str("src-1")).is_none());

        // 存
        store.save_mvu(make_mvu("src-1", "测试卡A")).unwrap();
        store.save_mvu(make_mvu("src-2", "测试卡B")).unwrap();
        assert_eq!(store.list_all_mvu().len(), 2);
        assert!(store.get_mvu(&Id::from_str("src-1")).is_some());
        assert_eq!(
            store
                .get_mvu(&Id::from_str("src-1"))
                .unwrap()
                .character_name,
            "测试卡A"
        );

        // 覆盖（同 source_character_id 去重）
        store.save_mvu(make_mvu("src-1", "测试卡A-改")).unwrap();
        assert_eq!(store.list_all_mvu().len(), 2);
        assert_eq!(
            store
                .get_mvu(&Id::from_str("src-1"))
                .unwrap()
                .character_name,
            "测试卡A-改"
        );

        // 删
        assert!(store.delete_mvu(&Id::from_str("src-1")).unwrap());
        assert_eq!(store.list_all_mvu().len(), 1);
        assert!(store.get_mvu(&Id::from_str("src-1")).is_none());
        // 再删返回 false
        assert!(!store.delete_mvu(&Id::from_str("src-1")).unwrap());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_mvu_cascade_delete_on_card_delete() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        store.save_card(make_card()).unwrap(); // card-1, source src-1
        store.save_mvu(make_mvu("src-1", "测试卡")).unwrap();

        assert!(store.get_mvu(&Id::from_str("src-1")).is_some());

        // 删卡 → MVU 级联清掉
        assert!(store.delete_card(&Id::from_str("card-1")).unwrap());
        assert!(store.get_mvu(&Id::from_str("src-1")).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── Phase A: 三态 upsert 测试 ────────────────────────────────────────

    fn make_knowledge_entry(id: &str, text: &str) -> CharacterKnowledgeEntry {
        CharacterKnowledgeEntry {
            id: Id::from_str(id),
            campaign_id: Id::from_str("camp-1"),
            character_id: Id::from_str("char-1"),
            knowledge_text: text.into(),
            source: storyforge_domain::character_knowledge::KnowledgeSource::Witnessed,
            source_character_id: None,
            source_knowledge_id: None,
            turn_number: 1,
            event_id: None,
            pinned: false,
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        }
    }

    #[test]
    fn upsert_knowledge_inserts_new() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let entry = make_knowledge_entry("k-1", "看到了刀");
        let result = store.upsert_knowledge(entry).unwrap();
        assert_eq!(result, UpsertResult::Inserted);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_knowledge_noop_on_identical_payload() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let entry = make_knowledge_entry("k-1", "看到了刀");
        store.upsert_knowledge(entry.clone()).unwrap();
        // 重放同一 entry → AlreadyPresent
        let result = store.upsert_knowledge(entry).unwrap();
        assert_eq!(result, UpsertResult::AlreadyPresent);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_knowledge_conflict_on_different_payload() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        store
            .upsert_knowledge(make_knowledge_entry("k-1", "看到了刀"))
            .unwrap();
        // 同 ID 不同 text → Conflict
        let result = store
            .upsert_knowledge(make_knowledge_entry("k-1", "看到了枪"))
            .unwrap();
        assert!(matches!(result, UpsertResult::Conflict(_)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_task_inserts_and_idempotent() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let task = StoryTask::from_narrative(
            Id::from_str("camp-1"),
            "寻找密室".to_string(),
            "在书房找到暗门".to_string(),
            vec![],
            1,
        );
        let task_id = task.id.clone();

        let result1 = store.upsert_task(task.clone()).unwrap();
        assert_eq!(result1, UpsertResult::Inserted);

        // 重放 → AlreadyPresent
        let result2 = store.upsert_task(task).unwrap();
        assert_eq!(result2, UpsertResult::AlreadyPresent);

        // 确认只有一条
        assert_eq!(store.list_tasks(&Id::from_str("camp-1")).len(), 1);
        let _ = task_id;
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_summary_inserts_and_idempotent_by_campaign_turn() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let summary = RoundSummary::new(
            Id::from_str("camp-1"),
            Id::from_str("conv-1"),
            1,
            "第一轮摘要".into(),
        );

        let result1 = store.upsert_summary(summary.clone()).unwrap();
        assert_eq!(result1, UpsertResult::Inserted);

        // 重放同一 summary → AlreadyPresent
        let result2 = store.upsert_summary(summary).unwrap();
        assert_eq!(result2, UpsertResult::AlreadyPresent);

        assert_eq!(store.list_summaries(&Id::from_str("camp-1")).len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_summary_conflict_on_different_content() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        store
            .upsert_summary(RoundSummary::new(
                Id::from_str("camp-1"),
                Id::from_str("conv-1"),
                1,
                "第一轮摘要".into(),
            ))
            .unwrap();

        // 同 campaign+turn 但不同 content → Conflict
        let result = store
            .upsert_summary(RoundSummary::new(
                Id::from_str("camp-1"),
                Id::from_str("conv-1"),
                1,
                "改过的摘要".into(),
            ))
            .unwrap();
        assert!(matches!(result, UpsertResult::Conflict(_)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn publish_compress_result_marks_covered_and_bumps_revision() {
        use storyforge_domain::agent::RoundSummary;
        use storyforge_domain::campaign::Campaign;
        use storyforge_domain::chronicle::ChronicleLevel;

        let dir =
            std::env::temp_dir().join(format!("storyforge-compress-pub-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = CampaignStore::new(&dir);
        let camp = Campaign::new(Id::from_str("card"), "c");
        let camp_id = camp.id.clone();
        store.save_campaign(camp).unwrap();

        let mut leaves = Vec::new();
        for t in 1..=4u32 {
            let s = RoundSummary::new(camp_id.clone(), Id::from_str("v"), t, format!("body{t}"))
                .with_code(format!("A{t:04}"))
                .with_headline(format!("h{t}"))
                .with_lineage(Id::from_str("lin"));
            leaves.push(s.clone());
            store.add_summary(s).unwrap();
        }
        let ids: Vec<_> = leaves.iter().map(|s| s.id.clone()).collect();
        let spans: Vec<(u32, u32)> = leaves.iter().map(|s| (s.turn, s.turn)).collect();
        let groups =
            storyforge_domain::chronicle::partition_compress_groups(&ids, &spans, 2).unwrap();
        let texts = vec![
            storyforge_domain::chronicle::CompressGroupText {
                headline: "B1".into(),
                summary: "sum1".into(),
            },
            storyforge_domain::chronicle::CompressGroupText {
                headline: "B2".into(),
                summary: "sum2".into(),
            },
        ];
        let pubr = storyforge_domain::chronicle::publish_compress_batch(
            &camp_id,
            &Id::from_str("lin"),
            &ids,
            &spans,
            &groups,
            &texts,
            ChronicleLevel::B,
            1,
        )
        .unwrap();
        let parents: Vec<_> = pubr
            .parents
            .iter()
            .map(|e| RoundSummary::from_chronicle_entry(e, Id::from_str("v")))
            .collect();
        let rev0 = store.get_campaign(&camp_id).unwrap().chronicle_revision;
        store
            .publish_compress_result(&camp_id, &parents, &pubr.child_covered_by)
            .unwrap();
        let all = store.list_summaries(&camp_id);
        assert_eq!(all.len(), 6, "4 leaves + 2 B");
        let covered = all
            .iter()
            .filter(|s| s.covered_by.is_some() && s.level == 0)
            .count();
        assert_eq!(covered, 4);
        let b_count = all.iter().filter(|s| s.level == 1).count();
        assert_eq!(b_count, 2);
        let camp2 = store.get_campaign(&camp_id).unwrap();
        assert!(camp2.chronicle_revision > rev0);
        assert!(camp2.context_epoch.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn heal_completes_pending_marker_without_epoch() {
        use storyforge_domain::chronicle::PendingCompressPublication;
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let camp_id = Id::from_str("camp-heal");
        let mut camp = Campaign::new(Id::from_str("card"), "n");
        camp.id = camp_id.clone();
        camp.chronicle_revision = 3;
        camp.context_epoch = None; // 模拟第一次 publish 后 epoch 已清，但第二次半提交
        store.save_campaign(camp.clone()).unwrap();

        // 先落盘 parent/child，再挂 marker（模拟 summaries 已写、metadata 未完成）
        let mut parent =
            RoundSummary::new(camp_id.clone(), Id::from_str("v"), 1, "B".into()).with_code("B0001");
        parent.id = Id::from_str("p1");
        parent.level = 1;
        parent.turn_end = 1;
        let mut child =
            RoundSummary::new(camp_id.clone(), Id::from_str("v"), 1, "A".into()).with_code("A0001");
        child.id = Id::from_str("c1");
        child.covered_by = Some(Id::from_str("p1"));
        // insert_stage_summary 按 id 去重，不会按 turn 覆盖 leaf/parent
        store.insert_stage_summary(parent).unwrap();
        store.insert_stage_summary(child).unwrap();
        let listed = store.list_summaries(&camp_id);
        assert_eq!(listed.len(), 2, "expected parent+child, got {listed:?}");

        camp.pending_compress_publication = Some(PendingCompressPublication::new(
            3,
            vec![Id::from_str("p1")],
            vec![(Id::from_str("c1"), Id::from_str("p1"))],
        ));
        store.update_campaign(camp).unwrap();
        assert!(store.needs_compress_metadata_heal(&camp_id));
        store.heal_compress_publication_metadata(&camp_id).unwrap();
        let camp2 = store.get_campaign(&camp_id).unwrap();
        assert!(camp2.pending_compress_publication.is_none());
        assert!(camp2.chronicle_revision > 3);
        assert!(camp2.context_epoch.is_none());
        // 再次 heal 幂等：不继续涨 revision
        let rev = camp2.chronicle_revision;
        store.heal_compress_publication_metadata(&camp_id).unwrap();
        assert_eq!(
            store.get_campaign(&camp_id).unwrap().chronicle_revision,
            rev
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn heal_keeps_marker_when_summaries_incomplete() {
        use storyforge_domain::chronicle::PendingCompressPublication;
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let camp_id = Id::from_str("camp-incomplete");
        let mut camp = Campaign::new(Id::from_str("card"), "n");
        camp.id = camp_id.clone();
        camp.chronicle_revision = 1;
        camp.pending_compress_publication = Some(PendingCompressPublication::new(
            1,
            vec![Id::from_str("missing-parent")],
            vec![(
                Id::from_str("missing-child"),
                Id::from_str("missing-parent"),
            )],
        ));
        store.save_campaign(camp).unwrap();
        let err = store
            .heal_compress_publication_metadata(&camp_id)
            .expect_err("should keep marker");
        assert!(
            err.contains("incomplete") || err.contains("missing"),
            "{err}"
        );
        let camp2 = store.get_campaign(&camp_id).unwrap();
        assert!(camp2.pending_compress_publication.is_some());
        assert_eq!(camp2.chronicle_revision, 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
