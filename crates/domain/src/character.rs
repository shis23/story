use serde::{Deserialize, Serialize};

use crate::world_info::WorldInfoBook;
use crate::{Id, Source};

/// 角色卡（内部表示，对齐 ST V3 spec 但重组为领域模型）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Character {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub first_mes: String,
    pub mes_example: String,
    pub system_prompt: String,
    pub post_history_instructions: String,
    pub tags: Vec<String>,
    pub creator: String,
    pub character_version: String,
    pub alternate_greetings: Vec<String>,
    /// 内嵌世界书（character_book，从 ST 卡解析出来）
    pub embedded_world_info: Option<WorldInfoBook>,
    /// ST extensions 字段（保留原始 JSON，不丢失任何信息）
    pub extensions: serde_json::Value,
    /// 可渲染资产（HTML/JS/CSS，用于 iframe 渲染）
    pub renderable_assets: Option<RenderableAssets>,
    /// 来源
    pub source: Source,
    /// 原始 ST spec 版本（"2.0" / "3.0"）
    pub spec_version: String,
    /// 原始卡的完整 JSON（保留，便于导出回 ST 格式）
    pub raw_card_json: serde_json::Value,
}

/// 角色卡中的可渲染前端资产（用于 iframe 沙箱渲染）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderableAssets {
    /// HTML 代码
    pub html: Option<String>,
    /// CSS 代码
    pub css: Option<String>,
    /// JS 代码
    pub js: Option<String>,
    /// 资产名（用于日志/调试）
    pub name: String,
}

/// ST V3 卡的 JSON 结构（用于反序列化导入的 JSON）
#[derive(Debug, Serialize, Deserialize)]
pub struct StCharacterCard {
    #[allow(dead_code)]
    pub spec: Option<String>,
    pub spec_version: Option<String>,
    pub data: StCharacterData,
}

/// ST 卡 data 字段
#[derive(Debug, Serialize, Deserialize)]
pub struct StCharacterData {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub personality: String,
    #[serde(default)]
    pub scenario: String,
    #[serde(default)]
    pub first_mes: String,
    #[serde(default)]
    pub mes_example: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub post_history_instructions: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub character_version: String,
    #[serde(default)]
    pub alternate_greetings: Vec<String>,
    #[serde(default)]
    pub extensions: serde_json::Value,
    /// 内嵌世界书（ST 字段名 character_book）
    #[serde(default)]
    pub character_book: Option<StWorldInfoBook>,
    /// ST data 顶层未知字段，作为 raw round-trip 保底保留。
    #[serde(default, flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

/// ST 内嵌世界书结构（导入时用，转换后丢弃）
#[derive(Debug, Serialize, Deserialize)]
pub struct StWorldInfoBook {
    #[serde(default)]
    pub entries: Vec<StWorldInfoEntry>,
    #[serde(default, flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

/// ST 世界书条目（导入时用）
#[derive(Debug, Serialize, Deserialize)]
pub struct StWorldInfoEntry {
    pub id: Option<i32>,
    #[serde(default)]
    pub keys: Vec<String>,
    /// Older ST exports use singular `key` instead of `keys`.
    #[serde(default, rename = "key", skip_serializing)]
    pub key_alias: Option<serde_json::Value>,
    #[serde(default)]
    pub secondary_keys: Option<Vec<String>>,
    /// Older ST exports use `keysecondary` (string or array) instead of `secondary_keys`.
    #[serde(default, rename = "keysecondary", skip_serializing)]
    pub keysecondary_alias: Option<serde_json::Value>,
    pub content: Option<String>,
    #[serde(default)]
    pub constant: bool,
    #[serde(default)]
    pub selective: bool,
    #[serde(default)]
    pub selective_logic: Option<i32>,
    /// ST 新版用字符串（"before_char" 等），旧版用数字，都要兼容
    #[serde(default)]
    pub position: Option<serde_json::Value>,
    #[serde(default)]
    pub disable: Option<bool>,
    #[serde(default)]
    pub order: Option<i32>,
    #[serde(default)]
    pub depth: Option<i32>,
    #[serde(default)]
    pub extensions: serde_json::Value,
}

impl StWorldInfoEntry {
    /// 解析 position 为数字（兼容字符串和数字两种格式）
    pub fn position_as_i32(&self) -> i32 {
        match &self.position {
            Some(serde_json::Value::Number(n)) => n.as_i64().unwrap_or(0) as i32,
            Some(serde_json::Value::String(s)) => match s.as_str() {
                "before_char" => 0,
                "after_char" => 1,
                "in_roleplay" => 2,
                "before_examples" => 3,
                "after_examples" => 4,
                _ => 0,
            },
            _ => 0,
        }
    }

