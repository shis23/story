//! Campaign (playthrough save) and CharacterInstance (session-scoped character)
//!
//! Design ref §17 / INTENT D36-D38.
//!
//! A Campaign = one playthrough save = one data isolation domain.
//! Characters are split into "definition" (card-level, global) and
//! "instance" (session-level, carries this save's knowledge/state).

use crate::Id;
use crate::variables::{VariableField, VariableValue};
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
    /// Campaign 级变量定义。旧存档缺失时回退系统基础 schema。
    #[serde(default = "crate::variables::default_campaign_variables")]
    pub variable_schema: Vec<VariableField>,
    /// 该 Campaign 绑定的唯一对话 ID（一 Campaign 一对话模型）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Id>,
    /// @deprecated -- use `get_variable("story_clock")` or `current_story_clock()` instead.
    /// Kept for backward compat (serialized by `set_variable`).
    /// Ideal: remove in data migration; this is the truth-source of the `variables` entry.
    #[serde(default = "default_story_clock")]
    pub story_clock: String,
    /// Campaign 聚合根的逻辑版本号。
    ///
    /// 一次 TurnCommit 或一次 MetaCommit 只 bump 一次(不是每个底层 CRUD bump)。
    /// 新建/fork 的 Campaign 从 0 开始;旧 JSON 反序列化为 0(向后兼容)。
    ///
    /// TurnAttempt.base_campaign_revision 与 accept 时的 CAS 校验依赖此字段,
    /// 用于检测"本轮基于的 Campaign revision 是否已被外部推进"。
    #[serde(default)]
    pub revision: u64,
    /// 记忆表示版本（Chronicle Accept / epoch rollover / 压缩发布等递增；与 revision 独立）。
    /// 旧 JSON 缺省为 0。
    #[serde(default)]
    pub chronicle_revision: u64,
    /// 当前写作记忆线（规格 lineage_id）；缺省 None → 运行时用 conversation 主线生成/回填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<Id>,
    /// 当前 Context epoch 快照（编译入口刷新；同 epoch 内 overview/band 冻结）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_epoch: Option<crate::chronicle::ContextEpochSnapshot>,
    /// 压缩发布半提交意图（summaries 已写、metadata 未完成时保留；heal 后清空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_compress_publication: Option<crate::chronicle::PendingCompressPublication>,
}

fn default_story_clock() -> String {
    "Day 1".into()
}

impl Campaign {
    pub fn new(card_id: Id, name: impl Into<String>) -> Self {
        Self::new_with_variable_schema(card_id, name, &[])
    }

    /// 用卡模板提供的全局字段创建 Campaign；卡字段可覆盖系统同名默认值。
    pub fn new_with_variable_schema(
        card_id: Id,
        name: impl Into<String>,
        card_schema: &[VariableField],
    ) -> Self {
        let name = name.into();
        let variable_schema = crate::variables::merge_schema(
            &crate::variables::default_campaign_variables(),
            card_schema,
        );
        Self {
            id: Id::new(),
            card_id,
            name,
            fork_from: None,
            created_at: now_iso(),
            variables: crate::variables::init_values_from_schema(&variable_schema, 0),
            variable_schema,
            conversation_id: None,
            story_clock: default_story_clock(),
            revision: 0,
            chronicle_revision: 0,
            lineage_id: Some(Id::new()),
            context_epoch: None,
            pending_compress_publication: None,
        }
    }

    pub fn fork(
        card_id: Id,
        name: impl Into<String>,
        source_campaign_id: Id,
        fork_node_id: Id,
    ) -> Self {
        let name = name.into();
        let variable_schema = crate::variables::default_campaign_variables();
        Self {
            id: Id::new(),
            card_id,
            name,
            fork_from: Some((source_campaign_id, fork_node_id)),
            created_at: now_iso(),
            variables: crate::variables::init_values_from_schema(&variable_schema, 0),
            variable_schema,
            conversation_id: None,
            story_clock: default_story_clock(),
            revision: 0,
            // fork：新记忆线（规格：fork_at → new lineage_id）
            chronicle_revision: 0,
            lineage_id: Some(Id::new()),
            context_epoch: None,
            pending_compress_publication: None,
        }
    }

    /// 确保有 lineage_id（旧存档迁移：惰性分配并返回是否新建）。
    pub fn ensure_lineage_id(&mut self) -> &Id {
        if self.lineage_id.is_none() {
            self.lineage_id = Some(Id::new());
        }
        self.lineage_id.as_ref().expect("just set")
    }

    pub fn bump_chronicle_revision(&mut self) {
        self.chronicle_revision = self.chronicle_revision.saturating_add(1);
    }

