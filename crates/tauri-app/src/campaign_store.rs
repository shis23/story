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

use std::path::PathBuf;
use std::sync::Mutex;

use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::CharacterCard;
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

/// 内存缓存（所有 7 个数据类型的集合，由单个 Mutex 保护）
struct CampaignCache {
    cards: Vec<StoredCard>,
    campaigns: Vec<Campaign>,
    instances: Vec<CharacterInstance>,
    knowledge: Vec<CharacterKnowledgeEntry>,
    tasks: Vec<StoryTask>,
    summaries: Vec<RoundSummary>,
    mvu: Vec<StoredMvuTranslation>,
}

pub struct CampaignStore {
    cards_path: PathBuf,
    campaigns_path: PathBuf,
    instances_path: PathBuf,
    knowledge_path: PathBuf,
    tasks_path: PathBuf,
    summaries_path: PathBuf,
    mvu_path: PathBuf,
    /// 内存缓存，保护并发读写（与 CharacterStore/ConnectionStore 模式一致）
    cache: Mutex<CampaignCache>,
}

impl CampaignStore {
    pub fn new(data_dir: &PathBuf) -> Self {
        let cards_path = data_dir.join("cards.json");
        let campaigns_path = data_dir.join("campaigns.json");
        let instances_path = data_dir.join("instances.json");
        let knowledge_path = data_dir.join("knowledge.json");
        let tasks_path = data_dir.join("tasks.json");
        let summaries_path = data_dir.join("round_summaries.json");
        let mvu_path = data_dir.join("mvu_translations.json");

        let cache = CampaignCache {
            cards: load_or_default(&cards_path),
            campaigns: load_or_default(&campaigns_path),
            instances: load_or_default(&instances_path),
            knowledge: load_or_default(&knowledge_path),
            tasks: load_or_default(&tasks_path),
            summaries: load_or_default(&summaries_path),
            mvu: load_or_default(&mvu_path),
        };

        Self {
            cards_path,
            campaigns_path,
            instances_path,
            knowledge_path,
            tasks_path,
            summaries_path,
            mvu_path,
            cache: Mutex::new(cache),
        }
    }

    // ─── CharacterCard CRUD ───────────────────────────────────────────────