    /// Resolve primary keys, accepting both `keys` and legacy `key`.
    pub fn resolved_keys(&self) -> Vec<String> {
        if !self.keys.is_empty() {
            return self.keys.clone();
        }
        parse_string_list_value(self.key_alias.as_ref())
    }

    /// Resolve secondary keys, accepting both `secondary_keys` and legacy `keysecondary`.
    pub fn resolved_secondary_keys(&self) -> Vec<String> {
        if let Some(keys) = &self.secondary_keys
            && !keys.is_empty()
        {
            return keys.clone();
        }
        parse_string_list_value(self.keysecondary_alias.as_ref())
    }
}

fn parse_string_list_value(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        Some(serde_json::Value::String(s)) => s
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// 角色卡元数据（列表展示用，不含大字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterMeta {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub creator: String,
    pub source: Source,
    pub has_world_info: bool,
    pub has_renderable_assets: bool,
}

impl Character {
    /// 提取元数据（列表展示用）
    pub fn to_meta(&self) -> CharacterMeta {
        CharacterMeta {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            tags: self.tags.clone(),
            creator: self.creator.clone(),
            source: self.source.clone(),
            has_world_info: self.embedded_world_info.is_some(),
            has_renderable_assets: self.renderable_assets.is_some(),
        }
    }

    /// 检查是否有可渲染的前端资产（HTML/JS/CSS）
    pub fn has_renderable_assets(&self) -> bool {
        self.renderable_assets.is_some()
            || self.extensions.get("assets").is_some_and(|v| !v.is_null())
    }

    /// Parses card-scoped ST regex scripts from `data.extensions.regex_scripts`.
    pub fn scoped_regex_scripts(&self) -> Vec<crate::preset::RegexScript> {
        crate::preset::extract_regex_scripts_with_source(
            &self.extensions,
            crate::preset::RegexScriptSource::Scoped,
        )
    }

