//! 变量层级体系（对应设计 §23 / INTENT D47-D48）
//!
//! 三级变量：CharacterDefinition.variable_schema（卡级定义）→
//!           CharacterInstance.variables（角色级当前值）→
//!           Campaign.variables（全局级，含 story_clock）→
//!           聚合成 active_variables 注入用（每轮覆盖，用完即弃）。
//!
//! 参考 MVU 的 initvar + stat_data 机制：卡定义 schema + 默认值，实例只存值。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ─── 变量类型 ──────────────────────────────────────────────────────────────

/// 变量字段类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VariableType {
    Int,
    Float,
    String,
    Bool,
    Json,
}

// ─── Schema 层（卡级定义，来自基础表 + 卡 initvar 扩展）──────────────────

/// 一个变量字段的定义（schema）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableField {
    /// 字段键名，如 "hp"
    pub key: String,
    /// UI 显示名，如 "生命值"
    pub label: String,
    pub value_type: VariableType,
    /// 默认值（实例初始化时拷贝）
    pub default: serde_json::Value,
    /// 说明（高玩模式 UI 显示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// UI 分组（"状态"/"关系"，用于折叠展示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl VariableField {
    fn int(key: &str, label: &str, default: i64, group: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value_type: VariableType::Int,
            default: serde_json::Value::Number(default.into()),
            description: None,
            group: Some(group.into()),
        }
    }

    fn string(key: &str, label: &str, default: &str, group: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value_type: VariableType::String,
            default: serde_json::Value::String(default.into()),
            description: None,
            group: Some(group.into()),
        }
    }

    fn json(key: &str, label: &str, default: serde_json::Value, group: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value_type: VariableType::Json,
            default,
            description: None,
            group: Some(group.into()),
        }
    }
}

/// 基础变量表（所有角色实例默认带，D48）
///
/// 参考 MVU 变量构建方式：导入角色时拿这份 schema 初始化实例的变量。
/// MVU 卡的 initvar 会覆盖/扩展此表；用户可在高玩模式加自定义字段。
pub fn default_character_variables() -> Vec<VariableField> {
    vec![
        VariableField::int("hp", "生命值", 100, "状态"),
        VariableField::int("mp", "体力/精力", 100, "状态"),
        VariableField::string("state", "状态", "正常", "状态"),
        VariableField::string("location", "位置", "", "状态"),
        VariableField::string("mood", "情绪", "平静", "状态"),
        VariableField::string(
            "relationship_to_player",
            "与玩家关系",
            "陌生",
            "关系",
        ),
        VariableField::json(
            "inventory",
            "物品",
            serde_json::Value::Array(vec![]),
            "状态",
        ),
    ]
}

/// Campaign 级基础变量 schema（全局状态，含 story_clock）
pub fn default_campaign_variables() -> Vec<VariableField> {
    vec![
        VariableField::string("story_clock", "故事时间", "第1天", "全局"),
        VariableField::string("weather", "天气", "晴", "全局"),
        VariableField::string("world_state", "大势", "和平", "全局"),
    ]
}

// ─── 实例值层（运行时存当前值）────────────────────────────────────────────

/// 一个变量的当前值（实例层）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableValue {
    pub key: String,
    pub value: serde_json::Value,
    /// 哪一轮更新的（调试/回溯用）
    pub last_updated_turn: u32,
}

impl VariableValue {
    pub fn new(key: impl Into<String>, value: serde_json::Value, turn: u32) -> Self {
        Self {
            key: key.into(),
            value,
            last_updated_turn: turn,
        }
    }
}

/// 用 schema 初始化实例的变量值列表（拷贝默认值）
pub fn init_values_from_schema(schema: &[VariableField], turn: u32) -> Vec<VariableValue> {
    schema
        .iter()
        .map(|f| VariableValue::new(f.key.clone(), f.default.clone(), turn))
        .collect()
}

/// 合并两份 schema（基础表 + 卡自定义/MVU initvar 扩展）。
///
/// 同 key 时后者覆盖前者的 label/type/default/description/group（卡级优先）。
pub fn merge_schema(base: &[VariableField], extra: &[VariableField]) -> Vec<VariableField> {
    let mut map: BTreeMap<String, VariableField> = BTreeMap::new();
    for f in base {
        map.insert(f.key.clone(), f.clone());
    }
    for f in extra {
        map.insert(f.key.clone(), f.clone());
    }
    map.into_values().collect()
}

// ─── 注入渲染（cache 友好：拼成文本放末尾 user message）──────────────────

