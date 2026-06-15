//! Campaign / CharacterCard / CharacterInstance 持久化
//!
//! 分三个文件（对应 P1 设计决策）：
//! - data/cards.json       —— CharacterCard（含 character_definitions）
//! - data/campaigns.json   —— Campaign
//! - data/instances.json   —— CharacterInstance（按 campaign_id 索引）
//!
//! 与现有 CharacterStore（扁平 Character）并存，向后兼容。

use std::path::PathBuf;

use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::CharacterCard;
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
}

impl CampaignStore {
    pub fn new(data_dir: &PathBuf) -> Self {
        Self {
            cards_path: data_dir.join("cards.json"),
            campaigns_path: data_dir.join("campaigns.json"),
            instances_path: data_dir.join("instances.json"),
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
}