    /// 从 ST 卡 JSON 解析为领域模型
    pub fn from_st_card(card: StCharacterCard) -> Self {
        let raw_json =
            serde_json::to_value(&card.data).expect("ST character data should serialize to JSON");
        let spec_version = card.spec_version.unwrap_or_else(|| "2.0".into());

        // 提取可渲染资产
        let renderable_assets = extract_renderable_assets(&card.data);

        // 提取内嵌世界书
        let embedded_world_info = card
            .data
            .character_book
            .map(crate::world_info::WorldInfoBook::from_st);

        Self {
            id: Id::new(),
            name: card.data.name,
            description: card.data.description,
            personality: card.data.personality,
            scenario: card.data.scenario,
            first_mes: card.data.first_mes,
            mes_example: card.data.mes_example,
            system_prompt: card.data.system_prompt,
            post_history_instructions: card.data.post_history_instructions,
            tags: card.data.tags,
            creator: card.data.creator,
            character_version: card.data.character_version,
            alternate_greetings: card.data.alternate_greetings,
            embedded_world_info,
            extensions: card.data.extensions,
            renderable_assets,
            source: Source::ImportedFromST,
            spec_version,
            raw_card_json: raw_json,
        }
    }
}

/// 从 Character + 可选 CharacterDefinition 构建 StCharacterData（导出用）
///
/// 策略：优先 raw_card_json round-trip 保底（不丢 ST 扩展字段），
/// 然后用 Character/Definition 的字段覆盖核心字段。
///
/// `character_book` 由调用方传入（共享 lorebook 或内嵌世界书）。
pub fn to_st_data(
    character: &Character,
    definition: Option<&CharacterDefinition>,
    character_book: Option<StWorldInfoBook>,
) -> StCharacterData {
    // 从 raw_card_json 反序列化为 base，保留扩展字段
    let mut data: StCharacterData = if !character.raw_card_json.is_null() {
        serde_json::from_value(character.raw_card_json.clone())
            .unwrap_or_else(|_| empty_st_data(&character.name))
    } else {
        empty_st_data(&character.name)
    };

    // 用 Character 字段覆盖核心字段（保证最新）
    data.name = character.name.clone();
    data.description = character.description.clone();
    data.personality = character.personality.clone();
    data.scenario = character.scenario.clone();
    data.first_mes = character.first_mes.clone();
    data.mes_example = character.mes_example.clone();
    data.system_prompt = character.system_prompt.clone();
    data.post_history_instructions = character.post_history_instructions.clone();
    data.tags = character.tags.clone();
    data.creator = character.creator.clone();
    data.character_version = character.character_version.clone();
    data.alternate_greetings = character.alternate_greetings.clone();

    // Definition 覆盖：persona + behavior 拼合进 description（如有）
    if let Some(def) = definition
        && (!def.persona_prompt.is_empty() || !def.behavior_rules.is_empty())
    {
        let mut parts = Vec::new();
        if !def.persona_prompt.is_empty() {
            parts.push(def.persona_prompt.clone());
        }
        if !def.behavior_rules.is_empty() {
            parts.push(format!("行为规则：{}", def.behavior_rules));
        }
        // 用 definition 的 persona/behavior 丰富 description
        let def_desc = parts.join("\n\n");
        if data.description.is_empty() {
            data.description = def_desc;
        } else {
            data.description = format!("{}\n\n{}", data.description, def_desc);
        }
    }

    // 世界书
    data.character_book = character_book;

    // extensions：保留 raw_card_json 的；只有明确的非空运行时 extensions 才覆盖。
    if is_non_empty_json(&character.extensions) {
        data.extensions = character.extensions.clone();
    }

    data
}

fn is_non_empty_json(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Object(map) => !map.is_empty(),
        _ => true,
    }
}

/// 从 CharacterCard + CharacterDefinition 构建 StCharacterData（Campaign 导出路径）
///
/// 用于无法直接拿到原始 Character 对象的场景（只有 CampaignStore 中的 Card）。
/// 优先用 card.raw_card_json round-trip，definition 字段覆盖核心字段。
pub fn to_st_data_from_card(
    card: &CharacterCard,
    definition: &CharacterDefinition,
    character_book: Option<StWorldInfoBook>,
) -> StCharacterData {
    let mut data: StCharacterData = if !card.raw_card_json.is_null() {
        serde_json::from_value(card.raw_card_json.clone())
            .unwrap_or_else(|_| empty_st_data(&definition.name))
    } else {
        empty_st_data(&definition.name)
    };

    // Definition 字段覆盖
    data.name = definition.name.clone();
    if !definition.persona_prompt.is_empty() || !definition.behavior_rules.is_empty() {
        let mut parts = Vec::new();
        if !definition.persona_prompt.is_empty() {
            parts.push(definition.persona_prompt.clone());
        }
        if !definition.behavior_rules.is_empty() {
            parts.push(format!("行为规则：{}", definition.behavior_rules));
        }
        let def_desc = parts.join("\n\n");
        if data.description.is_empty() {
            data.description = def_desc;
        } else {
            data.description = format!("{}\n\n{}", data.description, def_desc);
        }
    }

    // 世界书
    data.character_book = character_book;

    data
}

/// 构建空的 StCharacterData（fallback）
pub fn empty_st_data(name: &str) -> StCharacterData {
    StCharacterData {
        name: name.to_string(),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: String::new(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        tags: vec![],
        creator: String::new(),
        character_version: String::new(),
        alternate_greetings: vec![],
        extensions: serde_json::json!({}),
        character_book: None,
        extra: Default::default(),
    }
}

/// 从 ST extensions 中提取可渲染资产
fn extract_renderable_assets(data: &StCharacterData) -> Option<RenderableAssets> {
    // ST 的 assets 存在 extensions.assets 或 extensions.character_assets
    let assets = data
        .extensions
        .get("assets")
        .or_else(|| data.extensions.get("character_assets"))?;

    // assets 通常是 { "html": "...", "css": "...", "js": "..." } 或数组
    if let Some(obj) = assets.as_object() {
        let html = obj.get("html").and_then(|v| v.as_str()).map(String::from);
        let css = obj.get("css").and_then(|v| v.as_str()).map(String::from);
        let js = obj.get("js").and_then(|v| v.as_str()).map(String::from);
        if html.is_some() || css.is_some() || js.is_some() {
            return Some(RenderableAssets {
                html,
                css,
                js,
                name: data.name.clone(),
            });
        }
    }

    None
}

// ─── 多角色卡树形模型（D32-D33，对应设计 §17）─────────────────────────────
//
// 一卡多角色：CharacterCard 是卡本体，下挂多个 CharacterDefinition（卡内角色）。
// CharacterDefinition 是「模板」（卡级，全局共享），CharacterInstance 是「实例」（会话级）。
// 角色识别 Agent 导入时产出 Vec<CharacterDefinition>。

use crate::variables::{VariableField, merge_schema};

/// Character extraction state for a multi-character card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterExtractionStatus {
    /// Legacy or not-yet-processed card. Old `cards.json` records deserialize here.
    #[default]
    Unknown,
    /// The extractor successfully produced structured character definitions.
    Extracted,
    /// Extraction failed and the app created a single usable fallback definition.
    Fallback,
}

impl CharacterExtractionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Extracted => "extracted",
            Self::Fallback => "fallback",
        }
    }
}