    pub fn list_cards(&self) -> Vec<StoredCard> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.cards.clone()
    }

    pub fn get_card(&self, id: &Id) -> Option<StoredCard> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.cards.iter().find(|c| c.card.id == *id).cloned()
    }

    pub fn get_card_by_source(&self, source_character_id: &Id) -> Option<StoredCard> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .cards
            .iter()
            .find(|c| c.card.source_character_id == *source_character_id)
            .cloned()
    }

    pub fn save_card(&self, card: CharacterCard) -> StoredCard {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        // 同 source_character_id 去重（重跑识别时覆盖）
        cache
            .cards
            .retain(|c| c.card.source_character_id != card.source_character_id);
        let stored = StoredCard {
            card,
            imported_at: chrono::Utc::now().to_rfc3339(),
        };
        cache.cards.push(stored.clone());
        persist(&self.cards_path, &cache.cards);
        stored
    }

    pub fn update_card(&self, card: CharacterCard) -> Option<StoredCard> {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        let stored = StoredCard {
            card,
            imported_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Some(idx) = cache.cards.iter().position(|c| c.card.id == stored.card.id) {
            cache.cards[idx] = stored.clone();
            persist(&self.cards_path, &cache.cards);
            Some(stored)
        } else {
            None
        }
    }

    pub fn delete_card(&self, id: &Id) -> bool {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        let before = cache.cards.len();
        // 先记下要删的卡的 source_character_id（用于级联删 MVU 翻译）
        let source_ids: Vec<Id> = cache
            .cards
            .iter()
            .filter(|c| c.card.id == *id)
            .map(|c| c.card.source_character_id.clone())
            .collect();
        cache.cards.retain(|c| c.card.id != *id);
        let changed = cache.cards.len() != before;
        if changed {
            persist(&self.cards_path, &cache.cards);
            // 级联删除：该卡的 campaign + instances
            let camp_ids: Vec<Id> = cache
                .campaigns
                .iter()
                .filter(|c| c.card_id == *id)
                .map(|c| c.id.clone())
                .collect();
            for camp_id in &camp_ids {
                cache.campaigns.retain(|c| c.id != *camp_id);
                cache.instances.retain(|i| i.campaign_id != *camp_id);
                cache.knowledge.retain(|k| k.campaign_id != *camp_id);
                cache.tasks.retain(|t| t.campaign_id != *camp_id);
                cache.summaries.retain(|s| s.campaign_id != *camp_id);
            }
            persist(&self.campaigns_path, &cache.campaigns);
            persist(&self.instances_path, &cache.instances);
            persist(&self.knowledge_path, &cache.knowledge);
            persist(&self.tasks_path, &cache.tasks);
            persist(&self.summaries_path, &cache.summaries);
            // 级联删除：该卡的 MVU 翻译
            for source_id in &source_ids {
                cache.mvu.retain(|m| m.source_character_id != *source_id);
            }
            persist(&self.mvu_path, &cache.mvu);
        }
        changed
    }

    // ─── Campaign CRUD ────────────────────────────────────────────────────

    pub fn list_campaigns(&self) -> Vec<Campaign> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.campaigns.clone()
    }

    pub fn list_campaigns_of_card(&self, card_id: &Id) -> Vec<Campaign> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .campaigns
            .iter()
            .filter(|c| c.card_id == *card_id)
            .cloned()
            .collect()
    }

    pub fn get_campaign(&self, id: &Id) -> Option<Campaign> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.campaigns.iter().find(|c| c.id == *id).cloned()
    }

    pub fn save_campaign(&self, campaign: Campaign) {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.campaigns.retain(|c| c.id != campaign.id);
        cache.campaigns.push(campaign);
        persist(&self.campaigns_path, &cache.campaigns);
    }

    pub fn update_campaign(&self, campaign: Campaign) {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = cache.campaigns.iter().position(|c| c.id == campaign.id) {
            cache.campaigns[idx] = campaign;
            persist(&self.campaigns_path, &cache.campaigns);
        }
    }

    pub fn delete_campaign(&self, id: &Id) -> bool {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        let before = cache.campaigns.len();
        cache.campaigns.retain(|c| c.id != *id);
        let changed = cache.campaigns.len() != before;
        if changed {
            persist(&self.campaigns_path, &cache.campaigns);
            // 级联删除 instances + knowledge + tasks + round_summaries
            cache.instances.retain(|i| i.campaign_id != *id);
            persist(&self.instances_path, &cache.instances);
            cache.knowledge.retain(|k| k.campaign_id != *id);
            persist(&self.knowledge_path, &cache.knowledge);
            cache.tasks.retain(|t| t.campaign_id != *id);
            persist(&self.tasks_path, &cache.tasks);
            cache.summaries.retain(|s| s.campaign_id != *id);
            persist(&self.summaries_path, &cache.summaries);
        }
        changed
    }

    // ─── CharacterInstance CRUD ───────────────────────────────────────────

    pub fn list_all_instances(&self) -> Vec<CharacterInstance> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.instances.clone()
    }

    pub fn list_instances(&self, campaign_id: &Id) -> Vec<CharacterInstance> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .instances
            .iter()
            .filter(|i| i.campaign_id == *campaign_id)
            .cloned()
            .collect()
    }

    pub fn get_instance(&self, campaign_id: &Id, instance_id: &Id) -> Option<CharacterInstance> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .instances
            .iter()
            .find(|i| i.campaign_id == *campaign_id && i.id == *instance_id)
            .cloned()
    }

    pub fn add_instance(&self, instance: CharacterInstance) {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.instances.retain(|i| i.id != instance.id);
        cache.instances.push(instance);
        persist(&self.instances_path, &cache.instances);
    }

    pub fn update_instance(&self, instance: CharacterInstance) {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = cache.instances.iter().position(|i| i.id == instance.id) {
            cache.instances[idx] = instance;
            persist(&self.instances_path, &cache.instances);
        }
    }

    // ─── CharacterKnowledge CRUD（P2 新增）─────────────────────────────────

    pub fn list_all_knowledge(&self) -> Vec<CharacterKnowledgeEntry> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.knowledge.clone()
    }

    /// 查某 campaign 下所有角色的知识条目
    pub fn list_knowledge(&self, campaign_id: &Id) -> Vec<CharacterKnowledgeEntry> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .knowledge
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
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .knowledge
            .iter()
            .filter(|k| k.campaign_id == *campaign_id && k.character_id == *character_id)
            .cloned()
            .collect()
    }

    /// 批量追加知识条目（后处理 Agent 产出后调用）
    pub fn add_knowledge(&self, entries: Vec<CharacterKnowledgeEntry>) {
        if entries.is_empty() {
            return;
        }
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.knowledge.extend(entries);
        persist(&self.knowledge_path, &cache.knowledge);
    }

    // ─── StoryTask CRUD（P2 新增）──────────────────────────────────────────

    pub fn list_all_tasks(&self) -> Vec<StoryTask> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.tasks.clone()
    }

    /// 查某 campaign 下所有任务（按 status 筛选：传 None 返回全部）
    pub fn list_tasks(&self, campaign_id: &Id) -> Vec<StoryTask> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .tasks
            .iter()
            .filter(|t| t.campaign_id == *campaign_id)
            .cloned()
            .collect()
    }

    pub fn get_task(&self, task_id: &Id) -> Option<StoryTask> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.tasks.iter().find(|t| t.id == *task_id).cloned()
    }

    /// 新建任务（用户规划或后处理抽取）
    pub fn add_task(&self, task: StoryTask) {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.tasks.retain(|t| t.id != task.id);
        cache.tasks.push(task);
        persist(&self.tasks_path, &cache.tasks);
    }

    /// 更新任务（状态变化 / 注入记录 / 标完成）
    pub fn update_task(&self, task: StoryTask) {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = cache.tasks.iter().position(|t| t.id == task.id) {
            cache.tasks[idx] = task;
            persist(&self.tasks_path, &cache.tasks);
        }
    }

    /// 删除任务
    pub fn delete_task(&self, task_id: &Id) -> bool {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        let before = cache.tasks.len();
        cache.tasks.retain(|t| t.id != *task_id);
        let changed = cache.tasks.len() != before;
        if changed {
            persist(&self.tasks_path, &cache.tasks);
        }
        changed
    }

    // ─── RoundSummary CRUD（P2 新增）───────────────────────────────────────

    pub fn list_all_summaries(&self) -> Vec<RoundSummary> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.summaries.clone()
    }

    /// 查某 campaign 的所有本轮摘要（按 turn 升序）
    pub fn list_summaries(&self, campaign_id: &Id) -> Vec<RoundSummary> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        let mut out: Vec<_> = cache
            .summaries
            .iter()
            .filter(|s| s.campaign_id == *campaign_id)
            .cloned()
            .collect();
        out.sort_by_key(|s| s.turn);
        out
    }

    /// 追加一条本轮摘要（剧情总结 Agent 产出后调用）
    pub fn add_summary(&self, summary: RoundSummary) {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .summaries
            .retain(|s| !(s.campaign_id == summary.campaign_id && s.turn == summary.turn));
        cache.summaries.push(summary);
        persist(&self.summaries_path, &cache.summaries);
    }

    // ─── MVU 翻译存储（P3 新增）──────────────────────────────────────────

    /// 列所有 MVU 翻译
    pub fn list_all_mvu(&self) -> Vec<StoredMvuTranslation> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.mvu.clone()
    }

    /// 查某角色卡的 MVU 翻译
    pub fn get_mvu(&self, source_character_id: &Id) -> Option<StoredMvuTranslation> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .mvu
            .iter()
            .find(|m| m.source_character_id == *source_character_id)
            .cloned()
    }

    /// 保存/覆盖某角色卡的 MVU 翻译（按 source_character_id 去重）
    pub fn save_mvu(&self, stored: StoredMvuTranslation) {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .mvu
            .retain(|m| m.source_character_id != stored.source_character_id);
        cache.mvu.push(stored);
        persist(&self.mvu_path, &cache.mvu);
    }

    /// 删某角色卡的 MVU 翻译（删卡时级联）
    pub fn delete_mvu(&self, source_character_id: &Id) -> bool {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        let before = cache.mvu.len();
        cache
            .mvu
            .retain(|m| m.source_character_id != *source_character_id);
        let changed = cache.mvu.len() != before;
        if changed {
            persist(&self.mvu_path, &cache.mvu);
        }
        changed
    }
}

