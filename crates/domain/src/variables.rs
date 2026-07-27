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
    pub(crate) fn int(key: &str, label: &str, default: i64, group: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value_type: VariableType::Int,
            default: serde_json::Value::Number(default.into()),
            description: None,
            group: Some(group.into()),
        }
    }

    pub(crate) fn string(key: &str, label: &str, default: &str, group: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value_type: VariableType::String,
            default: serde_json::Value::String(default.into()),
            description: None,
            group: Some(group.into()),
        }
    }

    pub(crate) fn json(key: &str, label: &str, default: serde_json::Value, group: &str) -> Self {
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
        VariableField::string("relationship_to_player", "与玩家关系", "陌生", "关系"),
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

// ─── MVU 变量键记法归一化 ────────────────────────────────────────────────
//
// forge 差分实测（2026-07-26）：模型输出的变量键记法不稳定——同一张卡
// pro 输出斜杠记法（/世界/时间）、flash 输出点记法，模板段有 {角色名} 和
// <角色名> 两种写法，且常带 stat_data. 容器前缀。下游消费端（schema 应用、
// ui_bindings 匹配、变量读写）只认字面键，必须在解析边界统一记法。
// 前端镜像实现：frontend/src/utils/mvuKey.js（normalizeMvuKey）。

/// 归一化 MVU 变量键记法为 canonical 形式：点记法、无 `stat_data.` 前缀、
/// 模板占位符段统一 `{...}`。
///
/// - 斜杠记法 → 点记法：仅当键以 `/` 或 `stat_data/` 开头才转换，
///   避免误伤名字里含 `/` 的普通键（如 `攻/防`）
/// - 反复剥离 `stat_data.` 前缀（ST-MVU 的存储容器，原生变量无此层）
/// - 段级 `<xxx>` → `{xxx}`
/// - 去除首尾空白与空段
pub fn normalize_mvu_key(raw: &str) -> String {
    let trimmed = raw.trim();
    let dotted: String = if trimmed.starts_with('/') || trimmed.starts_with("stat_data/") {
        trimmed.trim_start_matches('/').replace('/', ".")
    } else {
        trimmed.to_string()
    };
    let mut rest = dotted.as_str();
    while let Some(stripped) = rest.strip_prefix("stat_data.") {
        rest = stripped;
    }
    rest.split('.')
        .map(str::trim)
        .filter(|seg| !seg.is_empty())
        .map(|seg| {
            if seg.len() >= 2 && seg.starts_with('<') && seg.ends_with('>') {
                format!("{{{}}}", &seg[1..seg.len() - 1])
            } else {
                seg.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// 对一份 schema 的所有 key 做记法归一，归一后同 key 去重（先到先得）、
/// 空 key 丢弃。用于 LLM 翻译产物入库前和存量产物应用前的边界清洗。
pub fn normalize_schema_keys(fields: Vec<VariableField>) -> Vec<VariableField> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(fields.len());
    for mut field in fields {
        field.key = normalize_mvu_key(&field.key);
        if field.key.is_empty() || !seen.insert(field.key.clone()) {
            continue;
        }
        out.push(field);
    }
    out
}

// ─── MVU initvar 探测（字段级解析，对应设计 §19 / §23.3）─────────────────
//
// P1 阶段只做字段级解析：探测 ST 卡 extensions 里的结构化 stat_data / initvar
// 字段，解析成 Vec<VariableField>。复杂 JS 分析 + WebView 兜底留 P3。
//
// 探测的常见结构（按优先级）：
// 1. extensions.mvu.initvar —— 显式 MVU 插件 initvar 字段（JSON 对象）
// 2. extensions.stat_data —— ST 风格的扁平 stat_data（JSON 对象）
// 3. extensions.variables / extensions.depth_prompt.variables —— 其他变量插件
// 找不到返回空 vec（保守策略，不报错；调用方用基础表兜底）。

/// 从 ST 卡的 extensions 探测 MVU / stat_data 字段，解析成变量 schema。
///
/// 输入是 `Character.extensions`（裸 serde_json::Value）。找不到结构化字段返回空 vec。
pub fn extract_mvu_schema_from_extensions(extensions: &serde_json::Value) -> Vec<VariableField> {
    // 候选路径（按优先级），任一命中即返回
    let candidates: &[&str] = &[
        // ① MVU 插件 initvar（最显式）
        "mvu.initvar",
        // ② ST 风格 stat_data
        "stat_data",
        // ③ 通用 variables 插件
        "variables",
        // ④ depth_prompt 内嵌变量
        "depth_prompt.variables",
    ];

    for path in candidates {
        if let Some(serde_json::Value::Object(map)) = pick_nested(extensions, path) {
            let fields = parse_variable_objects(map);
            if !fields.is_empty() {
                return fields;
            }
        }
    }
    vec![]
}

/// 从卡扩展中读取明确标记为 Campaign 作用域的变量 schema。
///
/// 这里只接受显式全局路径，绝不把普通 `stat_data` / `mvu.initvar` 自动提升为
/// Campaign 变量，避免把角色生命值、好感度等字段错误地共享给整局。
pub fn extract_campaign_variable_schema_from_extensions(
    extensions: &serde_json::Value,
) -> Vec<VariableField> {
    let candidates: &[&str] = &["storyforge.campaign_variables", "campaign_variables"];

    for path in candidates {
        if let Some(serde_json::Value::Object(map)) = pick_nested(extensions, path) {
            let fields = normalize_schema_keys(parse_variable_objects(map));
            if !fields.is_empty() {
                return fields;
            }
        }
    }
    vec![]
}

/// 按点分路径取嵌套字段（extensions.mvu.initvar → 取 obj["mvu"]["initvar"]）
fn pick_nested<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = root;
    for key in path.split('.') {
        current = current.get(key)?;
    }
    Some(current)
}

/// 把 stat_data / initvar 对象解析成 Vec<VariableField>
///
/// 兼容两种 MVU 写法：
/// - 标量值：`"hp": 100` → label 推断为 "hp"，类型按值推断
/// - 完整对象：`"hp": {"label": "生命值", "type": "int", "default": 100}`
fn parse_variable_objects(map: &serde_json::Map<String, serde_json::Value>) -> Vec<VariableField> {
    map.iter()
        .filter_map(|(key, val)| match val {
            // 完整字段定义对象
            serde_json::Value::Object(o) => {
                let label = o
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or(key)
                    .to_string();
                let type_str = o.get("type").and_then(|v| v.as_str()).unwrap_or("string");
                let default = o.get("default").cloned().unwrap_or(serde_json::Value::Null);
                let value_type = parse_type(type_str, &default);
                let description = o
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let group = o.get("group").and_then(|v| v.as_str()).map(String::from);
                Some(VariableField {
                    key: key.clone(),
                    label,
                    value_type,
                    default,
                    description,
                    group,
                })
            }
            // 标量值（直接当默认值）
            serde_json::Value::Bool(_) | serde_json::Value::Number(_) => Some(VariableField {
                key: key.clone(),
                label: key.clone(),
                value_type: infer_scalar_type(val),
                default: val.clone(),
                description: None,
                group: None,
            }),
            serde_json::Value::String(s) => Some(VariableField {
                key: key.clone(),
                label: key.clone(),
                value_type: VariableType::String,
                default: serde_json::Value::String(s.clone()),
                description: None,
                group: None,
            }),
            // null / array / 其他形态跳过（保守，宁缺勿错）
            _ => None,
        })
        .collect()
}

/// 根据 type 字符串 + 默认值推断 VariableType
fn parse_type(type_str: &str, default: &serde_json::Value) -> VariableType {
    match type_str.to_lowercase().as_str() {
        "int" | "integer" | "number" if default.is_i64() => VariableType::Int,
        "int" | "integer" => VariableType::Int,
        "float" | "double" | "number" => VariableType::Float,
        "bool" | "boolean" => VariableType::Bool,
        "json" | "object" | "array" => VariableType::Json,
        _ => VariableType::String,
    }
}

fn infer_scalar_type(val: &serde_json::Value) -> VariableType {
    match val {
        serde_json::Value::Bool(_) => VariableType::Bool,
        serde_json::Value::Number(n) if n.is_i64() => VariableType::Int,
        serde_json::Value::Number(_) => VariableType::Float,
        _ => VariableType::String,
    }
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
                out.push_str(&format!(
                    "{}：{}\n",
                    pretty_label(key),
                    value_to_str(&v.value)
                ));
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
            let hp = get_var(vars, "hp")
                .map(value_to_str)
                .unwrap_or_else(|| "-".into());
            let state = get_var(vars, "state")
                .map(value_to_str)
                .unwrap_or_else(|| "-".into());
            let loc = get_var(vars, "location")
                .map(value_to_str)
                .unwrap_or_else(|| "-".into());
            let mood = get_var(vars, "mood")
                .map(value_to_str)
                .unwrap_or_else(|| "-".into());
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

    // ─── MVU 探测测试 ──────────────────────────────────────────────────────

    #[test]
    fn test_extract_mvu_from_explicit_initvar() {
        // 显式 MVU 插件 initvar（带完整字段定义）
        let ext = serde_json::json!({
            "mvu": {
                "initvar": {
                    "hp": {"label": "生命值", "type": "int", "default": 200},
                    "sanity": {"label": "理智", "type": "int", "default": 50}
                }
            }
        });
        let schema = extract_mvu_schema_from_extensions(&ext);
        assert_eq!(schema.len(), 2);
        let hp = schema.iter().find(|f| f.key == "hp").unwrap();
        assert_eq!(hp.default.as_i64(), Some(200));
        assert_eq!(hp.value_type, VariableType::Int);
    }

    #[test]
    fn test_extract_mvu_from_flat_stat_data() {
        // ST 风格扁平 stat_data（标量值）
        let ext = serde_json::json!({
            "stat_data": {
                "money": 1000,
                "day": 1,
                "location": "家"
            }
        });
        let schema = extract_mvu_schema_from_extensions(&ext);
        assert_eq!(schema.len(), 3);
        let money = schema.iter().find(|f| f.key == "money").unwrap();
        assert_eq!(money.default.as_i64(), Some(1000));
        assert_eq!(money.value_type, VariableType::Int);
        let loc = schema.iter().find(|f| f.key == "location").unwrap();
        assert_eq!(loc.value_type, VariableType::String);
    }

    #[test]
    fn test_extract_mvu_empty_when_no_structure() {
        // 无 stat_data 结构（普通卡）→ 返回空 vec
        let ext = serde_json::json!({"depth_prompt": {"prompt": "无关字段"}});
        let schema = extract_mvu_schema_from_extensions(&ext);
        assert!(schema.is_empty());
    }

    #[test]
    fn test_extract_campaign_variables_only_from_explicit_global_scope() {
        let ext = serde_json::json!({
            "storyforge": {
                "campaign_variables": {
                    "faction_tension": {
                        "label": "阵营紧张度",
                        "type": "int",
                        "default": 12,
                        "description": "各阵营爆发公开冲突的风险"
                    }
                }
            },
            "stat_data": {
                "hp": 200
            }
        });

        let schema = extract_campaign_variable_schema_from_extensions(&ext);

        assert_eq!(schema.len(), 1);
        assert_eq!(schema[0].key, "faction_tension");
        assert_eq!(schema[0].label, "阵营紧张度");
        assert_eq!(schema[0].default, serde_json::json!(12));
        assert!(
            !schema.iter().any(|field| field.key == "hp"),
            "普通 MVU stat_data 不能被静默提升为 Campaign 全局变量"
        );
    }

    #[test]
    fn test_extract_campaign_variables_accepts_top_level_explicit_alias() {
        let ext = serde_json::json!({
            "campaign_variables": {
                "season": "春"
            }
        });

        let schema = extract_campaign_variable_schema_from_extensions(&ext);

        assert_eq!(schema.len(), 1);
        assert_eq!(schema[0].key, "season");
        assert_eq!(schema[0].value_type, VariableType::String);
        assert_eq!(schema[0].default, serde_json::json!("春"));
    }

    // ─── 键记法归一化测试 ──────────────────────────────────────────────────

    #[test]
    fn test_normalize_mvu_key_notations() {
        // 点记法（canonical）原样保留
        assert_eq!(normalize_mvu_key("主角.属性.力量"), "主角.属性.力量");
        // 斜杠记法（pro 实测输出）→ 点记法
        assert_eq!(normalize_mvu_key("/世界/时间"), "世界.时间");
        assert_eq!(normalize_mvu_key("/主角/属性/力量"), "主角.属性.力量");
        // stat_data 前缀剥离（点/斜杠两种形态）
        assert_eq!(normalize_mvu_key("stat_data.主角.好感度"), "主角.好感度");
        assert_eq!(normalize_mvu_key("stat_data/主角/好感度"), "主角.好感度");
        // 模板占位符段统一花括号
        assert_eq!(
            normalize_mvu_key("女性角色.<角色名>.好感度"),
            "女性角色.{角色名}.好感度"
        );
        assert_eq!(
            normalize_mvu_key("女性角色.{角色名}.好感度"),
            "女性角色.{角色名}.好感度"
        );
        // 首尾空白与空段清理
        assert_eq!(normalize_mvu_key("  hp  "), "hp");
        assert_eq!(normalize_mvu_key("主角..hp"), "主角.hp");
        // 不以 / 开头的含斜杠键不转换（可能是普通名字）
        assert_eq!(normalize_mvu_key("攻/防"), "攻/防");
        // 恰为 stat_data 本身不剥成空
        assert_eq!(normalize_mvu_key("stat_data"), "stat_data");
    }

    #[test]
    fn test_normalize_schema_keys_dedup_and_drop_empty() {
        let fields = vec![
            VariableField {
                key: "stat_data.hp".into(),
                label: "生命".into(),
                value_type: VariableType::Int,
                default: serde_json::json!(100),
                description: None,
                group: None,
            },
            VariableField {
                key: "/hp".into(), // 归一后与上一条同 key → 去重，先到先得
                label: "重复".into(),
                value_type: VariableType::Int,
                default: serde_json::json!(0),
                description: None,
                group: None,
            },
            VariableField {
                key: "   ".into(), // 归一后为空 → 丢弃
                label: "空".into(),
                value_type: VariableType::String,
                default: serde_json::json!(""),
                description: None,
                group: None,
            },
        ];
        let out = normalize_schema_keys(fields);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "hp");
        assert_eq!(out[0].label, "生命");
    }

    #[test]
    fn test_extract_mvu_merge_with_defaults() {
        // MVU 探测结果应能正确和基础表合并（hp 被覆盖、新字段追加）
        let ext = serde_json::json!({"stat_data": {"hp": 200, "fatigue": 0}});
        let mvu = extract_mvu_schema_from_extensions(&ext);
        let merged = merge_schema(&default_character_variables(), &mvu);
        let hp = merged.iter().find(|f| f.key == "hp").unwrap();
        assert_eq!(hp.default.as_i64(), Some(200), "hp 应被 MVU 覆盖");
        assert!(merged.iter().any(|f| f.key == "fatigue"), "应有新增字段");
        assert!(merged.iter().any(|f| f.key == "mp"), "基础字段不丢");
    }
}
