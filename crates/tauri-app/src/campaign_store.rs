//! Campaign / CharacterCard / CharacterInstance 持久化
//!
//! 分六个文件（对应 P1/P2 设计决策）：
//! - data/cards.json            —— CharacterCard（含 character_definitions）
//! - data/campaigns.json        —— Campaign
//! - data/instances.json        —— CharacterInstance（按 campaign_id 索引）
//! - data/knowledge.json        —— CharacterKnowledgeEntry（角色可见信息，P2 新增）
//! - data/tasks.json            —— StoryTask（叙事计划任务，P2 新增）
//! - data/round_summaries.json  —— RoundSummary（本轮剧情摘要，P2 新增）
//!
//! 与现有 CharacterStore（扁平 Character）并存，向后兼容。

use std::path::PathBuf;

use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::CharacterCard;
use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
use storyforge_domain::story_task::StoryTask;
use storyforge_domain::Id;

// ─── CharacterCard 存储 ────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredCard {
    pub card: CharacterCard,
    pub imported_at: String,
}

pub struct CampaignStore {
    cards_path: PathBuf,
    campaigns_path: PathBuf,
    instances_path: PathBuf,
    knowledge_path: PathBuf,
    tasks_path: PathBuf,
    summaries_path: PathBuf,
}

impl CampaignStore {
    pub fn new(data_dir: &PathBuf) -> Self {
        Self {
            cards_path: data_dir.join("cards.json"),
            campaigns_path: data_dir.join("campaigns.json"),
            instances_path: data_dir.join("instances.json"),
            knowledge_path: data_dir.join("knowledge.json"),
            tasks_path: data_dir.join("tasks.json"),
            summaries_path: data_dir.join("round_summaries.json"),
        }
    }

    // ─── CharacterCard CRUD ───────────────────────────────────────────────

    pub fn list_cards(&self) -> Vec<StoredCard> {
        load_or_default(&self.cards_path)
    }

    pub fn get_card(&self, id: &Id) -> Option<StoredCard> {
        self.list_cards().into_iter().find(|c| c.card.id == *id)
    }

    pub fn get_card_by_source(&self, source_character_id: &Id) -> Option<StoredCard> {
        self.list_cards()
            .into_iter()
            .find(|c| c.card.source_character_id == *source_character_id)
    }

    pub fn save_card(&self, card: CharacterCard) -> StoredCard {
        let mut all = self.list_cards();
        // 同 source_character_id 去重（重跑识别时覆盖）
        all.retain(|c| c.card.source_character_id != card.source_character_id);
        let stored = StoredCard {
            card,
            imported_at: chrono::Utc::now().to_rfc3339(),
        };
        all.push(stored.clone());
        persist(&self.cards_path, &all);
        stored
    }