/// 角色卡本体（一卡多角色的容器）
///
/// 导入 ST 卡后，角色识别 Agent 分析卡内容，产出卡内所有角色的 CharacterDefinition。
/// 注意：这与现有的扁平 `Character`（单角色导入表示）并存——
/// `Character` 是 ST 卡的原始解析结果，`CharacterCard` 是多角色识别后的结构化容器。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterCard {
    pub id: Id,
    /// 卡名（通常等于主角色名）
    pub name: String,
    /// 关联的扁平 Character（保留原始 ST 解析结果，便于回写/导出）
    pub source_character_id: Id,
    /// 卡内角色定义（角色识别 Agent 产出）
    pub character_definitions: Vec<CharacterDefinition>,
    /// 原始 ST 卡 data 字段的 JSON（导出时 round-trip 保底，不丢扩展字段）
    ///
    /// 新数据由导入流程填入；旧数据缺失时 serde 默认 `Value::Null`（向后兼容）。
    #[serde(default)]
    pub raw_card_json: serde_json::Value,
    /// Role extraction status. Missing legacy values remain `unknown` instead of
    /// inferring success from fallback definitions.
    #[serde(default)]
    pub extraction_status: CharacterExtractionStatus,
    /// User-safe extraction note shown in UI. Provider errors are logged, not persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction_message: Option<String>,
}

