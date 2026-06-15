use serde::{Deserialize, Serialize};

use crate::{Id, Source};
use crate::world_info::WorldInfoBook;

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
}

/// ST 内嵌世界书结构（导入时用，转换后丢弃）
#[derive(Debug, Serialize, Deserialize)]
pub struct StWorldInfoBook {
    #[serde(default)]
    pub entries: Vec<StWorldInfoEntry>,
}

/// ST 世界书条目（导入时用）
#[derive(Debug, Serialize, Deserialize)]
pub struct StWorldInfoEntry {
    pub id: Option<i32>,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub secondary_keys: Option<Vec<String>>,
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
            || self
                .extensions
                .get("assets")
                .is_some_and(|v| !v.is_null())
    }

    /// 从 ST 卡 JSON 解析为领域模型
    pub fn from_st_card(card: StCharacterCard) -> Self {
        let raw_json = serde_json::to_value(&card.data).unwrap_or_default();
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

use crate::variables::VariableField;

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
    pub role_type: RoleType,
    /// 变量 schema（基础表 + 卡 MVU initvar 扩展）
    ///
    /// 实例化时拷贝默认值到 CharacterInstance.variables。
    #[serde(default = "crate::variables::default_character_variables")]
    pub variable_schema: Vec<VariableField>,
}

/// 角色类型（决定是否常驻、是否进向量库）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoleType {
    /// 主角（常驻，进向量库）
    Protagonist,
    /// 配角（常驻，进向量库）
    Supporting,
    /// 临场基准（龙套默认类型，不进向量库；可升级）
    Extra,
}

impl Default for RoleType {
    fn default() -> Self {
        Self::Supporting
    }
}

impl CharacterCard {
    /// 从扁平 Character 初始化（角色识别 Agent 跑之前，先建空容器）
    pub fn from_character(character: &Character) -> Self {
        Self {
            id: Id::new(),
            name: character.name.clone(),
            source_character_id: character.id.clone(),
            character_definitions: vec![],
        }
    }

    /// 按名字找角色定义
    pub fn find_definition(&self, name: &str) -> Option<&CharacterDefinition> {
        self.character_definitions.iter().find(|d| d.name == name)
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
        };
        assert!(card.find_definition("林医生").is_some());
        assert!(card.find_definition("陈警官").is_some());
        assert!(card.find_definition("不存在").is_none());
    }
}