    pub fn update_card(&self, card: CharacterCard) -> Option<StoredCard> {
        let mut all = self.list_cards();
        let stored = StoredCard {
            card,
            imported_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Some(idx) = all.iter().position(|c| c.card.id == stored.card.id) {
            all[idx] = stored.clone();
            persist(&self.cards_path, &all);
            Some(stored)
        } else {
            None
        }
    }

    pub fn delete_card(&self, id: &Id) -> bool {
        let mut all = self.list_cards();
        let before = all.len();
        all.retain(|c| c.card.id != *id);
        let changed = all.len() != before;
        if changed {
            persist(&self.cards_path, &all);
            // 级联删除：该卡的 campaign + instances
            let camps = self
                .list_campaigns()
                .into_iter()
                .filter(|c| c.card_id == *id)
                .map(|c| c.id)
                .collect::<Vec<_>>();
            for camp_id in camps {
                self.delete_campaign(&camp_id);
            }
        }
        changed
    }

    // ─── Campaign CRUD ────────────────────────────────────────────────────

    pub fn list_campaigns(&self) -> Vec<Campaign> {
        load_or_default(&self.campaigns_path)
    }

    pub fn list_campaigns_of_card(&self, card_id: &Id) -> Vec<Campaign> {
        self.list_campaigns()
            .into_iter()
            .filter(|c| c.card_id == *card_id)
            .collect()
    }

    pub fn get_campaign(&self, id: &Id) -> Option<Campaign> {
        self.list_campaigns().into_iter().find(|c| c.id == *id)
    }

    pub fn save_campaign(&self, campaign: Campaign) {
        let mut all = self.list_campaigns();
        all.retain(|c| c.id != campaign.id);
        all.push(campaign);
        persist(&self.campaigns_path, &all);
    }

    pub fn update_campaign(&self, campaign: Campaign) {
        let mut all = self.list_campaigns();
        if let Some(idx) = all.iter().position(|c| c.id == campaign.id) {
            all[idx] = campaign;
            persist(&self.campaigns_path, &all);
        }
    }

    pub fn delete_campaign(&self, id: &Id) -> bool {
        let mut all = self.list_campaigns();
        let before = all.len();
        all.retain(|c| c.id != *id);
        let changed = all.len() != before;
        if changed {
            persist(&self.campaigns_path, &all);
            // 级联删除 instances
            let mut insts = self.list_all_instances();
            insts.retain(|i| i.campaign_id != *id);
            persist(&self.instances_path, &insts);
            // 级联删除 knowledge（P2 新增）
            let mut know = self.list_all_knowledge();
            know.retain(|k| k.campaign_id != *id);
            persist(&self.knowledge_path, &know);
            // 级联删除 tasks（P2 新增）
            let mut tasks = self.list_all_tasks();
            tasks.retain(|t| t.campaign_id != *id);
            persist(&self.tasks_path, &tasks);
            // 级联删除 round_summaries（P2 新增）
            let mut sums = self.list_all_summaries();
            sums.retain(|s| s.campaign_id != *id);
            persist(&self.summaries_path, &sums);
        }
        changed
    }

    // ─── CharacterInstance CRUD ───────────────────────────────────────────

    pub fn list_all_instances(&self) -> Vec<CharacterInstance> {
        load_or_default(&self.instances_path)
    }

    pub fn list_instances(&self, campaign_id: &Id) -> Vec<CharacterInstance> {
        self.list_all_instances()
            .into_iter()
            .filter(|i| i.campaign_id == *campaign_id)
            .collect()
    }

    pub fn get_instance(&self, campaign_id: &Id, instance_id: &Id) -> Option<CharacterInstance> {
        self.list_instances(campaign_id)
            .into_iter()
            .find(|i| i.id == *instance_id)
    }

    pub fn add_instance(&self, instance: CharacterInstance) {
        let mut all = self.list_all_instances();
        all.retain(|i| i.id != instance.id);
        all.push(instance);
        persist(&self.instances_path, &all);
    }

    pub fn update_instance(&self, instance: CharacterInstance) {
        let mut all = self.list_all_instances();
        if let Some(idx) = all.iter().position(|i| i.id == instance.id) {
            all[idx] = instance;
            persist(&self.instances_path, &all);
        }
    }

    // ─── CharacterKnowledge CRUD（P2 新增）─────────────────────────────────

    pub fn list_all_knowledge(&self) -> Vec<CharacterKnowledgeEntry> {
        load_or_default(&self.knowledge_path)
    }

    /// 查某 campaign 下所有角色的知识条目
    pub fn list_knowledge(&self, campaign_id: &Id) -> Vec<CharacterKnowledgeEntry> {
        self.list_all_knowledge()
            .into_iter()
            .filter(|k| k.campaign_id == *campaign_id)
            .collect()
    }

    /// 查某 campaign 下某角色的知识条目
    pub fn list_knowledge_of(
        &self,
        campaign_id: &Id,
        character_id: &Id,
    ) -> Vec<CharacterKnowledgeEntry> {
        self.list_knowledge(campaign_id)
            .into_iter()
            .filter(|k| k.character_id == *character_id)
            .collect()
    }

    /// 批量追加知识条目（后处理 Agent 产出后调用）
    pub fn add_knowledge(&self, entries: Vec<CharacterKnowledgeEntry>) {
        if entries.is_empty() {
            return;
        }
        let mut all = self.list_all_knowledge();
        all.extend(entries);
        persist(&self.knowledge_path, &all);
    }

    // ─── StoryTask CRUD（P2 新增）──────────────────────────────────────────

    pub fn list_all_tasks(&self) -> Vec<StoryTask> {
        load_or_default(&self.tasks_path)
    }

    /// 查某 campaign 下所有任务（按 status 筛选：传 None 返回全部）
    pub fn list_tasks(&self, campaign_id: &Id) -> Vec<StoryTask> {
        self.list_all_tasks()
            .into_iter()
            .filter(|t| t.campaign_id == *campaign_id)
            .collect()
    }

    pub fn get_task(&self, task_id: &Id) -> Option<StoryTask> {
        self.list_all_tasks()
            .into_iter()
            .find(|t| t.id == *task_id)
    }

    /// 新建任务（用户规划或后处理抽取）
    pub fn add_task(&self, task: StoryTask) {
        let mut all = self.list_all_tasks();
        all.retain(|t| t.id != task.id);
        all.push(task);
        persist(&self.tasks_path, &all);
    }

    /// 更新任务（状态变化 / 注入记录 / 标完成）
    pub fn update_task(&self, task: StoryTask) {
        let mut all = self.list_all_tasks();
        if let Some(idx) = all.iter().position(|t| t.id == task.id) {
            all[idx] = task;
            persist(&self.tasks_path, &all);
        }
    }

    /// 删除任务
    pub fn delete_task(&self, task_id: &Id) -> bool {
        let mut all = self.list_all_tasks();
        let before = all.len();
        all.retain(|t| t.id != *task_id);
        let changed = all.len() != before;
        if changed {
            persist(&self.tasks_path, &all);
        }
        changed
    }

    // ─── RoundSummary CRUD（P2 新增）───────────────────────────────────────

    pub fn list_all_summaries(&self) -> Vec<RoundSummary> {
        load_or_default(&self.summaries_path)
    }

    /// 查某 campaign 的所有本轮摘要（按 turn 升序）
    pub fn list_summaries(&self, campaign_id: &Id) -> Vec<RoundSummary> {
        let mut out: Vec<_> = self
            .list_all_summaries()
            .into_iter()
            .filter(|s| s.campaign_id == *campaign_id)
            .collect();
        out.sort_by_key(|s| s.turn);
        out
    }

    /// 追加一条本轮摘要（剧情总结 Agent 产出后调用）
    pub fn add_summary(&self, summary: RoundSummary) {
        let mut all = self.list_all_summaries();
        all.retain(|s| !(s.campaign_id == summary.campaign_id && s.turn == summary.turn));
        all.push(summary);
        persist(&self.summaries_path, &all);
    }
}

// ─── 持久化辅助 ─────────────────────────────────────────────────────────────

fn load_or_default<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Vec<T> {
    if !path.exists() {
        return vec![];
    }
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            tracing::warn!("加载 {} 失败，返回空: {e}", path.display());
            vec![]
        }),
        Err(_) => vec![],
    }
}