/// 卡内角色定义（模板，卡级全局共享）
///
/// 由角色识别 Agent 在导入时生成（D33）：语义级分析 description/first_mes/character_book，
/// 识别卡内角色，为每个角色生成 persona/behavior/backstory。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterDefinition {
    pub id: Id,
    /// 属于哪张卡
    pub card_id: Id,
    /// 角色名
    pub name: String,
    /// 人设：性格、说话风格、口头禅、外貌
    pub persona_prompt: String,
    /// 行为规则：决策倾向、绝对不会做的事
    pub behavior_rules: String,
    /// 游戏开始前就知道的事（3-5 条，导入时由识别 Agent 抽取）
    pub base_backstory: Vec<String>,
    /// 附加分组参数："主角团"/"反派"/None（D32）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// 角色类型（常驻/配角/临场基准，D35）
    #[serde(default)]
    pub role_type: RoleType,
    /// 变量 schema（基础表 + 卡 MVU initvar 扩展）
    ///
    /// 实例化时拷贝默认值到 CharacterInstance.variables。
    #[serde(default = "crate::variables::default_character_variables")]
    pub variable_schema: Vec<VariableField>,
}

/// 角色类型（决定是否常驻、是否进向量库）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoleType {
    /// 主角（常驻，进向量库）
    Protagonist,
    /// 配角（常驻，进向量库）
    #[default]
    Supporting,
    /// 临场基准（龙套默认类型，不进向量库；可升级）
    Extra,
}

impl CharacterCard {
    /// 从扁平 Character 初始化（角色识别 Agent 跑之前，先建空容器）
    pub fn from_character(character: &Character) -> Self {
        Self {
            id: Id::new(),
            name: character.name.clone(),
            source_character_id: character.id.clone(),
            character_definitions: vec![],
            raw_card_json: character.raw_card_json.clone(),
            extraction_status: CharacterExtractionStatus::Unknown,
            extraction_message: None,
        }
    }

    pub fn extraction_succeeded(&self) -> bool {
        self.extraction_status == CharacterExtractionStatus::Extracted
    }

    /// Parses card-scoped ST regex scripts from raw `data.extensions.regex_scripts`.
    pub fn scoped_regex_scripts(&self) -> Vec<crate::preset::RegexScript> {
        let Some(extensions) = self.raw_card_json.get("extensions") else {
            return Vec::new();
        };
        crate::preset::extract_regex_scripts_with_source(
            extensions,
            crate::preset::RegexScriptSource::Scoped,
        )
    }

    /// 按名字找角色定义
    pub fn find_definition(&self, name: &str) -> Option<&CharacterDefinition> {
        self.character_definitions.iter().find(|d| d.name == name)
    }
}

impl CharacterDefinition {
    /// 角色识别 Agent 失败时的降级构造（单角色 Protagonist）
    ///
    /// 把整张卡当作单一主角色，persona 取 description+personality，
    /// behavior 留空，backstory 空（导演/玩家后续可补），变量 schema 用基础表合并 MVU 探测结果。
    pub fn fallback_from_character(character: &Character, mvu_schema: &[VariableField]) -> Self {
        let persona = format!(
            "{}\n\n性格：{}",
            character.description.trim(),
            character.personality.trim()
        )
        .trim()
        .to_string();

        let merged_schema =
            merge_schema(&crate::variables::default_character_variables(), mvu_schema);

        Self {
            id: Id::new(),
            card_id: Id::from_str("__pending__"), // 调用方回填
            name: character.name.clone(),
            persona_prompt: persona,
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: merged_schema,
        }
    }

    /// 把卡级 MVU schema 合并进 definition 的 variable_schema（导入后处理用）
    pub fn merge_variable_schema(&mut self, mvu_schema: &[VariableField]) {
        self.variable_schema = merge_schema(&self.variable_schema, mvu_schema);
    }
}

#[cfg(test)]
mod multi_character_tests {
    use super::*;

