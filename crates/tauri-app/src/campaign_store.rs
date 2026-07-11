//! Campaign / CharacterCard / CharacterInstance 持久化
//!
//! 分七个文件（对应 P1/P2/P3 设计决策）：
//! - data/cards.json            —— CharacterCard（含 character_definitions）
//! - data/campaigns.json        —— Campaign
//! - data/instances.json        —— CharacterInstance（按 campaign_id 索引）
//! - data/knowledge.json        —— CharacterKnowledgeEntry（角色可见信息，P2 新增）
//! - data/tasks.json            —— StoryTask（叙事计划任务，P2 新增）
//! - data/round_summaries.json  —— RoundSummary（本轮剧情摘要，P2 新增）
//! - data/mvu_translations.json —— StoredMvuTranslation（MVU 五合一产物，P3 新增）
//!
//! 与现有 CharacterStore（扁平 Character）并存，向后兼容。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::{CharacterCard, RoleType};
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
use storyforge_domain::mvu_translation::MvuTranslation;
use storyforge_domain::story_task::StoryTask;

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
    /// 集合级缓存锁。写盘仍在对应集合锁内串行，避免锁外旧快照覆盖新快照。
    cards: Mutex<Vec<StoredCard>>,
    campaigns: Mutex<Vec<Campaign>>,
    instances: Mutex<Vec<CharacterInstance>>,
    knowledge: Mutex<Vec<CharacterKnowledgeEntry>>,
    tasks: Mutex<Vec<StoryTask>>,
    summaries: Mutex<Vec<RoundSummary>>,
    mvu: Mutex<Vec<StoredMvuTranslation>>,
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

        Self {
            cards: Mutex::new(load_or_default(&cards_path)),
            campaigns: Mutex::new(load_or_default(&campaigns_path)),
            instances: Mutex::new(load_or_default(&instances_path)),
            knowledge: Mutex::new(load_or_default(&knowledge_path)),
            tasks: Mutex::new(load_or_default(&tasks_path)),
            summaries: Mutex::new(load_or_default(&summaries_path)),
            mvu: Mutex::new(load_or_default(&mvu_path)),
            cards_path,
            campaigns_path,
            instances_path,
            knowledge_path,
            tasks_path,
            summaries_path,
            mvu_path,
        }
    }

    // ─── CharacterCard CRUD ───────────────────────────────────────────────

    pub fn list_cards(&self) -> Vec<StoredCard> {
        self.cards.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn get_card(&self, id: &Id) -> Option<StoredCard> {
        self.cards
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|c| c.card.id == *id)
            .cloned()
    }

    pub fn get_card_by_source(&self, source_character_id: &Id) -> Option<StoredCard> {
        self.cards
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|c| c.card.source_character_id == *source_character_id)
            .cloned()
    }

    pub fn save_card(&self, card: CharacterCard) -> Result<StoredCard, String> {
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
        self.campaigns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn list_campaigns_of_card(&self, card_id: &Id) -> Vec<Campaign> {
        self.campaigns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|c| c.card_id == *card_id)
            .cloned()
            .collect()
    }

    pub fn get_campaign(&self, id: &Id) -> Option<Campaign> {
        self.campaigns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|c| c.id == *id)
            .cloned()
    }

    pub fn save_campaign(&self, campaign: Campaign) -> Result<(), String> {
        let mut campaigns = self.campaigns.lock().unwrap_or_else(|p| p.into_inner());
        campaigns.retain(|c| c.id != campaign.id);
        campaigns.push(campaign);
        persist(&self.campaigns_path, &campaigns)
    }

    pub fn create_campaign_with_instances(
        &self,
        campaign: Campaign,
    ) -> Result<(StoredCard, Campaign, usize), String> {
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
        let mut campaigns = self.campaigns.lock().unwrap_or_else(|p| p.into_inner());
        let mut instances = self.instances.lock().unwrap_or_else(|p| p.into_inner());
        let mut knowledge = self.knowledge.lock().unwrap_or_else(|p| p.into_inner());
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        let mut summaries = self.summaries.lock().unwrap_or_else(|p| p.into_inner());

        let before = campaigns.len();
        campaigns.retain(|c| c.id != *id);
        let changed = campaigns.len() != before;
        if changed {
            persist(&self.campaigns_path, &campaigns)?;
            // 级联删除 instances + knowledge + tasks + round_summaries
            instances.retain(|i| i.campaign_id != *id);
            persist(&self.instances_path, &instances)?;
            knowledge.retain(|k| k.campaign_id != *id);
            persist(&self.knowledge_path, &knowledge)?;
            tasks.retain(|t| t.campaign_id != *id);
            persist(&self.tasks_path, &tasks)?;
            summaries.retain(|s| s.campaign_id != *id);
            persist(&self.summaries_path, &summaries)?;
        }
        Ok(changed)
    }

    // ─── CharacterInstance CRUD ───────────────────────────────────────────

    pub fn list_all_instances(&self) -> Vec<CharacterInstance> {
        self.instances
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn list_instances(&self, campaign_id: &Id) -> Vec<CharacterInstance> {
        self.instances
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|i| i.campaign_id == *campaign_id)
            .cloned()
            .collect()
    }

    pub fn get_instance(&self, campaign_id: &Id, instance_id: &Id) -> Option<CharacterInstance> {
        self.instances
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|i| i.campaign_id == *campaign_id && i.id == *instance_id)
            .cloned()
    }

    pub fn add_instance(&self, instance: CharacterInstance) -> Result<(), String> {
        let mut instances = self.instances.lock().unwrap_or_else(|p| p.into_inner());
        instances.retain(|i| i.id != instance.id);
        instances.push(instance);
        persist(&self.instances_path, &instances)
    }

    pub fn update_instance(&self, instance: CharacterInstance) -> Result<(), String> {
        let mut instances = self.instances.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = instances.iter().position(|i| i.id == instance.id) {
            instances[idx] = instance;
            persist(&self.instances_path, &instances)?;
        }
        Ok(())
    }

    // ─── CharacterKnowledge CRUD（P2 新增）─────────────────────────────────

    pub fn list_all_knowledge(&self) -> Vec<CharacterKnowledgeEntry> {
        self.knowledge
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// 查某 campaign 下所有角色的知识条目
    pub fn list_knowledge(&self, campaign_id: &Id) -> Vec<CharacterKnowledgeEntry> {
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
        if entries.is_empty() {
            return Ok(());
        }
        let mut knowledge = self.knowledge.lock().unwrap_or_else(|p| p.into_inner());
        knowledge.extend(entries);
        persist(&self.knowledge_path, &knowledge)
    }

    /// 删除单条知识条目（按 id），返回是否找到并删除
    pub fn delete_knowledge(&self, knowledge_id: &Id) -> Result<bool, String> {
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
        self.tasks.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// 查某 campaign 下所有任务（按 status 筛选：传 None 返回全部）
    pub fn list_tasks(&self, campaign_id: &Id) -> Vec<StoryTask> {
        self.tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|t| t.campaign_id == *campaign_id)
            .cloned()
            .collect()
    }

    pub fn get_task(&self, task_id: &Id) -> Option<StoryTask> {
        self.tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|t| t.id == *task_id)
            .cloned()
    }

    /// 新建任务（用户规划或后处理抽取）
    pub fn add_task(&self, task: StoryTask) -> Result<(), String> {
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        tasks.retain(|t| t.id != task.id);
        tasks.push(task);
        persist(&self.tasks_path, &tasks)
    }

    /// 更新任务（状态变化 / 注入记录 / 标完成）
    pub fn update_task(&self, task: StoryTask) -> Result<(), String> {
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = tasks.iter().position(|t| t.id == task.id) {
            tasks[idx] = task;
            persist(&self.tasks_path, &tasks)?;
        }
        Ok(())
    }

    /// 删除任务
    pub fn delete_task(&self, task_id: &Id) -> Result<bool, String> {
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
        self.summaries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// 查某 campaign 的所有本轮摘要（按 turn 升序）
    pub fn list_summaries(&self, campaign_id: &Id) -> Vec<RoundSummary> {
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
        let mut summaries = self.summaries.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = summaries.iter().position(|s| s.id == summary.id) {
            summaries[idx] = summary;
            return persist(&self.summaries_path, &summaries);
        }
        Err(format!("summary id={} 不存在", summary.id))
    }

    /// 插入 stage 纪要（B/C）：按 id 去重，不按 turn 覆盖 leaf A。
    pub fn insert_stage_summary(&self, summary: RoundSummary) -> Result<UpsertResult, String> {
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
    /// 半提交协议：
    /// 1) campaign.pending_compress_publication = intent（先落盘）
    /// 2) summaries 写入（clone-then-persist）
    /// 3) 完成：bump revision + clear epoch + clear pending
    /// heal 只依赖 pending marker，不依赖 epoch 是否仍在。
    pub fn publish_compress_result(
        &self,
        campaign_id: &Id,
        parents: &[RoundSummary],
        child_covered_by: &[(Id, Id)],
    ) -> Result<(), String> {
        use storyforge_domain::chronicle::PendingCompressPublication;

        let base_rev = self
            .get_campaign(campaign_id)
            .map(|c| c.chronicle_revision)
            .unwrap_or(0);
        let parent_ids: Vec<Id> = parents.iter().map(|p| p.id.clone()).collect();
        let child_ids: Vec<Id> = child_covered_by.iter().map(|(c, _)| c.clone()).collect();
        let pending = PendingCompressPublication::new(base_rev, parent_ids, child_ids);

        // 1) 先写 intent
        if let Some(mut camp) = self.get_campaign(campaign_id) {
            camp.pending_compress_publication = Some(pending);
            self.update_campaign(camp)?;
        }

        // 2) summaries：clone → persist → 写回内存（磁盘失败不污染内存）
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
                        return Err(format!("stage summary id={} conflict", parent.id));
                    }
                } else {
                    next.push(parent.clone());
                }
            }
            persist(&self.summaries_path, &next)?;
            *summaries = next;
        }

        // 3) 完成元数据
        self.complete_compress_publication(campaign_id)
    }

    /// 若存在 pending_compress_publication，完成 bump/clear；幂等。
    pub fn heal_compress_publication_metadata(&self, campaign_id: &Id) -> Result<(), String> {
        self.complete_compress_publication(campaign_id)
    }

    fn complete_compress_publication(&self, campaign_id: &Id) -> Result<(), String> {
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
        // 仅当 revision 尚未越过 base 时 bump，避免重复 heal 连涨
        if camp.chronicle_revision <= pending.base_chronicle_revision {
            camp.bump_chronicle_revision();
        }
        camp.context_epoch = None;
        camp.pending_compress_publication = None;
        self.update_campaign(camp)
    }

    /// 需要 heal：存在 pending marker，或（兼容）epoch 未失效且已有 stage/covered。
    pub fn needs_compress_metadata_heal(&self, campaign_id: &Id) -> bool {
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
        self.mvu.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// 查某角色卡的 MVU 翻译
    pub fn get_mvu(&self, source_character_id: &Id) -> Option<StoredMvuTranslation> {
        self.mvu
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|m| m.source_character_id == *source_character_id)
            .cloned()
    }

    /// 保存/覆盖某角色卡的 MVU 翻译（按 source_character_id 去重）
    pub fn save_mvu(&self, stored: StoredMvuTranslation) -> Result<(), String> {
        let mut mvu = self.mvu.lock().unwrap_or_else(|p| p.into_inner());
        mvu.retain(|m| m.source_character_id != stored.source_character_id);
        mvu.push(stored);
        persist(&self.mvu_path, &mvu)
    }

    /// 删某角色卡的 MVU 翻译（删卡时级联）
    pub fn delete_mvu(&self, source_character_id: &Id) -> Result<bool, String> {
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
        camp.pending_compress_publication = Some(PendingCompressPublication::new(
            3,
            vec![Id::from_str("p1")],
            vec![Id::from_str("c1")],
        ));
        store.save_campaign(camp).unwrap();
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
}