    pub fn get_variable(&self, key: &str) -> Option<&serde_json::Value> {
        self.variables
            .iter()
            .find(|v| v.key == key)
            .map(|v| &v.value)
    }

    /// Authoritative story clock: reads from `variables` first, falls back to
    /// the top-level `story_clock` field for old data that was never migrated.
    ///
    /// `variables["story_clock"]`（字符串值）是唯一权威；顶层字段仅为旧数据
    /// 保留的兼容镜像。两者不一致属于历史双表示残留，须经
    /// [`Campaign::repair_story_clock_authority`] 产生可审核修复，不得静默任选。
    pub fn current_story_clock(&self) -> &str {
        self.variables
            .iter()
            .find(|v| v.key == "story_clock")
            .and_then(|v| v.value.as_str())
            .unwrap_or(&self.story_clock)
    }

    /// Whether the legacy top-level `story_clock` field diverges from the
    /// authoritative `variables["story_clock"]` string entry.
    ///
    /// A non-string `variables["story_clock"]` value is not a valid authority
    /// (it is never accepted by `set_variable` as a clock update), so it is
    /// not treated as divergence — `current_story_clock()` falls back to the
    /// field in that case.
    pub fn story_clock_diverged(&self) -> bool {
        match self
            .variables
            .iter()
            .find(|v| v.key == "story_clock")
            .and_then(|v| v.value.as_str())
        {
            Some(authoritative) => authoritative != self.story_clock,
            None => false,
        }
    }

    /// Repair the legacy top-level field from the variables authority.
    /// Returns `true` when a divergence was corrected so callers can emit an
    /// auditable warning. This is the only sanctioned way to settle the old
    /// dual representation — never silently pick one side without repairing.
    pub fn repair_story_clock_authority(&mut self) -> bool {
        let Some(authoritative) = self
            .variables
            .iter()
            .find(|v| v.key == "story_clock")
            .and_then(|v| v.value.as_str())
        else {
            return false;
        };
        if authoritative != self.story_clock {
            self.story_clock = authoritative.to_string();
            true
        } else {
            false
        }
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

    /// 新增一个 Campaign 变量定义并用其默认值初始化当前值。
    pub fn add_variable_field(&mut self, mut field: VariableField) -> Result<(), String> {
        field.key = crate::variables::normalize_mvu_key(&field.key);
        if field.key.is_empty() {
            return Err("变量键名不能为空".into());
        }
        if self
            .variable_schema
            .iter()
            .any(|existing| existing.key == field.key)
        {
            return Err(format!("变量已存在: {}", field.key));
        }
        if self.get_variable(&field.key).is_none() {
            self.set_variable(&field.key, field.default.clone(), 0);
        }
        self.variable_schema.push(field);
        self.variable_schema
            .sort_by(|left, right| left.key.cmp(&right.key));
        Ok(())
    }

    /// 合并 schema，仅为缺失字段写入默认值。返回新增字段数。
    pub fn sync_variable_schema(&mut self, schema: &[VariableField]) -> usize {
        let merged = crate::variables::merge_schema(&self.variable_schema, schema);
        let added = merged
            .iter()
            .filter(|field| {
                !self
                    .variable_schema
                    .iter()
                    .any(|existing| existing.key == field.key)
            })
            .count();
        for field in &merged {
            if self.get_variable(&field.key).is_none() {
                self.set_variable(&field.key, field.default.clone(), 0);
            }
        }
        self.variable_schema = merged;
        added
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
            variables: crate::variables::init_values_from_schema(&definition.variable_schema, 0),
            is_temporary: false,
        }
    }

    pub fn temporary(campaign_id: Id, name: impl Into<String>) -> Self {
        Self::temporary_with_overrides(campaign_id, name, None, None)
    }

    /// 创建临时 instance，可选传入 persona/behavior override。
    ///
    /// Phase 6: Director 的 `context_package.character_brief` 可作为 persona_override
    /// 注入，使临时角色在当轮子 Agent 和落盘后都有可用的 persona。
    pub fn temporary_with_overrides(
        campaign_id: Id,
        name: impl Into<String>,
        persona_override: Option<String>,
        behavior_override: Option<String>,
    ) -> Self {
        let name = name.into();
        let name = if name.trim().is_empty() {
            "Unknown Character".to_string()
        } else {
            name.trim().to_string()
        };
        Self {
            id: Id::new(),
            campaign_id,
            definition_id: None,
            name,
            persona_override,
            behavior_override,
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

    /// Resolved persona: override 优先，fallback 到 definition.persona_prompt。
    ///
    /// Phase 1: definition 作为参数传入（而非存入 instance），保持 instance 轻量。
    pub fn resolved_persona<'a>(
        &'a self,
        definition: Option<&'a crate::character::CharacterDefinition>,
    ) -> Option<&'a str> {
        self.persona_override
            .as_deref()
            .or_else(|| definition.map(|d| d.persona_prompt.as_str()))
    }

