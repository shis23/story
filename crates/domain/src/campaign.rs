//! Campaign (playthrough save) and CharacterInstance (session-scoped character)
//!
//! Design ref §17 / INTENT D36-D38.
//!
//! A Campaign = one playthrough save = one data isolation domain.
//! Characters are split into "definition" (card-level, global) and
//! "instance" (session-level, carries this save's knowledge/state).

use crate::Id;
use crate::variables::VariableValue;
use serde::{Deserialize, Serialize};

// --- Campaign -------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Campaign {
    pub id: Id,
    pub card_id: Id,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_from: Option<(Id, Id)>,
    pub created_at: String,
    #[serde(default)]
    pub variables: Vec<VariableValue>,
    /// 故事时间（冗余缓存：真相源是 variables 里 key="story_clock" 的项）。
    ///
    /// 保留此顶层字段是为了向后兼容旧序列化数据 + 前端直接读取。set_variable("story_clock")
    /// 会同步更新两者。注意：若外部直接构造/反序列化导致两者不一致，应以 variables 为准
    /// （M-5 标注：理想做法是移除顶层字段统一到 variables，但会破坏序列化兼容，留待数据迁移专项）。
    #[serde(default = "default_story_clock")]
    pub story_clock: String,
}

fn default_story_clock() -> String {
    "Day 1".into()
}

impl Campaign {
    pub fn new(card_id: Id, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: Id::new(),
            card_id,
            name,
            fork_from: None,
            created_at: now_iso(),
            variables: crate::variables::init_values_from_schema(
                &crate::variables::default_campaign_variables(),
                0,
            ),
            story_clock: default_story_clock(),
        }
    }

    pub fn fork(
        card_id: Id,
        name: impl Into<String>,
        source_campaign_id: Id,
        fork_node_id: Id,
    ) -> Self {
        let name = name.into();
        Self {
            id: Id::new(),
            card_id,
            name,
            fork_from: Some((source_campaign_id, fork_node_id)),
            created_at: now_iso(),
            variables: crate::variables::init_values_from_schema(
                &crate::variables::default_campaign_variables(),
                0,
            ),
            story_clock: default_story_clock(),
        }
    }

    pub fn get_variable(&self, key: &str) -> Option<&serde_json::Value> {
        self.variables
            .iter()
            .find(|v| v.key == key)
            .map(|v| &v.value)
    }

    pub fn set_variable(&mut self, key: &str, value: serde_json::Value, turn: u32) {
        let clock_update = if key == "story_clock" {
            if let serde_json::Value::String(s) = &value {
                Some(s.clone())
            } else {
                None
            }
        } else {
            None
        };

        if let Some(v) = self.variables.iter_mut().find(|v| v.key == key) {
            v.value = value.clone();
            v.last_updated_turn = turn;
        } else {
            self.variables.push(VariableValue::new(key, value, turn));
        }

        if let Some(s) = clock_update {
            self.story_clock = s;
        }
    }
}

// --- CharacterInstance ----------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterInstance {
    pub id: Id,
    pub campaign_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_id: Option<Id>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior_override: Option<String>,
    #[serde(default)]
    pub variables: Vec<VariableValue>,
    #[serde(default)]
    pub is_temporary: bool,
}

impl CharacterInstance {
    pub fn from_definition(
        campaign_id: Id,
        definition: &crate::character::CharacterDefinition,
    ) -> Self {
        Self {
            id: Id::new(),
            campaign_id,
            definition_id: Some(definition.id.clone()),
            name: definition.name.clone(),
            persona_override: None,
            behavior_override: None,
            variables: crate::variables::init_values_from_schema(
                &definition.variable_schema,
                0,
            ),
            is_temporary: false,
        }
    }

    pub fn temporary(campaign_id: Id, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: Id::new(),
            campaign_id,
            definition_id: None,
            name,
            persona_override: None,
            behavior_override: None,
            variables: crate::variables::init_values_from_schema(
                &crate::variables::default_character_variables(),
                0,
            ),
            is_temporary: true,
        }
    }

    pub fn get_variable(&self, key: &str) -> Option<&serde_json::Value> {
        self.variables
            .iter()
            .find(|v| v.key == key)
            .map(|v| &v.value)
    }

    pub fn set_variable(&mut self, key: &str, value: serde_json::Value, turn: u32) {
        if let Some(v) = self.variables.iter_mut().find(|v| v.key == key) {
            v.value = value.clone();
            v.last_updated_turn = turn;
        } else {
            self.variables.push(VariableValue::new(key, value, turn));
        }
    }

    pub fn promote_to_permanent(&mut self) {
        self.is_temporary = false;
    }

    pub fn resolved_persona(&self) -> Option<&str> {
        self.persona_override.as_deref()
    }

    pub fn resolved_behavior(&self) -> Option<&str> {
        self.behavior_override.as_deref()
    }
}