/// 渲染变量为注入文本（供导演/编剧/子 Agent 末尾 user message 使用）
///
/// 输出形如：
/// ```text
/// 【当前世界状态】
/// 故事时间：第 47 天
/// 天气：雨
///
/// 【林医生】生命 80 / 状态 受伤 / 位置 急诊室 / 情绪 紧张
/// 【陈警官】生命 100 / 状态 警觉 / 位置 现场外
/// ```
pub fn render_variables_for_injection(
    campaign_vars: &[VariableValue],
    characters: &[(&str, &[VariableValue])], // (角色名, 该角色的变量)
) -> String {
    let mut out = String::new();

    // 全局变量（story_clock 等单独提出来更显眼）
    if !campaign_vars.is_empty() {
        out.push_str("【当前世界状态】\n");
        // story_clock / weather / world_state 优先展示
        let priority = ["story_clock", "weather", "world_state"];
        let mut shown = std::collections::HashSet::new();
        for key in &priority {
            if let Some(v) = campaign_vars.iter().find(|v| v.key == *key) {
                out.push_str(&format!("{}：{}\n", pretty_label(key), value_to_str(&v.value)));
                shown.insert(*key);
            }
        }
        for v in campaign_vars {
            if shown.contains(v.key.as_str()) {
                continue;
            }
            out.push_str(&format!("{}：{}\n", v.key, value_to_str(&v.value)));
        }
        out.push('\n');
    }

    // 在场角色状态（紧凑一行）
    if !characters.is_empty() {
        out.push_str("【在场角色状态】\n");
        for (name, vars) in characters {
            let hp = get_var(vars, "hp").map(value_to_str).unwrap_or_else(|| "-".into());
            let state = get_var(vars, "state").map(value_to_str).unwrap_or_else(|| "-".into());
            let loc = get_var(vars, "location").map(value_to_str).unwrap_or_else(|| "-".into());
            let mood = get_var(vars, "mood").map(value_to_str).unwrap_or_else(|| "-".into());
            out.push_str(&format!(
                "{name}：生命 {hp} / 状态 {state} / 位置 {loc} / 情绪 {mood}\n"
            ));
        }
    }

    out
}

fn get_var<'a>(vars: &'a [VariableValue], key: &str) -> Option<&'a serde_json::Value> {
    vars.iter().find(|v| v.key == key).map(|v| &v.value)
}

fn value_to_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "无".into(),
        other => other.to_string(),
    }
}

fn pretty_label(key: &str) -> &'static str {
    match key {
        "story_clock" => "故事时间",
        "weather" => "天气",
        "world_state" => "大势",
        _ => "状态",
    }
}

// ─── 测试 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_character_variables_has_basics() {
        let schema = default_character_variables();
        let keys: Vec<&str> = schema.iter().map(|f| f.key.as_str()).collect();
        assert!(keys.contains(&"hp"));
        assert!(keys.contains(&"mp"));
        assert!(keys.contains(&"state"));
        assert!(keys.contains(&"location"));
        assert!(keys.contains(&"mood"));
        assert!(keys.contains(&"relationship_to_player"));
        assert!(keys.contains(&"inventory"));
    }

    #[test]
    fn test_default_campaign_variables_has_clock() {
        let schema = default_campaign_variables();
        assert!(schema.iter().any(|f| f.key == "story_clock"));
        assert!(schema.iter().any(|f| f.key == "weather"));
    }

    #[test]
    fn test_init_values_from_schema_copies_defaults() {
        let schema = default_character_variables();
        let values = init_values_from_schema(&schema, 0);
        // 数量一致
        assert_eq!(values.len(), schema.len());
        // hp 默认应是 100
        let hp = values.iter().find(|v| v.key == "hp").unwrap();
        assert_eq!(hp.value.as_i64(), Some(100));
    }

    #[test]
    fn test_merge_schema_extra_overrides_base() {
        let base = default_character_variables();
        // 卡自定义：hp 默认改成 200
        let extra = vec![VariableField::int("hp", "生命值", 200, "状态")];
        let merged = merge_schema(&base, &extra);
        let hp = merged.iter().find(|f| f.key == "hp").unwrap();
        assert_eq!(hp.default.as_i64(), Some(200));
        // 其他字段保留
        assert!(merged.iter().any(|f| f.key == "mp"));
    }

    #[test]
    fn test_merge_schema_extra_adds_new_field() {
        let base = default_character_variables();
        let extra = vec![VariableField::int("fatigue", "疲劳度", 0, "状态")];
        let merged = merge_schema(&base, &extra);
        assert!(merged.iter().any(|f| f.key == "fatigue"));
        // 基础字段不丢
        assert!(merged.iter().any(|f| f.key == "hp"));
    }

    #[test]
    fn test_render_variables_for_injection() {
        let campaign = vec![
            VariableValue::new("story_clock", serde_json::json!("第 47 天"), 5),
            VariableValue::new("weather", serde_json::json!("雨"), 5),
        ];
        let lin = vec![
            VariableValue::new("hp", serde_json::json!(80), 5),
            VariableValue::new("state", serde_json::json!("受伤"), 5),
            VariableValue::new("location", serde_json::json!("急诊室"), 5),
            VariableValue::new("mood", serde_json::json!("紧张"), 5),
        ];
        let chars: Vec<(&str, &[VariableValue])> = vec![("林医生", &lin)];

        let out = render_variables_for_injection(&campaign, &chars);

        // 关键内容都应出现
        assert!(out.contains("第 47 天"), "应有故事时间");
        assert!(out.contains("雨"), "应有天气");
        assert!(out.contains("林医生"));
        assert!(out.contains("生命 80"));
        assert!(out.contains("状态 受伤"));
    }
}
