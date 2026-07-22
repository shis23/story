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
    /// guided = 协作整理；draft = 用户允许后的代写稿
    #[serde(default)]
    pub personality_mode: Option<String>,
    /// 性格阶段需要用户补充的问题
    #[serde(default)]
    pub personality_prompts: Vec<String>,
    /// A/B/C/unknown
    #[serde(default)]
    pub world_type: Option<String>,
    /// 开场大纲（可选，便于用户回看）
    #[serde(default)]
    pub opening_outline: Option<serde_json::Value>,
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
    /// 使用的 stage pack id（默认明月秋青 v1）
    #[serde(default = "default_stage_pack_id")]
    pub stage_pack_id: String,
    /// 是否允许 AI 在性格阶段代写衍生（默认 false，对齐明月“手写优先”）
    #[serde(default)]
    pub allow_ai_freewrite: bool,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_stage_output: Option<String>,
    #[serde(default)]
    pub imported_character_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn default_stage_pack_id() -> String {
    "mingyue_qiuqing_v1".into()
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
            stage_pack_id: default_stage_pack_id(),
            allow_ai_freewrite: false,
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

/// Embedded Mingyue Qiuqing methodology pack (StoryForge-adapted).
mod pack_mingyue_v1 {
    pub const CREATIVE_PRINCIPLES: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/common/creative_principles.md");
    pub const ABSOLUTE_ZERO: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/common/absolute_zero.md");
    pub const OUTPUT_CONTRACT: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/output_contract.md");
    pub const BASIC: &str = include_str!("../assets/cardstudio/mingyue_qiuqing_v1/stages/basic.md");
    pub const PERSONALITY: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/stages/personality.md");
    pub const WORLDVIEW: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/stages/worldview.md");
    pub const OPENING: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/stages/opening.md");
}

fn stage_template(stage_id: &str) -> Result<&'static str, String> {
    match stage_id {
        STAGE_BASIC => Ok(pack_mingyue_v1::BASIC),
        STAGE_PERSONALITY => Ok(pack_mingyue_v1::PERSONALITY),
        STAGE_WORLDVIEW => Ok(pack_mingyue_v1::WORLDVIEW),
        STAGE_OPENING => Ok(pack_mingyue_v1::OPENING),
        STAGE_BRIEF | STAGE_REVIEW | STAGE_COMPILE_IMPORT => {
            Err(format!("阶段 {stage_id} 不需要 LLM 生成"))
        }
        other => Err(format!("未知阶段: {other}")),
    }
}

fn stage_task_line(stage_id: &str, allow_ai_freewrite: bool) -> String {
    match stage_id {
        STAGE_BASIC => "当前任务：角色基础。只写基本信息/外貌差异化/背景/关系，严禁把性格写进 description。".into(),
        STAGE_PERSONALITY if allow_ai_freewrite => {
            "当前任务：性格调色盘（draft 模式）。用户已允许代写；仍须避免空洞标签，优先行为与语料。mode 填 draft。".into()
        }
        STAGE_PERSONALITY => {
            "当前任务：性格调色盘（guided 协作模式，默认）。整理底色/主色/点缀结构；衍生与关键台词用【待用户手写】占位，并在 user_prompts 列出需要用户补充的问题。禁止替用户编造无关联衍生。mode 填 guided。".into()
        }
        STAGE_WORLDVIEW => {
            "当前任务：世界观世界书条目。先判断 A/B/C 类型；用户没说的不扩展。输出 worldview_entries。".into()
        }
        STAGE_OPENING => {
            "当前任务：开场白。优先整理用户已给信息为可用 first_mes；未提供的 outline 字段留空，不要脑补剧情。".into()
        }
        _ => format!("当前任务阶段：{stage_id}"),
    }
}