// --- helpers --------------------------------------------------------------

/// 当前时间的 ISO-8601 / RFC3339 字符串（与 RoundSummary.created_at 格式一致）。
///
/// 历史版本误用 `SystemTime::as_secs()` 返回 Unix 秒数字符串，与字段名 `created_at`
/// 暗示的 ISO 格式矛盾，且与 `agent::RoundSummary` 的 rfc3339 格式不一致，统一修正。
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

// --- tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::CharacterDefinition;

    #[test]
    fn test_campaign_new_initializes_default_variables() {
        let card_id = Id::new();
        let campaign = Campaign::new(card_id.clone(), "run-1");
        assert_eq!(campaign.name, "run-1");
        assert_eq!(campaign.card_id, card_id);
        assert!(campaign.fork_from.is_none());
        assert_eq!(campaign.story_clock, "Day 1");
        assert!(campaign.get_variable("story_clock").is_some());
        assert!(campaign.get_variable("weather").is_some());
    }

    #[test]
    fn test_campaign_fork_records_source() {
        let card_id = Id::new();
        let src_campaign = Id::new();
        let fork_node = Id::new();
        let campaign = Campaign::fork(card_id, "fork", src_campaign.clone(), fork_node.clone());
        let (src, node) = campaign.fork_from.unwrap();
        assert_eq!(src, src_campaign);
        assert_eq!(node, fork_node);
    }

    #[test]
    fn test_campaign_set_variable_updates_story_clock() {
        let mut campaign = Campaign::new(Id::new(), "test");
        campaign.set_variable("story_clock", serde_json::json!("Day 47"), 5);
        assert_eq!(campaign.story_clock, "Day 47");
        assert_eq!(
            campaign.get_variable("story_clock"),
            Some(&serde_json::json!("Day 47"))
        );
    }

    #[test]
    fn test_campaign_set_variable_new_key_inserts() {
        let mut campaign = Campaign::new(Id::new(), "test");
        campaign.set_variable("custom_var", serde_json::json!(42), 3);
        assert_eq!(
            campaign.get_variable("custom_var"),
            Some(&serde_json::json!(42))
        );
    }

    #[test]
    fn test_character_instance_from_definition() {
        let def = CharacterDefinition {
            id: Id::from_str("char-lin"),
            card_id: Id::from_str("card-1"),
            name: "Lin".into(),
            persona_prompt: "calm".into(),
            behavior_rules: "save first".into(),
            base_backstory: vec!["is a surgeon".into()],
            group: Some("protagonist".into()),
            role_type: crate::character::RoleType::Protagonist,
            variable_schema: crate::variables::default_character_variables(),
        };
        let campaign_id = Id::new();
        let instance = CharacterInstance::from_definition(campaign_id.clone(), &def);

        assert_eq!(instance.name, "Lin");
        assert_eq!(instance.definition_id, Some(def.id.clone()));
        assert!(!instance.is_temporary);
        assert_eq!(instance.get_variable("hp"), Some(&serde_json::json!(100)));
    }

    #[test]
    fn test_character_instance_temporary() {
        let instance = CharacterInstance::temporary(Id::new(), "Wang");
        assert!(instance.is_temporary);
        assert!(instance.definition_id.is_none());
        assert_eq!(instance.name, "Wang");
        assert!(instance.get_variable("hp").is_some());
    }

    #[test]
    fn test_character_instance_promote_and_override() {
        let mut instance = CharacterInstance::temporary(Id::new(), "Wang");
        assert!(instance.is_temporary);
        assert!(instance.resolved_persona().is_none());

        instance.promote_to_permanent();
        assert!(!instance.is_temporary);

        instance.persona_override = Some("actually undercover".into());
        assert_eq!(instance.resolved_persona(), Some("actually undercover"));
    }

    #[test]
    fn test_character_instance_set_variable() {
        let mut instance = CharacterInstance::temporary(Id::new(), "test");
        instance.set_variable("hp", serde_json::json!(80), 5);
        assert_eq!(instance.get_variable("hp"), Some(&serde_json::json!(80)));

        instance.set_variable("fatigue", serde_json::json!(30), 5);
        assert_eq!(
            instance.get_variable("fatigue"),
            Some(&serde_json::json!(30))
        );
    }
}