    #[test]
    fn test_character_card_from_character() {
        let ch = Character {
            id: Id::from_str("src-1"),
            name: "测试卡".into(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions: serde_json::Value::Null,
            renderable_assets: None,
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::Value::Null,
        };
        let card = CharacterCard::from_character(&ch);
        assert_eq!(card.name, "测试卡");
        assert!(card.character_definitions.is_empty());
        assert_eq!(card.extraction_status, CharacterExtractionStatus::Unknown);
        assert!(!card.extraction_succeeded());
        assert!(card.extraction_message.is_none());
    }

    #[test]
    fn test_role_type_default_is_supporting() {
        assert_eq!(RoleType::default(), RoleType::Supporting);
    }

    #[test]
    fn test_find_definition_by_name() {
        let card = CharacterCard {
            id: Id::new(),
            name: "test".into(),
            source_character_id: Id::new(),
            character_definitions: vec![
                CharacterDefinition {
                    id: Id::from_str("d1"),
                    card_id: Id::from_str("c1"),
                    name: "林医生".into(),
                    persona_prompt: String::new(),
                    behavior_rules: String::new(),
                    base_backstory: vec![],
                    group: None,
                    role_type: RoleType::Protagonist,
                    variable_schema: vec![],
                },
                CharacterDefinition {
                    id: Id::from_str("d2"),
                    card_id: Id::from_str("c1"),
                    name: "陈警官".into(),
                    persona_prompt: String::new(),
                    behavior_rules: String::new(),
                    base_backstory: vec![],
                    group: None,
                    role_type: RoleType::Supporting,
                    variable_schema: vec![],
                },
            ],
            raw_card_json: serde_json::Value::Null,
            extraction_status: CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        assert!(card.find_definition("林医生").is_some());
        assert!(card.find_definition("陈警官").is_some());
        assert!(card.find_definition("不存在").is_none());
    }

    #[test]
    fn test_character_card_legacy_json_defaults_extraction_status_unknown() {
        let legacy = serde_json::json!({
            "id": "legacy-card",
            "name": "Legacy Card",
            "source_character_id": "legacy-source",
            "character_definitions": [{
                "id": "legacy-def",
                "card_id": "legacy-card",
                "name": "Legacy Hero",
                "persona_prompt": "kept for compatibility",
                "behavior_rules": "",
                "base_backstory": [],
                "role_type": "protagonist",
                "variable_schema": []
            }],
            "raw_card_json": null
        });

        let card: CharacterCard = serde_json::from_value(legacy).unwrap();

        assert_eq!(card.extraction_status, CharacterExtractionStatus::Unknown);
        assert!(!card.extraction_succeeded());
        assert_eq!(card.character_definitions.len(), 1);
        assert!(card.extraction_message.is_none());
    }

    #[test]
    fn test_fallback_from_character_builds_single_protagonist() {
        let ch = Character {
            id: Id::from_str("src-1"),
            name: "林医生".into(),
            description: "一位外科医生。".into(),
            personality: "冷静、理性".into(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions: serde_json::Value::Null,
            renderable_assets: None,
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::Value::Null,
        };
        let def = CharacterDefinition::fallback_from_character(&ch, &[]);
        assert_eq!(def.name, "林医生");
        assert_eq!(def.role_type, RoleType::Protagonist);
        assert!(def.persona_prompt.contains("外科医生"));
        assert!(def.persona_prompt.contains("冷静、理性"));
        // 默认带基础表
        assert!(def.variable_schema.iter().any(|f| f.key == "hp"));
    }

    #[test]
    fn test_fallback_merges_mvu_schema() {
        let ch = Character {
            id: Id::from_str("src-1"),
            name: "x".into(),
            description: "".into(),
            personality: "".into(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions: serde_json::Value::Null,
            renderable_assets: None,
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::Value::Null,
        };
        let mvu = vec![crate::variables::VariableField::int(
            "fatigue",
            "疲劳度",
            0,
            "状态",
        )];
        let def = CharacterDefinition::fallback_from_character(&ch, &mvu);
        assert!(def.variable_schema.iter().any(|f| f.key == "hp"));
        assert!(def.variable_schema.iter().any(|f| f.key == "fatigue"));
    }

    // ─── to_st_data round-trip 测试 ──────────────────────────────────────────

    #[test]
    fn test_to_st_data_round_trip_preserves_fields() {
        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "3.0",
            "data": {
                "name": "测试角色",
                "description": "一个用于测试的角色",
                "personality": "冷静、理性",
                "scenario": "在未来都市中",
                "first_mes": "你好，我是测试角色。",
                "mes_example": "",
                "system_prompt": "你是一个测试角色。",
                "post_history_instructions": "",
                "tags": ["test", "demo"],
                "creator": "StoryForge",
                "character_version": "1.0",
                "alternate_greetings": ["嗨！", "欢迎。"],
                "extensions": {"custom_key": "custom_value"},
                "character_book": {
                    "entries": [{
                        "id": 1,
                        "keys": ["未来"],
                        "content": "未来都市知识",
                        "constant": true,
                        "position": 0
                    }]
                }
            }
        });
        let card: StCharacterCard = serde_json::from_value(card_json).unwrap();
        let character = Character::from_st_card(card);

        // to_st_data round-trip（不带 definition）
        let exported = to_st_data(&character, None, None);

        assert_eq!(exported.name, "测试角色");
        assert_eq!(exported.description, "一个用于测试的角色");
        assert_eq!(exported.personality, "冷静、理性");
        assert_eq!(exported.first_mes, "你好，我是测试角色。");
        assert_eq!(exported.tags, vec!["test", "demo"]);
        assert_eq!(exported.creator, "StoryForge");
        assert_eq!(exported.alternate_greetings, vec!["嗨！", "欢迎。"]);
        // extensions round-trip
        assert_eq!(exported.extensions["custom_key"], "custom_value");
    }

    #[test]
    fn test_st_data_unknown_fields_round_trip_through_raw_json() {
        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "3.0",
            "data": {
                "name": "边缘字段角色",
                "description": "测试未知字段保真",
                "group_only": true,
                "creator_notes": "ST data 顶层未知字段",
                "custom_nested": {
                    "flag": "keep-me"
                },
                "extensions": {
                    "unknown_plugin": {
                        "state": 42
                    }
                }
            }
        });
        let card: StCharacterCard = serde_json::from_value(card_json).unwrap();
        let character = Character::from_st_card(card);

        assert_eq!(character.raw_card_json["group_only"], true);
        assert_eq!(
            character.raw_card_json["creator_notes"],
            "ST data 顶层未知字段"
        );
        assert_eq!(character.raw_card_json["custom_nested"]["flag"], "keep-me");

        let exported = to_st_data(&character, None, None);
        let exported_json = serde_json::to_value(exported).unwrap();
        assert_eq!(exported_json["group_only"], true);
        assert_eq!(exported_json["creator_notes"], "ST data 顶层未知字段");
        assert_eq!(exported_json["custom_nested"]["flag"], "keep-me");
        assert_eq!(exported_json["extensions"]["unknown_plugin"]["state"], 42);
    }