    /// Resolved behavior: override 优先，fallback 到 definition.behavior_rules。
    pub fn resolved_behavior<'a>(
        &'a self,
        definition: Option<&'a crate::character::CharacterDefinition>,
    ) -> Option<&'a str> {
        self.behavior_override
            .as_deref()
            .or_else(|| definition.map(|d| d.behavior_rules.as_str()))
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
    fn test_campaign_new_with_schema_merges_card_globals_and_initializes_values() {
        let card_schema = vec![crate::variables::VariableField {
            key: "faction_tension".into(),
            label: "阵营紧张度".into(),
            value_type: crate::variables::VariableType::Int,
            default: serde_json::json!(12),
            description: Some("公开冲突风险".into()),
            group: Some("世界".into()),
        }];

        let campaign = Campaign::new_with_variable_schema(Id::new(), "schema-run", &card_schema);

        assert!(
            campaign
                .variable_schema
                .iter()
                .any(|field| field.key == "story_clock")
        );
        assert!(
            campaign
                .variable_schema
                .iter()
                .any(|field| field.key == "faction_tension")
        );
        assert_eq!(
            campaign.get_variable("faction_tension"),
            Some(&serde_json::json!(12))
        );
    }

    #[test]
    fn test_campaign_add_variable_field_rejects_duplicates_and_keeps_schema_value_together() {
        let mut campaign = Campaign::new(Id::new(), "schema-edit");
        let field = crate::variables::VariableField {
            key: "danger_level".into(),
            label: "危险等级".into(),
            value_type: crate::variables::VariableType::Int,
            default: serde_json::json!(2),
            description: None,
            group: Some("全局".into()),
        };

        assert!(campaign.add_variable_field(field.clone()).is_ok());
        assert_eq!(
            campaign.get_variable("danger_level"),
            Some(&serde_json::json!(2))
        );
        assert!(campaign.add_variable_field(field).is_err());
    }

    #[test]
    fn test_campaign_add_variable_field_preserves_an_existing_orphan_value() {
        let mut campaign = Campaign::new(Id::new(), "schema-repair");
        campaign.set_variable("danger_level", serde_json::json!(77), 4);

        campaign
            .add_variable_field(crate::variables::VariableField {
                key: "danger_level".into(),
                label: "危险等级".into(),
                value_type: crate::variables::VariableType::Int,
                default: serde_json::json!(2),
                description: Some("整局风险".into()),
                group: Some("全局".into()),
            })
            .unwrap();

        assert_eq!(
            campaign.get_variable("danger_level"),
            Some(&serde_json::json!(77))
        );
    }

    #[test]
    fn test_campaign_sync_schema_counts_new_definitions_even_when_value_already_exists() {
        let mut campaign = Campaign::new(Id::new(), "schema-sync-count");
        campaign.set_variable("danger_level", serde_json::json!(77), 4);
        let schema = vec![crate::variables::VariableField {
            key: "danger_level".into(),
            label: "危险等级".into(),
            value_type: crate::variables::VariableType::Int,
            default: serde_json::json!(2),
            description: None,
            group: Some("全局".into()),
        }];

        assert_eq!(campaign.sync_variable_schema(&schema), 1);
        assert_eq!(
            campaign.get_variable("danger_level"),
            Some(&serde_json::json!(77))
        );
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
    fn test_current_story_clock_reads_from_variables() {
        let mut campaign = Campaign::new(Id::new(), "test");
        // Default from variable_schema is "第1天"
        assert_eq!(campaign.current_story_clock(), "第1天");
        campaign.set_variable("story_clock", serde_json::json!("Day 100"), 3);
        assert_eq!(campaign.current_story_clock(), "Day 100");
    }

    #[test]
    fn test_current_story_clock_falls_back_to_field() {
        // Simulate old data where story_clock field is set but variables entry is missing
        let mut campaign = Campaign::new(Id::new(), "test");
        // Manually remove the story_clock variable to simulate desync
        campaign.variables.retain(|v| v.key != "story_clock");
        // Top-level field still has old value
        assert_eq!(campaign.current_story_clock(), "Day 1");
    }

    #[test]
    fn test_story_clock_divergence_is_detected_and_repaired_from_variables_authority() {
        // 旧数据：顶层字段与 variables 权威不一致（历史双表示残留）。
        let mut campaign = Campaign::new(Id::new(), "test");
        campaign.set_variable("story_clock", serde_json::json!("Day 47"), 5);
        // 人为制造分歧：直接改顶层字段，绕过 set_variable 的同步。
        campaign.story_clock = "Day 1".to_string();
        assert!(campaign.story_clock_diverged());
        assert_eq!(
            campaign.current_story_clock(),
            "Day 47",
            "variables 权威优先"
        );

        assert!(
            campaign.repair_story_clock_authority(),
            "repair must report the correction for audit"
        );
        assert_eq!(campaign.story_clock, "Day 47");
        assert!(!campaign.story_clock_diverged());
        assert!(
            !campaign.repair_story_clock_authority(),
            "second repair must be a no-op"
        );
    }

    #[test]
    fn test_story_clock_non_string_variable_is_not_a_divergence() {
        // 非字符串 story_clock 变量不是合法权威（set_variable 不把它当 clock
        // 更新）；此时不得把它当作「与字段分歧」处理，也不得用它覆盖字段。
        let mut campaign = Campaign::new(Id::new(), "test");
        campaign.set_variable("story_clock", serde_json::json!("Day 9"), 0);
        // 人为制造旧数据形态：字段 = "Day 1"（老值），variables 项被替换成
        // 非字符串（set_variable 从不接受非字符串 clock 更新 → 不是合法权威）。
        campaign.story_clock = "Day 1".to_string();
        campaign.variables.retain(|v| v.key != "story_clock");
        campaign
            .variables
            .push(crate::variables::VariableValue::new(
                "story_clock",
                serde_json::json!(9),
                0,
            ));
        assert!(!campaign.story_clock_diverged());
        assert!(!campaign.repair_story_clock_authority());
        assert_eq!(campaign.current_story_clock(), "Day 1", "field fallback");
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
        // 无 override、无 definition → None
        assert!(instance.resolved_persona(None).is_none());

        instance.promote_to_permanent();
        assert!(!instance.is_temporary);

        instance.persona_override = Some("actually undercover".into());
        assert_eq!(instance.resolved_persona(None), Some("actually undercover"));
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

    // --- Phase 1: resolved_persona / resolved_behavior definition fallback ---

    fn make_definition() -> CharacterDefinition {
        CharacterDefinition {
            id: Id::from_str("def-1"),
            card_id: Id::from_str("card-1"),
            name: "Lin".into(),
            persona_prompt: "calm surgeon".into(),
            behavior_rules: "save first, ask later".into(),
            base_backstory: vec!["is a surgeon".into()],
            group: Some("protagonist".into()),
            role_type: crate::character::RoleType::Protagonist,
            variable_schema: crate::variables::default_character_variables(),
        }
    }

    #[test]
    fn resolved_persona_override_takes_priority() {
        let mut instance = CharacterInstance::temporary(Id::new(), "Lin");
        instance.persona_override = Some("angry variant".into());
        let def = make_definition();
        assert_eq!(
            instance.resolved_persona(Some(&def)),
            Some("angry variant"),
            "override must take priority over definition"
        );
    }

    #[test]
    fn resolved_persona_falls_back_to_definition() {
        let instance = CharacterInstance::temporary(Id::new(), "Lin");
        // instance has no persona_override
        assert!(instance.persona_override.is_none());
        let def = make_definition();
        assert_eq!(
            instance.resolved_persona(Some(&def)),
            Some("calm surgeon"),
            "must fall back to definition.persona_prompt"
        );
    }

    #[test]
    fn resolved_persona_none_when_nothing_available() {
        let instance = CharacterInstance::temporary(Id::new(), "Ghost");
        assert_eq!(
            instance.resolved_persona(None),
            None,
            "no override and no definition → None"
        );
    }

    #[test]
    fn resolved_behavior_override_takes_priority() {
        let mut instance = CharacterInstance::temporary(Id::new(), "Lin");
        instance.behavior_override = Some("reckless".into());
        let def = make_definition();
        assert_eq!(
            instance.resolved_behavior(Some(&def)),
            Some("reckless"),
            "override must take priority over definition"
        );
    }

    #[test]
    fn resolved_behavior_falls_back_to_definition() {
        let instance = CharacterInstance::temporary(Id::new(), "Lin");
        assert!(instance.behavior_override.is_none());
        let def = make_definition();
        assert_eq!(
            instance.resolved_behavior(Some(&def)),
            Some("save first, ask later"),
            "must fall back to definition.behavior_rules"
        );
    }

    #[test]
    fn resolved_behavior_none_when_nothing_available() {
        let instance = CharacterInstance::temporary(Id::new(), "Ghost");
        assert_eq!(
            instance.resolved_behavior(None),
            None,
            "no override and no definition → None"
        );
    }

    #[test]
    fn resolved_persona_empty_override_does_not_fallback() {
        let mut instance = CharacterInstance::temporary(Id::new(), "Lin");
        instance.persona_override = Some("".into());
        let def = make_definition();
        // Empty string is still "present" — override wins even if empty
        assert_eq!(
            instance.resolved_persona(Some(&def)),
            Some(""),
            "explicit empty override is still an override"
        );
    }

    #[test]
    fn resolved_methods_from_definition_instance() {
        // An instance created from_definition has no overrides
        let def = make_definition();
        let instance = CharacterInstance::from_definition(Id::new(), &def);
        assert_eq!(instance.resolved_persona(Some(&def)), Some("calm surgeon"));
        assert_eq!(
            instance.resolved_behavior(Some(&def)),
            Some("save first, ask later")
        );
    }

    // --- Phase 6: temporary_with_overrides ---

    #[test]
    fn temporary_with_overrides_sets_persona() {
        let instance = CharacterInstance::temporary_with_overrides(
            Id::new(),
            "AdHoc",
            Some("mysterious stranger".into()),
            None,
        );
        assert!(instance.is_temporary);
        assert_eq!(instance.name, "AdHoc");
        assert_eq!(
            instance.persona_override,
            Some("mysterious stranger".into())
        );
        assert!(instance.behavior_override.is_none());
        assert_eq!(instance.resolved_persona(None), Some("mysterious stranger"));
    }

    #[test]
    fn temporary_with_overrides_sets_behavior() {
        let instance = CharacterInstance::temporary_with_overrides(
            Id::new(),
            "Guard",
            None,
            Some("block the way".into()),
        );
        assert!(instance.is_temporary);
        assert!(instance.persona_override.is_none());
        assert_eq!(instance.behavior_override, Some("block the way".into()));
        assert_eq!(instance.resolved_behavior(None), Some("block the way"));
    }

    #[test]
    fn temporary_with_overrides_sets_both() {
        let instance = CharacterInstance::temporary_with_overrides(
            Id::new(),
            "NPC",
            Some("friendly shopkeeper".into()),
            Some("offer discounts".into()),
        );
        assert_eq!(instance.resolved_persona(None), Some("friendly shopkeeper"));
        assert_eq!(instance.resolved_behavior(None), Some("offer discounts"));
    }

    #[test]
    fn temporary_with_overrides_none_equivalent_to_temporary() {
        let a = CharacterInstance::temporary_with_overrides(Id::new(), "Test", None, None);
        let b = CharacterInstance::temporary(Id::new(), "Test");
        // Both should have the same field values (except id which is random)
        assert_eq!(a.name, b.name);
        assert_eq!(a.is_temporary, b.is_temporary);
        assert_eq!(a.persona_override, b.persona_override);
        assert_eq!(a.behavior_override, b.behavior_override);
        assert_eq!(a.definition_id, b.definition_id);
    }

    // --- Phase A: Campaign.revision ---

    #[test]
    fn test_campaign_new_revision_starts_at_zero() {
        let campaign = Campaign::new(Id::new(), "run-1");
        assert_eq!(
            campaign.revision, 0,
            "new campaign must start at revision 0"
        );
    }

    #[test]
    fn test_campaign_fork_revision_starts_at_zero() {
        let campaign = Campaign::fork(Id::new(), "fork", Id::new(), Id::new());
        assert_eq!(
            campaign.revision, 0,
            "forked campaign must start at revision 0"
        );
    }

    #[test]
    fn test_campaign_revision_backward_compat_old_json() {
        // Simulate old JSON that has no revision field at all
        let old_json = serde_json::json!({
            "id": "camp-old",
            "card_id": "card-1",
            "name": "old campaign",
            "created_at": "2026-01-01T00:00:00Z",
            "story_clock": "Day 1"
        });
        let campaign: Campaign = serde_json::from_value(old_json)
            .expect("old JSON without revision must deserialize successfully (serde default = 0)");
        assert_eq!(
            campaign.revision, 0,
            "missing revision field must default to 0"
        );
    }

    #[test]
    fn test_campaign_revision_serializes_and_round_trips() {
        let mut campaign = Campaign::new(Id::new(), "test");
        campaign.revision = 42;
        let json = serde_json::to_string(&campaign).unwrap();
        let restored: Campaign = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.revision, 42, "revision must survive round-trip");
    }
}