fn persist<T: serde::Serialize>(path: &PathBuf, data: &[T]) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::error!("创建数据目录失败: {e}");
            return;
        }
    }
    let json = match serde_json::to_string_pretty(data) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("序列化失败: {e}");
            return;
        }
    };
    // 原子写入：.tmp → rename
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, &json) {
        tracing::error!("写临时文件失败 {}: {e}", tmp.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        tracing::error!("rename 失败，回退直写: {e}");
        let _ = std::fs::write(path, json);
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

        let inst = CharacterInstance::from_definition(camp.id.clone(), &make_card().character_definitions[0]);
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
        let mut inst =
            CharacterInstance::from_definition(camp.id.clone(), &make_card().character_definitions[0]);
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
        let e1 = CharacterKnowledgeEntry::witnessed(
            camp_id.clone(),
            char_id.clone(),
            "看到尸体",
            1,
        );
        let e2 = CharacterKnowledgeEntry::backstory(
            camp_id.clone(),
            char_id.clone(),
            "我是外科医生",
        );
        store.add_knowledge(vec![e1.clone(), e2.clone()]);

        assert_eq!(store.list_knowledge(&camp_id).len(), 2);
        assert_eq!(store.list_knowledge_of(&camp_id, &char_id).len(), 2);
        // 另一角色查不到
        assert!(store
            .list_knowledge_of(&camp_id, &Id::from_str("char-other"))
            .is_empty());
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
}