    #[test]
    fn test_from_st_card_exposes_scoped_regex_scripts() {
        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "3.0",
            "data": {
                "name": "Scoped Regex Card",
                "extensions": {
                    "regex_scripts": [
                        {
                            "id": "scoped-1",
                            "scriptName": "Scoped output cleanup",
                            "findRegex": "<data_block>[\\s\\S]*?</data_block>",
                            "replaceString": "<status>$0</status>",
                            "placement": [2],
                            "disabled": false,
                            "markdownOnly": true,
                            "promptOnly": false,
                            "runOnEdit": true,
                            "substituteRegex": 0,
                            "trimStrings": ["```"],
                            "minDepth": null,
                            "maxDepth": 4
                        }
                    ]
                }
            }
        });

        let card: StCharacterCard = serde_json::from_value(card_json).unwrap();
        let character = Character::from_st_card(card);
        let scripts = character.scoped_regex_scripts();

        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].id, "scoped-1");
        assert_eq!(scripts[0].script_name, "Scoped output cleanup");
        assert_eq!(scripts[0].placement_codes, vec![2]);
        assert_eq!(scripts[0].source, crate::preset::RegexScriptSource::Scoped);
        assert_eq!(scripts[0].placement, crate::preset::RegexPlacement::Output);
        assert_eq!(scripts[0].markdown_only, Some(true));
        assert_eq!(scripts[0].prompt_only, Some(false));
        assert_eq!(scripts[0].run_on_edit, Some(true));
        assert_eq!(scripts[0].substitute_regex, Some(0));
        assert_eq!(scripts[0].trim_strings, vec!["```"]);
        assert_eq!(scripts[0].min_depth, None);
        assert_eq!(scripts[0].max_depth, Some(4));
        assert!(character.extensions.get("regex_scripts").is_some());
    }

    #[test]
    fn test_character_card_exposes_scoped_regex_scripts_from_raw_st_data() {
        let card = CharacterCard {
            id: Id::from_str("card-campaign"),
            name: "Campaign Card".into(),
            source_character_id: Id::from_str("source-campaign"),
            character_definitions: vec![],
            raw_card_json: serde_json::json!({
                "name": "Campaign Card",
                "extensions": {
                    "regex_scripts": [
                        {
                            "id": "campaign-scoped",
                            "scriptName": "Campaign scoped output",
                            "findRegex": "foo",
                            "replaceString": "bar",
                            "placement": [2],
                            "disabled": false
                        }
                    ]
                }
            }),
            extraction_status: CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };

        let scripts = card.scoped_regex_scripts();

        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].id, "campaign-scoped");
        assert_eq!(scripts[0].source, crate::preset::RegexScriptSource::Scoped);
        assert_eq!(scripts[0].placement, crate::preset::RegexPlacement::Output);
    }

    #[test]
    fn test_to_st_data_with_definition_enriches_description() {
        let character = Character {
            id: Id::from_str("src-1"),
            name: "林医生".into(),
            description: "一位外科医生。".into(),
            personality: "冷静".into(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions: serde_json::json!({}),
            renderable_assets: None,
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::Value::Null,
        };
        let def = CharacterDefinition {
            id: Id::from_str("d1"),
            card_id: Id::from_str("c1"),
            name: "林医生".into(),
            persona_prompt: "温柔的外科医生".into(),
            behavior_rules: "先救人后问话".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        };

        let exported = to_st_data(&character, Some(&def), None);
        assert_eq!(exported.name, "林医生");
        // description 应包含原始 + definition 的 persona/behavior
        assert!(exported.description.contains("一位外科医生。"));
        assert!(exported.description.contains("温柔的外科医生"));
        assert!(exported.description.contains("先救人后问话"));
    }

    #[test]
    fn test_to_st_data_with_character_book() {
        let character = Character {
            id: Id::from_str("src-1"),
            name: "test".into(),
            description: String::new(),
            personality: String::new(),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: String::new(),
            character_version: String::new(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions: serde_json::json!({}),
            renderable_assets: None,
            source: Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::Value::Null,
        };
        let book = StWorldInfoBook {
            entries: vec![StWorldInfoEntry {
                id: Some(1),
                keys: vec!["测试".into()],
                key_alias: None,
                secondary_keys: None,
                keysecondary_alias: None,
                content: Some("测试知识".into()),
                constant: true,
                selective: false,
                selective_logic: None,
                position: Some(serde_json::json!(0)),
                disable: None,
                order: None,
                depth: None,
                extensions: serde_json::json!({}),
            }],
            extra: Default::default(),
        };
        let exported = to_st_data(&character, None, Some(book));
        assert!(exported.character_book.is_some());
        let cb = exported.character_book.unwrap();
        assert_eq!(cb.entries.len(), 1);
        assert_eq!(cb.entries[0].keys, vec!["测试"]);
    }
}
