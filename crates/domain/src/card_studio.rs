//! Card Studio Phase 1 domain: from-scratch card drafting, compile, checks.
//!
//! Independent of Campaign writing pipeline. Produces ST-compatible Character drafts.

use serde::{Deserialize, Serialize};

use crate::character::{Character, StCharacterCard, StCharacterData, StWorldInfoBook, StWorldInfoEntry};
use crate::world_info::{LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry};
use crate::{Id, Source};

/// Stage identifiers for the Phase 1 pack.
pub const STAGE_BRIEF: &str = "brief";
pub const STAGE_BASIC: &str = "basic";
pub const STAGE_PERSONALITY: &str = "personality";
pub const STAGE_WORLDVIEW: &str = "worldview";
pub const STAGE_OPENING: &str = "opening";
pub const STAGE_REVIEW: &str = "review";
pub const STAGE_COMPILE_IMPORT: &str = "compile_import";

/// Ordered stage list for FromScratch Phase 1.
pub fn phase1_stage_ids() -> &'static [&'static str] {
    &[
        STAGE_BRIEF,
        STAGE_BASIC,
        STAGE_PERSONALITY,
        STAGE_WORLDVIEW,
        STAGE_OPENING,
        STAGE_REVIEW,
        STAGE_COMPILE_IMPORT,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CardProjectMode {
    #[default]
    FromScratch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    #[default]
    Pending,
    Ready,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldviewDraftEntry {
    #[serde(default)]
    pub keys: Vec<String>,
    pub content: String,
    #[serde(default)]
    pub constant: bool,
    #[serde(default = "default_order")]
    pub order: i32,
}

fn default_order() -> i32 {
    100
}

impl Default for WorldviewDraftEntry {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            content: String::new(),
            constant: false,
            order: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CardArtifacts {
    #[serde(default)]
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
    pub tags: Vec<String>,
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub worldview_entries: Vec<WorldviewDraftEntry>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardProject {
    pub id: String,
    pub name: String,
    pub mode: CardProjectMode,
    #[serde(default)]
    pub brief: String,
    #[serde(default = "default_current_stage")]
    pub current_stage: String,
    /// stage_id -> status
    #[serde(default)]
    pub stage_status: std::collections::BTreeMap<String, StageStatus>,
    #[serde(default)]
    pub artifacts: CardArtifacts,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_stage_output: Option<String>,
    #[serde(default)]
    pub imported_character_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn default_current_stage() -> String {
    STAGE_BRIEF.to_string()
}

impl CardProject {
    pub fn new_from_scratch(name: impl Into<String>, brief: impl Into<String>) -> Self {
        let now = chrono_like_now();
        let mut stage_status = std::collections::BTreeMap::new();
        for id in phase1_stage_ids() {
            stage_status.insert((*id).to_string(), StageStatus::Pending);
        }
        stage_status.insert(STAGE_BRIEF.to_string(), StageStatus::Ready);

        let name = name.into();
        let brief = brief.into();
        let mut artifacts = CardArtifacts::default();
        artifacts.notes = brief.clone();
        if artifacts.name.is_empty() {
            artifacts.name = name.clone();
        }

        Self {
            id: Id::new().to_string(),
            name,
            mode: CardProjectMode::FromScratch,
            brief,
            current_stage: STAGE_BRIEF.to_string(),
            stage_status,
            artifacts,
            last_error: None,
            last_stage_output: None,
            imported_character_id: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = chrono_like_now();
    }

    pub fn set_stage_status(&mut self, stage_id: &str, status: StageStatus) {
        self.stage_status.insert(stage_id.to_string(), status);
        self.touch();
    }
}

fn chrono_like_now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckIssue {
    pub code: String,
    pub message: String,
    pub severity: CheckSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckReport {
    pub ok: bool,
    pub issues: Vec<CheckIssue>,
}

/// Structural L1 checks for Phase 1.
pub fn run_checks(artifacts: &CardArtifacts) -> CheckReport {
    let mut issues = Vec::new();

    if artifacts.name.trim().is_empty() {
        issues.push(CheckIssue {
            code: "name_required".into(),
            message: "角色名不能为空".into(),
            severity: CheckSeverity::Error,
        });
    }
    if artifacts.description.trim().is_empty() {
        issues.push(CheckIssue {
            code: "description_required".into(),
            message: "角色描述不能为空".into(),
            severity: CheckSeverity::Error,
        });
    }
    if artifacts.first_mes.trim().is_empty() {
        issues.push(CheckIssue {
            code: "first_mes_required".into(),
            message: "开场白不能为空".into(),
            severity: CheckSeverity::Error,
        });
    }
    if artifacts.personality.trim().is_empty() {
        issues.push(CheckIssue {
            code: "personality_missing".into(),
            message: "性格文本为空，建议补齐".into(),
            severity: CheckSeverity::Warning,
        });
    }

    for (idx, entry) in artifacts.worldview_entries.iter().enumerate() {
        if entry.content.trim().is_empty() {
            issues.push(CheckIssue {
                code: format!("worldview_{idx}_empty"),
                message: format!("世界书条目 #{idx} 内容为空"),
                severity: CheckSeverity::Error,
            });
        }
        if !entry.constant && entry.keys.iter().all(|k| k.trim().is_empty()) {
            issues.push(CheckIssue {
                code: format!("worldview_{idx}_keys"),
                message: format!("世界书条目 #{idx} 为绿灯但缺少触发关键词"),
                severity: CheckSeverity::Error,
            });
        }
    }

    let ok = !issues
        .iter()
        .any(|i| matches!(i.severity, CheckSeverity::Error));
    CheckReport { ok, issues }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileResult {
    pub character: Character,
    pub st_card_json: serde_json::Value,
    pub warnings: Vec<String>,
}

/// Compile artifacts into a native Character + ST JSON payload.
pub fn compile_artifacts(artifacts: &CardArtifacts) -> Result<CompileResult, String> {
    let report = run_checks(artifacts);
    if !report.ok {
        let msgs: Vec<String> = report
            .issues
            .iter()
            .filter(|i| matches!(i.severity, CheckSeverity::Error))
            .map(|i| i.message.clone())
            .collect();
        return Err(format!("编译前检查失败: {}", msgs.join("; ")));
    }

    let mut warnings: Vec<String> = report
        .issues
        .iter()
        .filter(|i| matches!(i.severity, CheckSeverity::Warning))
        .map(|i| i.message.clone())
        .collect();

    let book = if artifacts.worldview_entries.is_empty() {
        None
    } else {
        let entries = artifacts
            .worldview_entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let constant = e.constant;
                let selective = !constant;
                let route = if constant {
                    LoreRoute::Constant
                } else {
                    LoreRoute::Selective
                };
                let keys: Vec<String> = e
                    .keys
                    .iter()
                    .map(|k| k.trim().to_string())
                    .filter(|k| !k.is_empty())
                    .collect();
                if keys.is_empty() && !constant {
                    warnings.push(format!("条目 #{i} 无 keys，已跳过 keys 校验（应被 checks 拦住）"));
                }
                WorldInfoEntry {
                    st_id: Some(i as i32),
                    keys,
                    secondary_keys: Vec::new(),
                    content: e.content.clone(),
                    constant,
                    selective,
                    selective_logic: SelectiveLogic::And,
                    disabled: false,
                    position: 0,
                    depth: 4,
                    order: e.order,
                    route,
                    extensions: serde_json::json!({}),
                    extra: Default::default(),
                }
            })
            .collect();
        Some(WorldInfoBook {
            entries,
            source: Source::Native,
            metadata: Default::default(),
        })
    };

    let name = artifacts.name.trim().to_string();
    let creator = if artifacts.creator.trim().is_empty() {
        "StoryForge Card Studio".to_string()
    } else {
        artifacts.creator.trim().to_string()
    };

    let mut tags = artifacts.tags.clone();
    if !tags.iter().any(|t| t == "card-studio") {
        tags.push("card-studio".into());
    }

    let st_data = StCharacterData {
        name: name.clone(),
        description: artifacts.description.clone(),
        personality: artifacts.personality.clone(),
        scenario: artifacts.scenario.clone(),
        first_mes: artifacts.first_mes.clone(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        tags: tags.clone(),
        creator: creator.clone(),
        character_version: "cardstudio-1".into(),
        alternate_greetings: Vec::new(),
        extensions: serde_json::json!({
            "storyforge": {
                "card_studio": true,
                "phase": 1
            }
        }),
        character_book: book.as_ref().map(|b| b.to_st_book()),
        extra: Default::default(),
    };

    let st_card = StCharacterCard {
        spec: Some("chara_card_v3".into()),
        spec_version: Some("3.0".into()),
        data: st_data,
    };

    let st_card_json = serde_json::to_value(&st_card)
        .map_err(|e| format!("序列化 ST 卡失败: {e}"))?;

    // Rebuild Character with Source::Native after from_st_card.
    let mut character = Character::from_st_card(st_card);
    character.source = Source::Native;
    character.spec_version = "3.0".into();
    if character.embedded_world_info.is_some() {
        // from_st filters disabled only; ok
    }

    Ok(CompileResult {
        character,
        st_card_json,
        warnings,
    })
}

/// Build LLM system+user prompts for generative stages.
pub fn build_stage_prompt(
    stage_id: &str,
    project: &CardProject,
    user_note: Option<&str>,
) -> Result<(String, String), String> {
    let arts = &project.artifacts;
    let brief = if project.brief.trim().is_empty() {
        arts.notes.as_str()
    } else {
        project.brief.as_str()
    };
    let note = user_note.unwrap_or("").trim();

    let system = match stage_id {
        STAGE_BASIC => "你是角色卡写卡助手。根据用户 brief 产出角色基础信息。只输出 JSON，不要 markdown 代码围栏。遵守：用户没说的不要编造；性格不要写进 description。字段：name, description, scenario, tags(数组)。description 写外貌/背景/关系，不含性格。",
        STAGE_PERSONALITY => "你是角色卡写卡助手。根据已有角色基础与 brief 写性格文本。只输出 JSON：{ \"personality\": \"...\" }。避免单一标签堆砌，用可观察行为与内在驱动力描述。不要编造用户未提供的重大设定。",
        STAGE_WORLDVIEW => "你是角色卡写卡助手。根据 brief 与角色信息产出 1-5 条世界书条目。只输出 JSON：{ \"worldview_entries\": [ { \"keys\": [\"词\"], \"content\": \"...\", \"constant\": true/false, \"order\": 100 } ] }。蓝灯 constant=true 用于核心世界设定；绿灯 false 需要 keys。不要空 content。",
        STAGE_OPENING => "你是角色卡写卡助手。根据角色与场景写开场白 first_mes。只输出 JSON：{ \"first_mes\": \"...\" }。开场要给互动点，不要超长，不要替用户决定未确认的剧情走向。",
        STAGE_BRIEF | STAGE_REVIEW | STAGE_COMPILE_IMPORT => {
            return Err(format!("阶段 {stage_id} 不需要 LLM 生成"));
        }
        other => return Err(format!("未知阶段: {other}")),
    };

    let mut user = String::new();
    user.push_str(&format!("# Brief\n{brief}\n\n"));
    user.push_str(&format!("# 当前产物\n{}\n\n", serde_json::to_string_pretty(arts).unwrap_or_default()));
    if !note.is_empty() {
        user.push_str(&format!("# 用户补充\n{note}\n\n"));
    }
    user.push_str("请按系统要求输出 JSON。");

    Ok((system.to_string(), user))
}

/// Apply a generative stage JSON patch onto artifacts.
pub fn apply_stage_json(stage_id: &str, artifacts: &mut CardArtifacts, value: &serde_json::Value) -> Result<(), String> {
    match stage_id {
        STAGE_BASIC => {
            if let Some(s) = value.get("name").and_then(|v| v.as_str()) {
                if !s.trim().is_empty() {
                    artifacts.name = s.trim().to_string();
                }
            }
            if let Some(s) = value.get("description").and_then(|v| v.as_str()) {
                artifacts.description = s.to_string();
            }
            if let Some(s) = value.get("scenario").and_then(|v| v.as_str()) {
                artifacts.scenario = s.to_string();
            }
            if let Some(arr) = value.get("tags").and_then(|v| v.as_array()) {
                artifacts.tags = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
            }
            Ok(())
        }
        STAGE_PERSONALITY => {
            if let Some(s) = value.get("personality").and_then(|v| v.as_str()) {
                artifacts.personality = s.to_string();
                Ok(())
            } else {
                Err("personality 字段缺失".into())
            }
        }
        STAGE_WORLDVIEW => {
            let arr = value
                .get("worldview_entries")
                .and_then(|v| v.as_array())
                .ok_or_else(|| "worldview_entries 字段缺失".to_string())?;
            let mut entries = Vec::new();
            for item in arr {
                let content = item
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let constant = item
                    .get("constant")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let order = item
                    .get("order")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(100) as i32;
                let keys = item
                    .get("keys")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                entries.push(WorldviewDraftEntry {
                    keys,
                    content,
                    constant,
                    order,
                });
            }
            artifacts.worldview_entries = entries;
            Ok(())
        }
        STAGE_OPENING => {
            if let Some(s) = value.get("first_mes").and_then(|v| v.as_str()) {
                artifacts.first_mes = s.to_string();
                Ok(())
            } else {
                Err("first_mes 字段缺失".into())
            }
        }
        other => Err(format!("阶段 {other} 不支持 JSON 应用")),
    }
}

/// Best-effort extract JSON object from model text.
pub fn extract_json_object(text: &str) -> Result<serde_json::Value, String> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if v.is_object() {
            return Ok(v);
        }
    }
    // fenced ```json
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                let slice = &trimmed[start..=end];
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(slice) {
                    if v.is_object() {
                        return Ok(v);
                    }
                }
            }
        }
    }
    Err("模型输出中未找到可用 JSON 对象".into())
}

/// Helper used by compile tests / export: ensure StWorldInfoBook can be built from drafts.
#[allow(dead_code)]
pub fn drafts_to_st_book(entries: &[WorldviewDraftEntry]) -> StWorldInfoBook {
    StWorldInfoBook {
        entries: entries
            .iter()
            .enumerate()
            .map(|(i, e)| StWorldInfoEntry {
                id: Some(i as i32),
                keys: e.keys.clone(),
                key_alias: None,
                secondary_keys: None,
                keysecondary_alias: None,
                content: Some(e.content.clone()),
                constant: e.constant,
                selective: !e.constant,
                selective_logic: Some(0),
                order: Some(e.order),
                position: Some(serde_json::Value::Number(0.into())),
                disable: Some(false),
                depth: Some(4),
                extensions: serde_json::json!({}),
                extra: Default::default(),
            })
            .collect(),
        extra: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ok_artifacts() -> CardArtifacts {
        CardArtifacts {
            name: "秋青".into(),
            description: "白发少年，旅人。".into(),
            personality: "外冷内热，观察后行动。".into(),
            scenario: "雨夜车站。".into(),
            first_mes: "他抬头看你：…你也在等车？".into(),
            tags: vec!["原创".into()],
            creator: "tester".into(),
            worldview_entries: vec![
                WorldviewDraftEntry {
                    keys: vec![],
                    content: "世界规则：灵视者能看见旧日残影。".into(),
                    constant: true,
                    order: 10,
                },
                WorldviewDraftEntry {
                    keys: vec!["车站".into(), "雨".into()],
                    content: "这座车站总在雨夜汇聚迷路的人。".into(),
                    constant: false,
                    order: 20,
                },
            ],
            notes: "从零测试".into(),
        }
    }

    #[test]
    fn new_project_sets_brief_stage_ready() {
        let p = CardProject::new_from_scratch("demo", "一个雨夜少年");
        assert_eq!(p.mode, CardProjectMode::FromScratch);
        assert_eq!(p.current_stage, STAGE_BRIEF);
        assert_eq!(
            p.stage_status.get(STAGE_BRIEF).copied().unwrap_or_default(),
            StageStatus::Ready
        );
        assert!(!p.id.is_empty());
    }

    #[test]
    fn checks_require_core_fields() {
        let report = run_checks(&CardArtifacts::default());
        assert!(!report.ok);
        assert!(report.issues.iter().any(|i| i.code == "name_required"));
        assert!(report.issues.iter().any(|i| i.code == "description_required"));
        assert!(report.issues.iter().any(|i| i.code == "first_mes_required"));
    }

    #[test]
    fn checks_selective_entry_needs_keys() {
        let mut a = sample_ok_artifacts();
        a.worldview_entries[1].keys.clear();
        let report = run_checks(&a);
        assert!(!report.ok);
        assert!(report.issues.iter().any(|i| i.code.contains("keys")));
    }

    #[test]
    fn compile_ok_artifacts_to_native_character() {
        let a = sample_ok_artifacts();
        let compiled = compile_artifacts(&a).expect("compile");
        assert_eq!(compiled.character.name, "秋青");
        assert_eq!(compiled.character.source, Source::Native);
        assert_eq!(compiled.character.spec_version, "3.0");
        let book = compiled
            .character
            .embedded_world_info
            .as_ref()
            .expect("book");
        assert_eq!(book.entries.len(), 2);
        assert!(book.entries[0].constant);
        assert!(!book.entries[1].constant);
        assert!(compiled.st_card_json.get("data").is_some());
    }

    #[test]
    fn extract_json_from_fenced_and_prose() {
        let v = extract_json_object("```json\n{\"name\":\"A\"}\n```").unwrap();
        assert_eq!(v["name"], "A");
        let v2 = extract_json_object("好的，如下：\n{\"personality\":\"冷静\"}\n完").unwrap();
        assert_eq!(v2["personality"], "冷静");
    }

    #[test]
    fn apply_stage_json_updates_fields() {
        let mut a = CardArtifacts::default();
        let basic = serde_json::json!({
            "name": "络络",
            "description": "黑发女孩",
            "scenario": "图书馆",
            "tags": ["校园"]
        });
        apply_stage_json(STAGE_BASIC, &mut a, &basic).unwrap();
        assert_eq!(a.name, "络络");
        apply_stage_json(
            STAGE_PERSONALITY,
            &mut a,
            &serde_json::json!({"personality": "好奇"}),
        )
        .unwrap();
        assert_eq!(a.personality, "好奇");
        apply_stage_json(
            STAGE_OPENING,
            &mut a,
            &serde_json::json!({"first_mes": "嗨"}),
        )
        .unwrap();
        assert_eq!(a.first_mes, "嗨");
    }

    #[test]
    fn build_stage_prompt_rejects_non_llm_stages() {
        let p = CardProject::new_from_scratch("x", "y");
        assert!(build_stage_prompt(STAGE_BRIEF, &p, None).is_err());
        assert!(build_stage_prompt(STAGE_BASIC, &p, Some("偏现代")).is_ok());
    }
}
