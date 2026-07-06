//! MVU schema apply patch：提议→预览→接受→写盘闭环。
//!
//! 纯函数层，不碰 store lock。store 访问在 Tauri 命令层（lib.rs）。

use serde::{Deserialize, Serialize};
use storyforge_domain::character::CharacterDefinition;
use storyforge_domain::variables::{VariableField, merge_schema};

/// MVU schema 合并预览：对比 CharacterDefinition 当前 schema vs 合并 MVU 后的 schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MvuApplyPreview {
    pub source_character_id: String,
    pub character_name: String,
    pub definition_id: String,
    /// 合并后会被新增的字段（MVU 有、当前 schema 无）
    pub added_fields: Vec<VariableField>,
    /// 合并后会被覆盖的字段（同名，MVU 版本替换当前）
    pub overwritten_fields: Vec<VariableField>,
    /// 不变的字段（同名同值）
    pub unchanged_count: usize,
    /// 合并后的完整 schema（预览最终结果）
    pub merged_schema: Vec<VariableField>,
    /// 是否有任何变化（added + overwritten 都空 = 无变化）
    pub has_changes: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum MvuApplyError {
    #[error("MVU 翻译不存在: {0}")]
    TranslationNotFound(String),
    #[error("Definition 不存在: {0}")]
    DefinitionNotFound(String),
    #[error("合并后无变化")]
    NoChanges,
}

/// 判断两个 VariableField 内容是否等价（key 相同且 label/value_type/default 相同）。
///
/// VariableField 未 derive PartialEq，手动比较关键语义字段。
fn field_semantic_eq(a: &VariableField, b: &VariableField) -> bool {
    a.key == b.key && a.label == b.label && a.value_type == b.value_type && a.default == b.default
}

/// 计算合并预览（纯函数，不写盘）。
///
/// 输入：当前 definition 的 variable_schema + MVU translation 的 variable_schema。
pub fn compute_apply_preview(
    current_schema: &[VariableField],
    mvu_schema: &[VariableField],
    definition_id: &str,
    character_name: &str,
    source_character_id: &str,
) -> MvuApplyPreview {
    let merged = merge_schema(current_schema, mvu_schema);

    // 按 key 建索引
    let current_map: std::collections::BTreeMap<&str, &VariableField> =
        current_schema.iter().map(|f| (f.key.as_str(), f)).collect();

    let mut added_fields = Vec::new();
    let mut overwritten_fields = Vec::new();
    let mut unchanged_count = 0usize;

    for field in &merged {
        match current_map.get(field.key.as_str()) {
            None => {
                // 当前 schema 无此 key → 新增
                added_fields.push(field.clone());
            }
            Some(current_field) => {
                if field_semantic_eq(current_field, field) {
                    unchanged_count += 1;
                } else {
                    overwritten_fields.push(field.clone());
                }
            }
        }
    }

    let has_changes = !added_fields.is_empty() || !overwritten_fields.is_empty();

    MvuApplyPreview {
        source_character_id: source_character_id.to_string(),
        character_name: character_name.to_string(),
        definition_id: definition_id.to_string(),
        added_fields,
        overwritten_fields,
        unchanged_count,
        merged_schema: merged,
        has_changes,
    }
}