/// Build LLM system+user prompts for generative stages.
///
/// System prompt is assembled from the Mingyue methodology pack:
/// output contract + creative principles + absolute zero + stage template.
pub fn build_stage_prompt(
    stage_id: &str,
    project: &CardProject,
    user_note: Option<&str>,
) -> Result<(String, String), String> {
    let template = stage_template(stage_id)?;
    let arts = &project.artifacts;
    let brief = if project.brief.trim().is_empty() {
        arts.notes.as_str()
    } else {
        project.brief.as_str()
    };
    let note = user_note.unwrap_or("").trim();

    // Detect freewrite intent from explicit project flag or user note keywords.
    let freewrite = project.allow_ai_freewrite
        || note.contains("允许代写")
        || note.contains("自由发挥")
        || note.contains("你可以写衍生")
        || note.contains("AI代写");

    let mut system = String::new();
    system.push_str("# 角色\n");
    system.push_str("你是 StoryForge 写卡引擎的阶段生成器，执行「明月秋青」写卡方法论（已去 ST 宿主宏与聊天人设）。\n");
    system.push_str("你不是角色扮演陪伴，不使用“哥哥/秋青子”口吻；只产出可编译的结构化结果。\n\n");
    system.push_str(pack_mingyue_v1::OUTPUT_CONTRACT);
    system.push_str("\n\n# 写卡方法论 · 创作思路\n");
    system.push_str(pack_mingyue_v1::CREATIVE_PRINCIPLES);
    system.push_str("\n\n# 写卡方法论 · 绝对零度\n");
    system.push_str(pack_mingyue_v1::ABSOLUTE_ZERO);
    system.push_str("\n\n# 当前阶段模板\n");
    system.push_str(template);
    system.push_str("\n\n# 本轮任务约束\n");
    system.push_str(&stage_task_line(stage_id, freewrite));
    system.push_str("\n最终只输出 JSON 对象。\n");

    let mut user = String::new();
    user.push_str(&format!("# Brief\n{brief}\n\n"));
    user.push_str(&format!(
        "# 项目设置\n- stage_pack: {}\n- allow_ai_freewrite: {}\n- current_stage: {stage_id}\n\n",
        project.stage_pack_id, freewrite
    ));
    user.push_str(&format!(
        "# 当前产物 JSON\n{}\n\n",
        serde_json::to_string_pretty(arts).unwrap_or_else(|_| "{}".into())
    ));
    if !note.is_empty() {
        user.push_str(&format!("# 用户补充\n{note}\n\n"));
    }
    user.push_str("请严格按系统中的输出契约返回 JSON。");

    Ok((system, user))
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
            } else {
                return Err("personality 字段缺失".into());
            }
            artifacts.personality_mode = value
                .get("mode")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| Some("guided".into()));
            artifacts.personality_prompts = value
                .get("user_prompts")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            Ok(())
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
            if let Some(t) = value.get("world_type").and_then(|v| v.as_str()) {
                artifacts.world_type = Some(t.to_string());
            }
            if let Some(n) = value.get("notes").and_then(|v| v.as_str()) {
                if !n.trim().is_empty() {
                    if artifacts.notes.is_empty() {
                        artifacts.notes = n.to_string();
                    } else {
                        artifacts.notes = format!("{}\n\n[世界观备注]\n{}", artifacts.notes, n);
                    }
                }
            }
            Ok(())
        }
        STAGE_OPENING => {
            if let Some(s) = value.get("first_mes").and_then(|v| v.as_str()) {
                artifacts.first_mes = s.to_string();
            } else {
                return Err("first_mes 字段缺失".into());
            }
            if let Some(outline) = value.get("outline") {
                artifacts.opening_outline = Some(outline.clone());
            }
            Ok(())
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
            personality_mode: Some("guided".into()),
            personality_prompts: vec![],
            world_type: Some("B".into()),
            opening_outline: None,
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

    #[test]
    fn build_stage_prompt_embeds_mingyue_methodology() {
        let p = CardProject::new_from_scratch("秋青", "雨夜车站的旅人");
        let (system, user) = build_stage_prompt(STAGE_BASIC, &p, None).unwrap();
        // 来自明月方法论资产，而不是一行薄骨架
        assert!(system.contains("绝对零度") || system.contains("白描") || system.contains("八股"));
        assert!(system.contains("角色基础") || system.contains("template_basic") || system.contains("外貌"));
        assert!(system.contains("只输出一个 JSON") || system.contains("JSON 对象") || system.contains("输出契约"));
        assert!(system.len() > 2000, "system prompt should embed full stage pack, got {}", system.len());
        assert!(user.contains("雨夜车站"));
        assert!(user.contains("当前产物"));
    }

    #[test]
    fn personality_prompt_defaults_to_guided_not_freewrite() {
        let p = CardProject::new_from_scratch("x", "冷淡少年");
        let (system, _) = build_stage_prompt(STAGE_PERSONALITY, &p, None).unwrap();
        assert!(system.contains("guided") || system.contains("协作") || system.contains("待用户手写"));
        assert!(!system.contains("draft 模式") || system.contains("guided"));
        let (system2, _) =
            build_stage_prompt(STAGE_PERSONALITY, &p, Some("允许代写，你可以写衍生")).unwrap();
        assert!(system2.contains("draft") || system2.contains("代写"));
    }

    #[test]
    fn apply_personality_keeps_prompts_and_mode() {
        let mut a = CardArtifacts::default();
        apply_stage_json(
            STAGE_PERSONALITY,
            &mut a,
            &serde_json::json!({
                "personality": "底色：克制\n主色：温柔\n衍生：【待用户手写】",
                "mode": "guided",
                "user_prompts": ["请手写一个反差衍生"]
            }),
        )
        .unwrap();
        assert_eq!(a.personality_mode.as_deref(), Some("guided"));
        assert_eq!(a.personality_prompts.len(), 1);
    }
}