// ─── 持久化辅助 ─────────────────────────────────────────────────────────────

fn load_or_default<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Vec<T> {
    if !path.exists() {
        return vec![];
    }
    match std::fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str(&s) {
            Ok(data) => return data,
            Err(e) => {
                // 主文件损坏，尝试读 .tmp 备份（atomic_write 先写 .tmp 再 rename，
                // 崩溃时 .tmp 可能保存了最新数据）
                tracing::warn!("加载 {} 失败({e})，尝试 .tmp 备份", path.display());
                let tmp_path = std::path::PathBuf::from(format!("{}.tmp", path.display()));
                if let Ok(tmp_s) = std::fs::read_to_string(&tmp_path) {
                    if let Ok(data) = serde_json::from_str(&tmp_s) {
                        tracing::info!("从 .tmp 备份恢复成功: {}", tmp_path.display());
                        return data;
                    }
                }
                tracing::error!(
                    "加载 {} 失败且无可用备份，返回空（下次保存将覆盖！）",
                    path.display()
                );
                vec![]
            }
        },
        Err(_) => vec![],
    }
}

fn persist<T: serde::Serialize>(path: &PathBuf, data: &[T]) {
    if let Err(e) = storyforge_infra_util::atomic_write_json(path, data) {
        tracing::error!("持久化失败 {}: {e}", path.display());
    }
}