/// 把合并后的 schema 应用到一份 definition 副本（纯函数，验证用）。
pub fn apply_schema_to_definition(
    def: &mut CharacterDefinition,
    merged_schema: Vec<VariableField>,
) {
    def.variable_schema = merged_schema;
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::variables::VariableType;

    fn field(key: &str, label: &str, default: i64) -> VariableField {
        VariableField {
            key: key.into(),
            label: label.into(),
            value_type: VariableType::Int,
            default: serde_json::Value::Number(default.into()),
            description: None,
            group: None,
        }
    }

    fn field_str(key: &str, label: &str, default: &str) -> VariableField {
        VariableField {
            key: key.into(),
            label: label.into(),
            value_type: VariableType::String,
            default: serde_json::Value::String(default.into()),
            description: None,
            group: None,
        }
    }

    // ── compute_apply_preview 测试 ─────────────────────────────────────────

    #[test]
    fn preview_mvu_all_new_fields() {
        let current = vec![field("hp", "生命值", 100)];
        let mvu = vec![
            field("hp", "生命值", 100),
            field("mp", "魔法值", 50),
            field("atk", "攻击力", 10),
        ];

        let preview = compute_apply_preview(&current, &mvu, "def-1", "角色A", "src-1");

        assert!(preview.has_changes);
        assert_eq!(preview.added_fields.len(), 2); // mp, atk
        assert!(preview.overwritten_fields.is_empty());
        assert_eq!(preview.unchanged_count, 1); // hp 不变
    }

    #[test]
    fn preview_mvu_overwrite_existing_field() {
        let current = vec![field("hp", "生命值", 100), field("mp", "魔法值", 30)];
        let mvu = vec![field("hp", "生命值", 200), field("mp", "魔法值", 30)];

        let preview = compute_apply_preview(&current, &mvu, "def-1", "角色A", "src-1");

        assert!(preview.has_changes);
        assert!(preview.added_fields.is_empty());
        assert_eq!(preview.overwritten_fields.len(), 1); // hp default 100→200
        assert_eq!(preview.overwritten_fields[0].key, "hp");
        assert_eq!(preview.unchanged_count, 1); // mp 不变
    }

    #[test]
    fn preview_no_changes() {
        let current = vec![field("hp", "生命值", 100), field("mp", "魔法值", 50)];
        let mvu = vec![field("hp", "生命值", 100), field("mp", "魔法值", 50)];

        let preview = compute_apply_preview(&current, &mvu, "def-1", "角色A", "src-1");

        assert!(!preview.has_changes);
        assert!(preview.added_fields.is_empty());
        assert!(preview.overwritten_fields.is_empty());
        assert_eq!(preview.unchanged_count, 2);
    }

    #[test]
    fn preview_mixed_added_overwritten_unchanged() {
        let current = vec![
            field("hp", "生命值", 100),
            field("mp", "魔法值", 30),
            field("atk", "攻击力", 10),
        ];
        let mvu = vec![
            field("hp", "HP", 100),    // label 不同 → overwritten
            field("mp", "魔法值", 30), // 完全相同 → unchanged
            field("def", "防御力", 5), // 新增 → added
        ];

        let preview = compute_apply_preview(&current, &mvu, "def-1", "角色A", "src-1");

        assert!(preview.has_changes);
        assert_eq!(preview.added_fields.len(), 1); // def
        assert_eq!(preview.added_fields[0].key, "def");
        assert_eq!(preview.overwritten_fields.len(), 1); // hp
        assert_eq!(preview.overwritten_fields[0].key, "hp");
        // unchanged: mp + atk（atk 不在 mvu 里，merge_schema 保留 base 独有的，仍算 unchanged）
        assert_eq!(preview.unchanged_count, 2);
        // merged 包含 hp, mp, atk, def 共 4 个
        assert_eq!(preview.merged_schema.len(), 4);
    }

    #[test]
    fn preview_empty_mvu() {
        let current = vec![field("hp", "生命值", 100)];
        let mvu: Vec<VariableField> = vec![];

        let preview = compute_apply_preview(&current, &mvu, "def-1", "角色A", "src-1");

        assert!(!preview.has_changes);
        assert!(preview.added_fields.is_empty());
        assert!(preview.overwritten_fields.is_empty());
        assert_eq!(preview.unchanged_count, 1);
    }

    #[test]
    fn preview_empty_current() {
        let current: Vec<VariableField> = vec![];
        let mvu = vec![field("hp", "生命值", 100)];

        let preview = compute_apply_preview(&current, &mvu, "def-1", "角色A", "src-1");

        assert!(preview.has_changes);
        assert_eq!(preview.added_fields.len(), 1);
        assert!(preview.overwritten_fields.is_empty());
        assert_eq!(preview.unchanged_count, 0);
    }

    // ── apply_schema_to_definition 测试 ────────────────────────────────────

    #[test]
    fn apply_replaces_variable_schema() {
        let mut def = CharacterDefinition {
            id: storyforge_domain::Id::from_str("def-1"),
            card_id: storyforge_domain::Id::from_str("card-1"),
            name: "角色A".into(),
            persona_prompt: "性格温柔".into(),
            behavior_rules: "不做坏事".into(),
            base_backstory: vec!["背景1".into()],
            group: None,
            role_type: storyforge_domain::character::RoleType::Supporting,
            variable_schema: vec![field("hp", "生命值", 100)],
        };

        let merged = vec![field("hp", "生命值", 200), field("mp", "魔法值", 50)];
        apply_schema_to_definition(&mut def, merged);

        assert_eq!(def.variable_schema.len(), 2);
        assert_eq!(def.variable_schema[0].key, "hp");
        assert_eq!(def.variable_schema[0].default, serde_json::json!(200));
        assert_eq!(def.variable_schema[1].key, "mp");
    }

    #[test]
    fn apply_preserves_other_fields() {
        let mut def = CharacterDefinition {
            id: storyforge_domain::Id::from_str("def-1"),
            card_id: storyforge_domain::Id::from_str("card-1"),
            name: "角色A".into(),
            persona_prompt: "性格温柔".into(),
            behavior_rules: "不做坏事".into(),
            base_backstory: vec!["背景1".into()],
            group: Some("主角团".into()),
            role_type: storyforge_domain::character::RoleType::Protagonist,
            variable_schema: vec![field("hp", "生命值", 100)],
        };

        let merged = vec![field("hp", "生命值", 200)];
        apply_schema_to_definition(&mut def, merged);

        assert_eq!(def.name, "角色A");
        assert_eq!(def.persona_prompt, "性格温柔");
        assert_eq!(def.behavior_rules, "不做坏事");
        assert_eq!(def.base_backstory, vec!["背景1"]);
        assert_eq!(def.group, Some("主角团".into()));
        assert_eq!(
            def.role_type,
            storyforge_domain::character::RoleType::Protagonist
        );
    }

    #[test]
    fn preview_string_type_field_change() {
        let current = vec![field_str("title", "称号", "新手")];
        let mvu = vec![field_str("title", "称号", "勇者")];

        let preview = compute_apply_preview(&current, &mvu, "def-1", "角色A", "src-1");

        assert!(preview.has_changes);
        assert_eq!(preview.overwritten_fields.len(), 1);
        assert_eq!(
            preview.overwritten_fields[0].default,
            serde_json::json!("勇者")
        );
    }
}