// ─── 测试 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
    use storyforge_domain::variables::default_character_variables;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sf-campaign-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
    fn test_card_save_get_list() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let card = make_card();
        let stored = store.save_card(card.clone());
        assert!(stored.imported_at.len() > 0);

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
        store.save_card(card.clone());
        // 重跑（同名新 id），应覆盖而非累积
        card.id = Id::from_str("card-2");
        store.save_card(card);
        assert_eq!(store.list_cards().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_campaign_and_instances_cascade_delete() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);

        // 建 card
        store.save_card(make_card());

        // 建 campaign + instances
        let camp_id = Id::from_str("camp-1");
        let camp = Campaign::new(Id::from_str("card-1"), "新档".to_string());
        let _ = camp_id;
        store.save_campaign(camp.clone());

        let inst = CharacterInstance::from_definition(
            camp.id.clone(),
            &make_card().character_definitions[0],
        );
        store.add_instance(inst.clone());

        assert_eq!(store.list_campaigns().len(), 1);
        assert_eq!(store.list_instances(&camp.id).len(), 1);

        // 删 card 级联删 campaign + instances
        assert!(store.delete_card(&Id::from_str("card-1")));
        assert_eq!(store.list_campaigns().len(), 0);
        assert_eq!(store.list_all_instances().len(), 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_persistence_reload() {
        let dir = temp_dir();
        {
            let store = CampaignStore::new(&dir);
            store.save_card(make_card());
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
        store.save_campaign(camp.clone());
        store.add_instance(inst.clone());

        inst.set_variable("hp", serde_json::json!(50), 3);
        store.update_instance(inst.clone());

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
        store.add_knowledge(vec![e1.clone(), e2.clone()]);

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
        store.add_task(task);
        assert_eq!(store.list_tasks(&camp_id).len(), 1);

        // 更新：标完成
        let mut got = store.get_task(&task_id).unwrap();
        got.complete();
        store.update_task(got);
        assert_eq!(
            store.get_task(&task_id).unwrap().status,
            storyforge_domain::story_task::TaskStatus::Completed
        );

        // 删除
        assert!(store.delete_task(&task_id));
        assert!(store.get_task(&task_id).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_summary_add_dedup_by_turn_and_sorted() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let camp_id = Id::from_str("camp-s");
        let conv = Id::from_str("conv-1");

        store.add_summary(RoundSummary::new(
            camp_id.clone(),
            conv.clone(),
            2,
            "第二轮".into(),
        ));
        store.add_summary(RoundSummary::new(
            camp_id.clone(),
            conv.clone(),
            1,
            "第一轮".into(),
        ));
        // 同 turn 覆盖
        store.add_summary(RoundSummary::new(
            camp_id.clone(),
            conv.clone(),
            1,
            "第一轮（重写）".into(),
        ));

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
        store.save_card(make_card()); // 保证 card-1 存在

        // 建 campaign（Campaign::new 内部分配 id）
        let camp = Campaign::new(Id::from_str("card-1"), "cascade");
        let camp_id = camp.id.clone();
        store.save_campaign(camp);

        // 塞三类 P2 数据
        store.add_knowledge(vec![CharacterKnowledgeEntry::witnessed(
            camp_id.clone(),
            Id::from_str("c1"),
            "x",
            1,
        )]);
        store.add_task(StoryTask::user_planned(
            camp_id.clone(),
            "t",
            "d",
            vec![],
            1,
        ));
        store.add_summary(RoundSummary::new(
            camp_id.clone(),
            Id::from_str("conv"),
            1,
            "s".into(),
        ));

        assert_eq!(store.list_knowledge(&camp_id).len(), 1);
        assert_eq!(store.list_tasks(&camp_id).len(), 1);
        assert_eq!(store.list_summaries(&camp_id).len(), 1);

        // 删 campaign 级联清掉三类
        assert!(store.delete_campaign(&camp_id));
        assert!(store.list_knowledge(&camp_id).is_empty());
        assert!(store.list_tasks(&camp_id).is_empty());
        assert!(store.list_summaries(&camp_id).is_empty());

        std::fs::remove_dir_all(&dir).ok();
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
        store.save_mvu(make_mvu("src-1", "测试卡A"));
        store.save_mvu(make_mvu("src-2", "测试卡B"));
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
        store.save_mvu(make_mvu("src-1", "测试卡A-改"));
        assert_eq!(store.list_all_mvu().len(), 2);
        assert_eq!(
            store
                .get_mvu(&Id::from_str("src-1"))
                .unwrap()
                .character_name,
            "测试卡A-改"
        );

        // 删
        assert!(store.delete_mvu(&Id::from_str("src-1")));
        assert_eq!(store.list_all_mvu().len(), 1);
        assert!(store.get_mvu(&Id::from_str("src-1")).is_none());
        // 再删返回 false
        assert!(!store.delete_mvu(&Id::from_str("src-1")));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_mvu_cascade_delete_on_card_delete() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        store.save_card(make_card()); // card-1, source src-1
        store.save_mvu(make_mvu("src-1", "测试卡"));

        assert!(store.get_mvu(&Id::from_str("src-1")).is_some());

        // 删卡 → MVU 级联清掉
        assert!(store.delete_card(&Id::from_str("card-1")));
        assert!(store.get_mvu(&Id::from_str("src-1")).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
