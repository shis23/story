//! Card Studio Phase 1 domain: from-scratch drafting, reverse-parse revise, compile, checks.
//!
//! Independent of Campaign writing pipeline. Produces ST-compatible Character drafts.
//! Mode C (`FromExistingCard`) always 另存 on compile/import — never overwrites source cards.

use serde::{Deserialize, Serialize};

use crate::character::{
    Character, StCharacterCard, StCharacterData, StWorldInfoBook, StWorldInfoEntry,
};
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
    /// 从小说摘录/全文蒸馏预填写卡工程（B path MVP）
    FromNovel,
    /// 从已有角色卡反解析进入写卡工程（默认另存，不覆盖原卡）
    FromExistingCard,
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
    /// Phase 3 高级策略（骨架）：触发概率 0-100（None = 100%）。
    /// 编译时写入 entry.extensions；ST 一等字段全量映射属 Phase 3 后续。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability: Option<u8>,
    /// Phase 3 高级策略（骨架）：排除递归触发。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_recursion: Option<bool>,
    /// Phase 3 高级策略（骨架）：条目分组（蓝绿灯分组策略）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
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
            probability: None,
            exclude_recursion: None,
            group: None,
        }
    }
}

/// Phase 3 骨架：多角色卡的附加 CharacterDefinition 草案。
/// 编译时进 ST 卡 extensions.storyforge.extra_definitions；导入侧
/// `extra_definitions_from_st_extensions` 取回并 attach 到 CharacterCard。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ExtraDefinitionDraft {
    pub name: String,
    #[serde(default)]
    pub persona_prompt: String,
    #[serde(default)]
    pub behavior_rules: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
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
    /// 文风提示/公式（B 路径；进 notes 旁路，不直接当 description）
    #[serde(default)]
    pub style_notes: Option<String>,
    /// Phase 3 骨架：多角色卡附加定义草案（编译进 extensions 通道）
    #[serde(default)]
    pub extra_definitions: Vec<ExtraDefinitionDraft>,
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
    /// 反解析来源：Character domain id / source_character_id
    #[serde(default)]
    pub source_character_id: Option<String>,
    /// 反解析来源：CharacterStore 存储 id（若有）
    #[serde(default)]
    pub source_stored_id: Option<String>,
    /// B 路径：小说原文（可截断；完整大文件后续可外置）
    #[serde(default)]
    pub novel_text: Option<String>,
    /// B 路径：小说标题
    #[serde(default)]
    pub novel_title: Option<String>,
    /// B 路径：切分后的摘录（用于 prefill / style）
    #[serde(default)]
    pub novel_excerpts: Vec<String>,
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
        let mut artifacts = CardArtifacts {
            notes: brief.clone(),
            ..CardArtifacts::default()
        };
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
            source_character_id: None,
            source_stored_id: None,
            novel_text: None,
            novel_title: None,
            novel_excerpts: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Create a FromNovel project from pasted/imported text (B path MVP).
    ///
    /// Does not run LLM yet; stores excerpts and lands on BASIC ready for prefill.
    pub fn new_from_novel(
        name: impl Into<String>,
        brief: impl Into<String>,
        novel_title: impl Into<String>,
        novel_text: impl Into<String>,
    ) -> Self {
        let now = chrono_like_now();
        let mut stage_status = std::collections::BTreeMap::new();
        for id in phase1_stage_ids() {
            stage_status.insert((*id).to_string(), StageStatus::Pending);
        }
        // brief is given by novel intent; jump to basic prefill
        stage_status.insert(STAGE_BRIEF.to_string(), StageStatus::Done);
        stage_status.insert(STAGE_BASIC.to_string(), StageStatus::Ready);

        let name = name.into();
        let brief = brief.into();
        let novel_title = {
            let t = novel_title.into();
            if t.trim().is_empty() { name.clone() } else { t }
        };
        let novel_text = novel_text.into();
        let excerpts = sample_novel_excerpts(&novel_text, 12_000);
        // Keep full text only when small; large novels rely on excerpts for prefill/re-run.
        let stored_full_text = if novel_text.chars().count() > 80_000 {
            None
        } else {
            Some(novel_text)
        };
        let mut artifacts = CardArtifacts {
            notes: if brief.trim().is_empty() {
                format!("小说改编：{}", novel_title)
            } else {
                brief.clone()
            },
            ..CardArtifacts::default()
        };
        artifacts.name = name.clone();

        Self {
            id: Id::new().to_string(),
            name: if name.trim().is_empty() {
                format!("{}（小说改编）", novel_title)
            } else {
                name
            },
            mode: CardProjectMode::FromNovel,
            brief: if brief.trim().is_empty() {
                format!("从小说《{}》改编角色卡", novel_title)
            } else {
                brief
            },
            current_stage: STAGE_BASIC.to_string(),
            stage_status,
            artifacts,
            stage_pack_id: default_stage_pack_id(),
            allow_ai_freewrite: false,
            last_error: None,
            last_stage_output: None,
            imported_character_id: None,
            source_character_id: None,
            source_stored_id: None,
            novel_text: stored_full_text,
            novel_title: Some(novel_title),
            novel_excerpts: excerpts,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Create a revise project from an existing Character (C path).
    pub fn new_from_existing_character(
        character: &Character,
        stored_id: Option<String>,
        brief: impl Into<String>,
    ) -> Self {
        let now = chrono_like_now();
        let mut stage_status = std::collections::BTreeMap::new();
        for id in phase1_stage_ids() {
            // reverse-parse fills artifacts; mark generative stages ready for selective re-run
            let status = match *id {
                STAGE_BRIEF => StageStatus::Done,
                STAGE_COMPILE_IMPORT => StageStatus::Pending,
                STAGE_REVIEW => StageStatus::Ready,
                _ => StageStatus::Ready,
            };
            stage_status.insert((*id).to_string(), status);
        }

        let artifacts = reverse_parse_character(character);
        let brief = {
            let b = brief.into();
            if b.trim().is_empty() {
                format!("修订已有角色卡：{}", character.name)
            } else {
                b
            }
        };

        Self {
            id: Id::new().to_string(),
            name: format!("{}（修订）", character.name),
            mode: CardProjectMode::FromExistingCard,
            brief,
            current_stage: STAGE_REVIEW.to_string(),
            stage_status,
            artifacts,
            stage_pack_id: default_stage_pack_id(),
            allow_ai_freewrite: false,
            last_error: None,
            last_stage_output: None,
            imported_character_id: None,
            source_character_id: Some(character.id.as_str().to_string()),
            source_stored_id: stored_id,
            novel_text: None,
            novel_title: None,
            novel_excerpts: Vec::new(),
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

/// Sample head/mid/tail excerpts for B-path prefill without shipping whole novels into every prompt.
pub fn sample_novel_excerpts(text: &str, max_chars_per_slice: usize) -> Vec<String> {
    let cleaned = text.replace("\r\n", "\n");
    let chars: Vec<char> = cleaned.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let n = max_chars_per_slice.max(500);
    if chars.len() <= n {
        return vec![chars.iter().collect()];
    }
    let head: String = chars.iter().take(n).collect();
    let mid_start = chars.len().saturating_sub(n) / 2;
    let mid: String = chars.iter().skip(mid_start).take(n).collect();
    let tail: String = chars
        .iter()
        .rev()
        .take(n)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut out = vec![head];
    if mid_start > n / 2 {
        out.push(mid);
    }
    if chars.len() > n * 2 {
        out.push(tail);
    }
    out
}

/// Build prompt for novel → card prefill (B path).
pub fn build_novel_prefill_prompt(project: &CardProject) -> Result<String, String> {
    if project.novel_excerpts.is_empty()
        && project
            .novel_text
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        return Err("小说正文为空，无法预填".into());
    }
    let excerpts = if project.novel_excerpts.is_empty() {
        sample_novel_excerpts(project.novel_text.as_deref().unwrap_or(""), 12_000)
    } else {
        project.novel_excerpts.clone()
    };
    let body = excerpts
        .iter()
        .enumerate()
        .map(|(i, e)| format!("### 摘录 {}\n{}", i + 1, e))
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(format!(
        "{common}\n\n{task}\n\n## 用户 brief\n{brief}\n\n## 小说标题\n{title}\n\n## 小说摘录\n{body}\n",
        common = DISTILL_PACK::COMMON,
        task = DISTILL_PACK::PREFILL,
        brief = if project.brief.trim().is_empty() {
            "（未提供，请自选主卡角色）"
        } else {
            project.brief.as_str()
        },
        title = project
            .novel_title
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(project.name.as_str()),
        body = body,
    ))
}

/// Build short style-sample prompt from stored excerpts.
pub fn build_novel_style_prompt(project: &CardProject) -> Result<String, String> {
    let sample = project
        .novel_excerpts
        .first()
        .cloned()
        .or_else(|| {
            project.novel_text.as_ref().map(|t| {
                sample_novel_excerpts(t, 6_000)
                    .into_iter()
                    .next()
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default();
    if sample.trim().is_empty() {
        return Err("无可用摘录做文风抽样".into());
    }
    Ok(format!(
        "{common}\n\n{task}\n\n## 摘录\n{sample}\n",
        common = DISTILL_PACK::COMMON,
        task = DISTILL_PACK::STYLE,
        sample = sample,
    ))
}

/// Apply prefill JSON (from novel distill) onto artifacts.
pub fn apply_novel_prefill_json(
    artifacts: &mut CardArtifacts,
    value: &serde_json::Value,
) -> Result<(), String> {
    apply_stage_json(STAGE_BASIC, artifacts, value)?;
    if let Some(p) = value.get("personality").and_then(|v| v.as_str())
        && !p.trim().is_empty()
    {
        artifacts.personality = p.to_string();
    }
    if let Some(s) = value.get("scenario").and_then(|v| v.as_str())
        && !s.trim().is_empty()
    {
        artifacts.scenario = s.to_string();
    }
    if let Some(f) = value.get("first_mes").and_then(|v| v.as_str())
        && !f.trim().is_empty()
    {
        artifacts.first_mes = f.to_string();
    }
    if let Some(style) = value
        .get("style_notes")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        artifacts.style_notes = Some(style.to_string());
        if !artifacts.notes.contains("文风") {
            artifacts.notes = format!("{}\n\n[文风笔记]\n{}", artifacts.notes.trim(), style)
                .trim()
                .to_string();
        }
    }
    if let Some(wt) = value.get("world_type").and_then(|v| v.as_str()) {
        artifacts.world_type = Some(wt.to_string());
    }
    if let Some(arr) = value.get("worldview_entries").and_then(|v| v.as_array()) {
        let mut entries = Vec::new();
        for (i, item) in arr.iter().enumerate() {
            let keys = item
                .get("keys")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let content = item
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if content.trim().is_empty() {
                continue;
            }
            let constant = item
                .get("constant")
                .and_then(|v| v.as_bool())
                .unwrap_or(keys.is_empty());
            let order = item
                .get("order")
                .and_then(|v| v.as_i64())
                .map(|n| n as i32)
                .unwrap_or(((i as i32) + 1) * 10);
            entries.push(WorldviewDraftEntry {
                keys,
                content,
                constant,
                order,
                ..WorldviewDraftEntry::default()
            });
        }
        if !entries.is_empty() {
            artifacts.worldview_entries = entries;
        }
    }
    if let Some(sec) = value.get("secondary_characters").and_then(|v| v.as_array()) {
        let lines: Vec<String> = sec
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .map(|s| format!("- {s}"))
            .collect();
        if !lines.is_empty() {
            artifacts.notes = format!("{}\n\n[配角]\n{}", artifacts.notes.trim(), lines.join("\n"))
                .trim()
                .to_string();
        }
    }
    Ok(())
}

/// Reverse-parse a domain Character into editable CardArtifacts.
pub fn reverse_parse_character(character: &Character) -> CardArtifacts {
    let worldview_entries = character
        .embedded_world_info
        .as_ref()
        .map(|book| {
            book.entries
                .iter()
                .filter(|e| !e.disabled)
                .map(|e| WorldviewDraftEntry {
                    keys: e.keys.clone(),
                    content: e.content.clone(),
                    constant: e.constant
                        || matches!(e.route, LoreRoute::Constant | LoreRoute::Both),
                    order: e.order,
                    ..WorldviewDraftEntry::default()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    CardArtifacts {
        name: character.name.clone(),
        description: character.description.clone(),
        personality: character.personality.clone(),
        scenario: character.scenario.clone(),
        first_mes: character.first_mes.clone(),
        tags: character.tags.clone(),
        creator: character.creator.clone(),
        worldview_entries,
        notes: format!(
            "反解析自角色卡 {}（source={}）",
            character.name,
            character.id.as_str()
        ),
        personality_mode: Some("guided".into()),
        personality_prompts: Vec::new(),
        world_type: None,
        opening_outline: None,
        style_notes: None,
        extra_definitions: Vec::new(),
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
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default)]
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckReport {
    pub ok: bool,
    pub issues: Vec<CheckIssue>,
    #[serde(default)]
    pub score: Option<u32>,
    #[serde(default)]
    pub summary: Option<String>,
    /// rule | hybrid | llm
    #[serde(default)]
    pub source: Option<String>,
}

fn issue(
    code: &str,
    message: &str,
    severity: CheckSeverity,
    field: Option<&str>,
    suggestion: Option<&str>,
) -> CheckIssue {
    CheckIssue {
        code: code.into(),
        message: message.into(),
        severity,
        field: field.map(|s| s.into()),
        suggestion: suggestion.map(|s| s.into()),
    }
}

fn contains_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

/// Structural + lightweight methodology heuristics (L1/L2).
pub fn run_checks(artifacts: &CardArtifacts) -> CheckReport {
    let mut issues = Vec::new();

    if artifacts.name.trim().is_empty() {
        issues.push(issue(
            "name_required",
            "角色名不能为空",
            CheckSeverity::Error,
            Some("name"),
            Some("填写角色名"),
        ));
    }
    if artifacts.description.trim().is_empty() {
        issues.push(issue(
            "description_required",
            "角色描述不能为空",
            CheckSeverity::Error,
            Some("description"),
            Some("补齐基本信息/外貌差异化/背景/关系"),
        ));
    }
    if artifacts.first_mes.trim().is_empty() {
        issues.push(issue(
            "first_mes_required",
            "开场白不能为空",
            CheckSeverity::Error,
            Some("first_mes"),
            Some("写一段可互动的开场"),
        ));
    }
    if artifacts.personality.trim().is_empty() {
        issues.push(issue(
            "personality_missing",
            "性格文本为空，建议补齐",
            CheckSeverity::Warning,
            Some("personality"),
            Some("至少写出底色/主色调"),
        ));
    }

    // description should not mainly be personality dump
    let desc = artifacts.description.as_str();
    if !desc.trim().is_empty()
        && contains_any(
            desc,
            &[
                "性格：",
                "性格是",
                "性格特点",
                "性格调色盘",
                "底色：",
                "主色调：",
                "性格标签",
            ],
        )
    {
        issues.push(issue(
            "description_has_personality",
            "description 疑似混入性格内容（应放到 personality/调色盘）",
            CheckSeverity::Warning,
            Some("description"),
            Some("把性格段落移到 personality，description 只保留外貌/背景/关系"),
        ));
    }

    // bagua-ish wording heuristic
    let joined = format!(
        "{}\n{}\n{}",
        artifacts.description, artifacts.personality, artifacts.first_mes
    );
    let bagua = [
        "似乎",
        "仿佛",
        "宛如",
        "如同",
        "嘴角上扬",
        "眼里闪过",
        "指尖泛白",
        "心湖",
        "小兽",
        "投石入湖",
        "不是…而是",
        "不是...而是",
    ];
    let hits: Vec<&str> = bagua
        .iter()
        .copied()
        .filter(|w| joined.contains(w))
        .collect();
    if !hits.is_empty() {
        issues.push(issue(
            "bagua_wording",
            &format!("检测到可能的八股措辞：{}", hits.join("、")),
            CheckSeverity::Warning,
            Some("general"),
            Some("按绝对零度/白描改写：少模糊词与微表情套路，改写具体行为"),
        ));
    }

    // personality guided placeholders
    if artifacts.personality_mode.as_deref().unwrap_or("guided") == "guided"
        && !artifacts.personality.trim().is_empty()
        && artifacts.personality.contains("【待用户手写】")
    {
        issues.push(issue(
            "personality_pending_handwrite",
            "性格仍有【待用户手写】占位，导入前建议补完衍生",
            CheckSeverity::Info,
            Some("personality"),
            Some("按 user_prompts 手写衍生后再导入，或显式允许 AI 代写后重跑"),
        ));
    }

    if artifacts.worldview_entries.is_empty() {
        issues.push(issue(
            "worldview_empty",
            "尚无世界书条目",
            CheckSeverity::Warning,
            Some("worldview"),
            Some("至少补 1 条蓝灯核心设定，或确认这是纯人设小卡"),
        ));
    }

    for (idx, entry) in artifacts.worldview_entries.iter().enumerate() {
        if entry.content.trim().is_empty() {
            issues.push(issue(
                &format!("worldview_{idx}_empty"),
                &format!("世界书条目 #{idx} 内容为空"),
                CheckSeverity::Error,
                Some("worldview"),
                Some("删除空条目或补正文"),
            ));
        } else if entry.content.trim().chars().count() < 12 {
            issues.push(issue(
                &format!("worldview_{idx}_too_short"),
                &format!("世界书条目 #{idx} 过短，可能信息不足"),
                CheckSeverity::Warning,
                Some("worldview"),
                Some("补充可检索的具体设定，避免空话"),
            ));
        }
        if !entry.constant && entry.keys.iter().all(|k| k.trim().is_empty()) {
            issues.push(issue(
                &format!("worldview_{idx}_keys"),
                &format!("世界书条目 #{idx} 为绿灯但缺少触发关键词"),
                CheckSeverity::Error,
                Some("worldview"),
                Some("为绿灯条目填写 keys，或改成蓝灯 constant=true"),
            ));
        }
        // tag-spec soft check: if content has <tag> style, ok; if long constant entry without any structure marker, info
        if entry.constant
            && entry.content.chars().count() > 80
            && !entry.content.contains('<')
            && !entry.content.contains("：")
            && !entry.content.contains(":")
        {
            issues.push(issue(
                &format!("worldview_{idx}_structure"),
                &format!("世界书条目 #{idx} 较长且缺少结构标记，后续可按标签规范整理"),
                CheckSeverity::Info,
                Some("worldview"),
                Some("可用小标题或 <名称_idN> 标签包裹，便于检索与维护"),
            ));
        }
    }

    if !artifacts.first_mes.trim().is_empty() {
        let fm = artifacts.first_mes.as_str();
        let interactive = contains_any(
            fm,
            &["？", "?", "你", "您", "咱们", "一起", "要不要", "……", "..."],
        );
        if !interactive && fm.chars().count() > 20 {
            issues.push(issue(
                "opening_no_hook",
                "开场白可能缺少互动点/对用户的抓手",
                CheckSeverity::Warning,
                Some("first_mes"),
                Some("补一个可回应的动作、问题或选择点"),
            ));
        }
    }

    let error_n = issues
        .iter()
        .filter(|i| matches!(i.severity, CheckSeverity::Error))
        .count();
    let warn_n = issues
        .iter()
        .filter(|i| matches!(i.severity, CheckSeverity::Warning))
        .count();
    let ok = error_n == 0;
    let mut score: i32 = 100;
    score -= (error_n as i32) * 25;
    score -= (warn_n as i32) * 8;
    score = score.clamp(0, 100);
    let summary = if ok && warn_n == 0 {
        "结构完整，启发式检查未发现明显问题".into()
    } else if ok {
        format!("可通过导入门槛，但仍有 {warn_n} 条建议")
    } else {
        format!("存在 {error_n} 个必须修复项")
    };

    CheckReport {
        ok,
        issues,
        score: Some(score as u32),
        summary: Some(summary),
        source: Some("rule".into()),
    }
}

/// Merge LLM review JSON into a base rule report (errors from rules still gate import).
pub fn merge_review_reports(base: CheckReport, llm_value: &serde_json::Value) -> CheckReport {
    let mut issues = base.issues;
    if let Some(arr) = llm_value.get("issues").and_then(|v| v.as_array()) {
        for item in arr {
            let code = item
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("llm_issue")
                .to_string();
            let message = item
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if message.trim().is_empty() {
                continue;
            }
            let severity = match item
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("warning")
            {
                "error" => CheckSeverity::Error,
                "info" => CheckSeverity::Info,
                _ => CheckSeverity::Warning,
            };
            // LLM cannot alone invent hard blockers for missing core fields; demote unknown errors to warning
            // unless code is clearly aligned with methodology.
            let severity = if matches!(severity, CheckSeverity::Error)
                && !code.contains("required")
                && !code.contains("keys")
                && !code.contains("empty")
            {
                CheckSeverity::Warning
            } else {
                severity
            };
            issues.push(CheckIssue {
                code: format!("llm_{code}"),
                message,
                severity,
                field: item
                    .get("field")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                suggestion: item
                    .get("suggestion")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            });
        }
    }

    let error_n = issues
        .iter()
        .filter(|i| matches!(i.severity, CheckSeverity::Error))
        .count();
    let ok = error_n == 0;
    let score = llm_value
        .get("score")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .or(base.score)
        .map(|s| {
            // never higher than rule score if errors remain
            if !ok { s.min(60) } else { s }
        });
    let summary = llm_value
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(base.summary)
        .map(|s| format!("{s}（规则+LLM）"));

    CheckReport {
        ok,
        issues,
        score,
        summary,
        source: Some("hybrid".into()),
    }
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
                    warnings.push(format!(
                        "条目 #{i} 无 keys，已跳过 keys 校验（应被 checks 拦住）"
                    ));
                }
                // Phase 3 骨架：高级策略进 extensions（ST 一等字段全量映射后续做）
                let mut ext = serde_json::Map::new();
                if let Some(p) = e.probability {
                    ext.insert("probability".into(), serde_json::json!(p.min(100)));
                    ext.insert("useProbability".into(), serde_json::json!(true));
                }
                if let Some(x) = e.exclude_recursion {
                    ext.insert("exclude_recursion".into(), serde_json::json!(x));
                }
                if let Some(g) = e.group.as_deref().filter(|g| !g.trim().is_empty()) {
                    ext.insert("group".into(), serde_json::json!(g.trim()));
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
                    extensions: serde_json::Value::Object(ext),
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
        extensions: {
            let mut sf = serde_json::json!({
                "card_studio": true,
                "phase": 1
            });
            // Phase 3 骨架：附加定义草案随卡携带（导入侧
            // extra_definitions_from_st_extensions 取回 attach）
            if !artifacts.extra_definitions.is_empty()
                && let Some(obj) = sf.as_object_mut()
            {
                obj.insert(
                    "extra_definitions".into(),
                    serde_json::to_value(&artifacts.extra_definitions)
                        .map_err(|e| format!("附加定义序列化失败: {e}"))?,
                );
            }
            serde_json::json!({ "storyforge": sf })
        },
        character_book: book.as_ref().map(|b| b.to_st_book()),
        extra: Default::default(),
    };

    let st_card = StCharacterCard {
        spec: Some("chara_card_v3".into()),
        spec_version: Some("3.0".into()),
        data: st_data,
    };

    let st_card_json =
        serde_json::to_value(&st_card).map_err(|e| format!("序列化 ST 卡失败: {e}"))?;

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

/// InitVar 条目的固定标记（内容前缀 + keys；MVU 分析器按内容含 "initvar"
/// 识别并给大预算——见 mvu_analyzer 的 [InitVar] 预算规则）。
pub const MVU_INITVAR_MARKER: &str = "[InitVar]";

/// Phase 3 骨架：把 MVU InitVar YAML 草稿注入 artifacts 世界书。
///
/// 走普通 `WorldviewDraftEntry` 通道（constant 常驻 + `[InitVar]` 内容标记），
/// 出卡闸门（content/keys/order roundtrip）与 forge 差分口径零改动即成立；
/// 导入侧 MVU 分析器按内容关键字识别。幂等：已有 InitVar 条目则替换。
pub fn apply_mvu_bootstrap_entry(
    artifacts: &mut CardArtifacts,
    initvar_yaml: &str,
) -> Result<(), String> {
    let yaml = initvar_yaml.trim();
    if yaml.is_empty() {
        return Err("InitVar YAML 为空".into());
    }
    serde_yaml_probe(yaml)?;
    let content = format!("{MVU_INITVAR_MARKER}\n{yaml}");
    let max_order = artifacts
        .worldview_entries
        .iter()
        .map(|e| e.order)
        .max()
        .unwrap_or(0);
    let entry = WorldviewDraftEntry {
        keys: vec!["InitVar".into()],
        content,
        constant: true,
        order: max_order + 10,
        ..WorldviewDraftEntry::default()
    };
    if let Some(existing) = artifacts
        .worldview_entries
        .iter_mut()
        .find(|e| e.content.starts_with(MVU_INITVAR_MARKER))
    {
        let order = existing.order;
        *existing = WorldviewDraftEntry { order, ..entry };
    } else {
        artifacts.worldview_entries.push(entry);
    }
    Ok(())
}

/// 轻量 YAML 结构探针：拒绝把明显不是 mapping 的文本当 InitVar 落卡。
fn serde_yaml_probe(yaml: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_yaml::from_str(yaml).map_err(|e| format!("InitVar YAML 解析失败: {e}"))?;
    if !value.is_object() {
        return Err("InitVar YAML 顶层必须是 mapping（变量树）".into());
    }
    Ok(())
}

/// Phase 3 骨架：从 ST 卡 extensions 取回附加定义草案（导入侧 attach 用）。
/// 形态异常时返回空集（防御：extensions 是外来 JSON）。
pub fn extra_definitions_from_st_extensions(
    extensions: &serde_json::Value,
) -> Vec<ExtraDefinitionDraft> {
    extensions
        .get("storyforge")
        .and_then(|sf| sf.get("extra_definitions"))
        .and_then(|v| serde_json::from_value::<Vec<ExtraDefinitionDraft>>(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|d| !d.name.trim().is_empty())
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// 出卡质量闸门（确定性，无 LLM）
// ═══════════════════════════════════════════════════════════════════════════

/// 出卡质量闸门单条检查结果。
/// 与 harness card_translation 验收的 Check 同形（name/pass/detail），
/// 但期望值不是固定阈值，而是从编译前 artifacts 精确导出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateCheck {
    pub name: String,
    pub pass: bool,
    pub detail: String,
}

fn gate(name: &str, pass: bool, detail: String) -> GateCheck {
    GateCheck {
        name: name.into(),
        pass,
        detail,
    }
}

/// 对「编译产物经真实导入路径 round-trip 后的 Character」做确定性检查。
///
/// 调用方（tauri-app）负责 round-trip：compile → 序列化 → infra-import
/// `import_character` → 把重新导入的 Character 传进来。这里只做纯 domain 断言，
/// 语义对齐 harness card_translation 的导入检查（世界书路由、禁用条目、组件归类），
/// 差异在于 Studio 卡的期望是精确值（条目数、全启用、无脚本组件）而非下限阈值。
pub fn export_gate_checks(artifacts: &CardArtifacts, reimported: &Character) -> Vec<GateCheck> {
    let mut out = Vec::new();

    let expected_name = artifacts.name.trim();
    out.push(gate(
        "gate.name_roundtrip",
        reimported.name == expected_name,
        format!(
            "导入后 name={:?}（期望 {:?}）",
            reimported.name, expected_name
        ),
    ));
    out.push(gate(
        "gate.first_mes_roundtrip",
        !reimported.first_mes.trim().is_empty() && reimported.first_mes == artifacts.first_mes,
        format!(
            "导入后 first_mes {} 字符（期望与产物一致且非空）",
            reimported.first_mes.chars().count()
        ),
    ));

    let book = reimported.embedded_world_info.as_ref();
    let total = book.map(|b| b.entries.len()).unwrap_or(0);
    let expected_total = artifacts.worldview_entries.len();
    out.push(gate(
        "gate.book_total",
        total == expected_total,
        format!("{total} 条世界书（期望 {expected_total}）"),
    ));

    let disabled = book
        .map(|b| b.entries.iter().filter(|e| e.disabled).count())
        .unwrap_or(0);
    out.push(gate(
        "gate.book_all_enabled",
        disabled == 0,
        format!("{disabled} 条被标记禁用（Studio 编译应全启用）"),
    ));

    let constant = book
        .map(|b| b.entries.iter().filter(|e| e.constant).count())
        .unwrap_or(0);
    let expected_constant = artifacts
        .worldview_entries
        .iter()
        .filter(|e| e.constant)
        .count();
    out.push(gate(
        "gate.constant_entries",
        constant == expected_constant,
        format!("{constant} 条常驻（期望 {expected_constant}）"),
    ));

    // 死路由检查：启用、非常驻、带主键的条目绝不能被路由成 Disabled
    // （harness import.no_dead_keyword_routes 的 Studio 版）
    let dead = book
        .map(|b| {
            b.entries
                .iter()
                .filter(|e| {
                    !e.disabled
                        && !e.constant
                        && e.keys.iter().any(|k| !k.trim().is_empty())
                        && matches!(e.route, LoreRoute::Disabled)
                })
                .count()
        })
        .unwrap_or(0);
    out.push(gate(
        "gate.no_dead_keyword_routes",
        dead == 0,
        format!("{dead} 条启用关键词条目被路由成 Disabled（应为 0）"),
    ));

    // 非常驻条目 keys 不得在 round-trip 中丢失（选择性条目失去 keys 即成死条目）
    let keyless_selective = book
        .map(|b| {
            b.entries
                .iter()
                .filter(|e| !e.constant && !e.keys.iter().any(|k| !k.trim().is_empty()))
                .count()
        })
        .unwrap_or(0);
    out.push(gate(
        "gate.selective_keys_retained",
        keyless_selective == 0,
        format!("{keyless_selective} 条选择性条目丢失 keys（应为 0）"),
    ));

    // 组件零漏项（Studio 版）：Phase 1 产卡不携带任何脚本组件；
    // round-trip 后出现 regex_scripts / tavern_helper 即为编译或导入层污染
    let regex_n = reimported
        .extensions
        .get("regex_scripts")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let th_n = reimported
        .extensions
        .get("tavern_helper")
        .and_then(|v| v.get("scripts"))
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    out.push(gate(
        "gate.no_script_components",
        regex_n == 0 && th_n == 0,
        format!("regex_scripts={regex_n} tavern_helper={th_n}（Studio 卡应为 0/0）"),
    ));

    out.push(gate(
        "gate.spec_v3",
        reimported.spec_version.starts_with('3'),
        format!("spec_version={}（期望 3.x）", reimported.spec_version),
    ));

    // ── forge 差分教训（2026-07-26）加的三维保真：内容字节 / keys 精确 /
    // 插入顺序。计数与路由对了但正文或顺序漂了，卡照样坏——外部预言机
    // （tavern-cards forge）对导入层验出的正是这三类回归。──────────────

    let normalize_content = |s: &str| s.replace("\r\n", "\n").trim().to_string();
    let expected_contents: Vec<String> = artifacts
        .worldview_entries
        .iter()
        .map(|e| normalize_content(&e.content))
        .collect();
    let actual_contents: Vec<String> = book
        .map(|b| {
            b.entries
                .iter()
                .map(|e| normalize_content(&e.content))
                .collect()
        })
        .unwrap_or_default();
    out.push(gate(
        "gate.content_roundtrip",
        actual_contents == expected_contents,
        format!(
            "逐条正文（CRLF/首尾空白归一）比对：{}（顺序敏感）",
            if actual_contents == expected_contents {
                "全一致".to_string()
            } else {
                expected_contents
                    .iter()
                    .zip(actual_contents.iter())
                    .position(|(a, b)| a != b)
                    .map(|i| format!("首个差异在 #{i}"))
                    .unwrap_or_else(|| "条数不同".to_string())
            }
        ),
    ));

    let expected_keys: Vec<Vec<String>> = artifacts
        .worldview_entries
        .iter()
        .map(|e| {
            e.keys
                .iter()
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect()
        })
        .collect();
    let actual_keys: Vec<Vec<String>> = book
        .map(|b| b.entries.iter().map(|e| e.keys.clone()).collect())
        .unwrap_or_default();
    out.push(gate(
        "gate.keys_roundtrip",
        actual_keys == expected_keys,
        format!(
            "逐条触发键精确比对（trim 后）：期望 {} 组，实际 {} 组",
            expected_keys.len(),
            actual_keys.len()
        ),
    ));

    let expected_orders: Vec<i32> = artifacts
        .worldview_entries
        .iter()
        .map(|e| e.order)
        .collect();
    let actual_orders: Vec<i32> = book
        .map(|b| b.entries.iter().map(|e| e.order).collect())
        .unwrap_or_default();
    out.push(gate(
        "gate.insertion_order_roundtrip",
        actual_orders == expected_orders,
        format!("插入顺序（insertion_order）：期望 {expected_orders:?}，实际 {actual_orders:?}"),
    ));

    out
}

/// Embedded Mingyue Qiuqing methodology pack (StoryForge-adapted).
mod pack_mingyue_v1 {
    pub const CREATIVE_PRINCIPLES: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/common/creative_principles.md");
    pub const ABSOLUTE_ZERO: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/common/absolute_zero.md");
    pub const TAG_SPEC: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/common/tag_spec.md");
    pub const WORLDBOOK_CONFIG: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/common/worldbook_config.md");
    pub const OUTPUT_CONTRACT: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/output_contract.md");
    pub const BASIC: &str = include_str!("../assets/cardstudio/mingyue_qiuqing_v1/stages/basic.md");
    pub const PERSONALITY: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/stages/personality.md");
    pub const WORLDVIEW: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/stages/worldview.md");
    pub const OPENING: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/stages/opening.md");
    pub const REVIEW_CONTRACT: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/checkers/review_contract.md");
    pub const WORLDBOOK_EVAL: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/checkers/worldbook_eval.md");
    pub const WORLDVIEW_SELFCHECK: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/checkers/worldview_selfcheck.md");
    pub const ENTRY_SELFCHECK: &str =
        include_str!("../assets/cardstudio/mingyue_qiuqing_v1/checkers/entry_selfcheck.md");
}

/// B-path novel distill prompts (adapted from 明月小说文风蒸馏总结工具; native MVP).
#[allow(non_snake_case)]
mod DISTILL_PACK {
    pub const COMMON: &str = include_str!("../assets/cardstudio/mingyue_distill_v1/common.md");
    pub const PREFILL: &str =
        include_str!("../assets/cardstudio/mingyue_distill_v1/prefill_card.md");
    pub const STYLE: &str = include_str!("../assets/cardstudio/mingyue_distill_v1/style_sample.md");
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

/// Build LLM prompt for methodology review (used after rule checks).
pub fn build_review_prompt(
    project: &CardProject,
    rule_report: &CheckReport,
    user_note: Option<&str>,
) -> (String, String) {
    let mut system = String::new();
    system.push_str("你是 StoryForge 写卡审查器，执行明月秋青方法论与写卡知识库自查清单。\n");
    system.push_str("不要扮演角色，不要输出角色卡正文，只输出审查 JSON。\n\n");
    system.push_str(pack_mingyue_v1::REVIEW_CONTRACT);
    system.push_str("\n\n# 创作原则\n");
    system.push_str(pack_mingyue_v1::CREATIVE_PRINCIPLES);
    system.push_str("\n\n# 绝对零度\n");
    system.push_str(pack_mingyue_v1::ABSOLUTE_ZERO);
    system.push_str("\n\n# 标签规范（软约束）\n");
    system.push_str(pack_mingyue_v1::TAG_SPEC);
    system.push_str("\n\n# 世界书配置指南（摘要审查用）\n");
    // keep prompt bounded: first ~3500 bytes of long guide（char 边界安全）
    system.push_str(truncate_at_char_boundary(
        pack_mingyue_v1::WORLDBOOK_CONFIG,
        3500,
    ));
    system.push_str("\n\n# 世界书评估\n");
    system.push_str(pack_mingyue_v1::WORLDBOOK_EVAL);
    system.push_str("\n\n# 世界观自查\n");
    system.push_str(truncate_at_char_boundary(
        pack_mingyue_v1::WORLDVIEW_SELFCHECK,
        3000,
    ));
    system.push_str("\n\n# 一般条目自查\n");
    system.push_str(truncate_at_char_boundary(
        pack_mingyue_v1::ENTRY_SELFCHECK,
        2500,
    ));

    let mut user = String::new();
    user.push_str(&format!("# Brief\n{}\n\n", project.brief));
    user.push_str(&format!(
        "# Artifacts\n{}\n\n",
        serde_json::to_string_pretty(&project.artifacts).unwrap_or_else(|_| "{}".into())
    ));
    user.push_str(&format!(
        "# 规则检查结果（必须尊重 error 门槛）\n{}\n\n",
        serde_json::to_string_pretty(rule_report).unwrap_or_else(|_| "{}".into())
    ));
    if let Some(n) = user_note.map(str::trim).filter(|s| !s.is_empty()) {
        user.push_str(&format!("# 用户补充\n{n}\n\n"));
    }
    user.push_str("请输出审查 JSON。");
    (system, user)
}

/// Apply a generative stage JSON patch onto artifacts.
pub fn apply_stage_json(
    stage_id: &str,
    artifacts: &mut CardArtifacts,
    value: &serde_json::Value,
) -> Result<(), String> {
    match stage_id {
        STAGE_BASIC => {
            if let Some(s) = value.get("name").and_then(|v| v.as_str())
                && !s.trim().is_empty()
            {
                artifacts.name = s.trim().to_string();
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
                let order = item.get("order").and_then(|v| v.as_i64()).unwrap_or(100) as i32;
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
                    ..WorldviewDraftEntry::default()
                });
            }
            artifacts.worldview_entries = entries;
            if let Some(t) = value.get("world_type").and_then(|v| v.as_str()) {
                artifacts.world_type = Some(t.to_string());
            }
            if let Some(n) = value.get("notes").and_then(|v| v.as_str())
                && !n.trim().is_empty()
            {
                if artifacts.notes.is_empty() {
                    artifacts.notes = n.to_string();
                } else {
                    artifacts.notes = format!("{}\n\n[世界观备注]\n{}", artifacts.notes, n);
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

/// 按字节预算截断到最近的 char 边界。
///
/// 中文常量硬切字节（`&s[..3000]`）在多字节字符中间 panic；且资产文件的
/// CRLF/LF 行尾差异会移动字节偏移——本地（CRLF）不炸不代表 CI（LF）不炸
///（Linux CI 首个完整 rust-test run 抓到的真实跨平台缺陷）。
fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Best-effort extract JSON object from model text.
pub fn extract_json_object(text: &str) -> Result<serde_json::Value, String> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed)
        && v.is_object()
    {
        return Ok(v);
    }
    // fenced ```json
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
        && end > start
    {
        let slice = &trimmed[start..=end];
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(slice)
            && v.is_object()
        {
            return Ok(v);
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
                    ..WorldviewDraftEntry::default()
                },
                WorldviewDraftEntry {
                    keys: vec!["车站".into(), "雨".into()],
                    content: "这座车站总在雨夜汇聚迷路的人。".into(),
                    constant: false,
                    order: 20,
                    ..WorldviewDraftEntry::default()
                },
            ],
            notes: "从零测试".into(),
            personality_mode: Some("guided".into()),
            personality_prompts: vec![],
            world_type: Some("B".into()),
            opening_outline: None,
            style_notes: None,
            extra_definitions: Vec::new(),
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
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.code == "description_required")
        );
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
    fn export_gate_passes_on_faithful_roundtrip() {
        let a = sample_ok_artifacts();
        let compiled = compile_artifacts(&a).expect("compile");
        // domain 层用 from_st_card 模拟 round-trip；真实 infra-import 路径由
        // tauri-app 的 export gate 测试覆盖
        let st_card: StCharacterCard =
            serde_json::from_value(compiled.st_card_json.clone()).expect("st card json");
        let reimported = Character::from_st_card(st_card);
        let checks = export_gate_checks(&a, &reimported);
        let failed: Vec<_> = checks.iter().filter(|c| !c.pass).collect();
        assert!(failed.is_empty(), "gate 应全过: {failed:?}");
    }

    #[test]
    fn export_gate_detects_lost_entries_and_dead_routes() {
        let a = sample_ok_artifacts();
        let compiled = compile_artifacts(&a).expect("compile");
        let mut damaged = compiled.character.clone();
        {
            let book = damaged.embedded_world_info.as_mut().expect("book");
            // 丢一条 + 把选择性条目路由打死
            book.entries.remove(0);
            book.entries[0].route = LoreRoute::Disabled;
        }
        let checks = export_gate_checks(&a, &damaged);
        let by_name = |n: &str| checks.iter().find(|c| c.name == n).expect("check exists");
        assert!(!by_name("gate.book_total").pass);
        assert!(!by_name("gate.constant_entries").pass);
        assert!(!by_name("gate.no_dead_keyword_routes").pass);
        // 无关检查不受牵连
        assert!(by_name("gate.name_roundtrip").pass);
        assert!(by_name("gate.no_script_components").pass);
    }

    #[test]
    fn export_gate_detects_content_keys_and_order_drift() {
        // forge 差分三维：正文字节 / keys 精确 / 插入顺序。
        // 计数与路由全对但这三样漂移的卡必须被闸门拦下。
        let a = sample_ok_artifacts();
        let compiled = compile_artifacts(&a).expect("compile");

        // 正文漂移
        let mut content_drift = compiled.character.clone();
        content_drift.embedded_world_info.as_mut().unwrap().entries[1].content =
            "被悄悄改写的正文".into();
        let checks = export_gate_checks(&a, &content_drift);
        let by_name =
            |cs: &[GateCheck], n: &str| cs.iter().find(|c| c.name == n).expect("check exists").pass;
        assert!(!by_name(&checks, "gate.content_roundtrip"));
        assert!(by_name(&checks, "gate.keys_roundtrip"));
        assert!(by_name(&checks, "gate.insertion_order_roundtrip"));

        // keys 漂移（掉一个触发键，计数类检查全过）
        let mut key_drift = compiled.character.clone();
        key_drift.embedded_world_info.as_mut().unwrap().entries[1]
            .keys
            .pop();
        let checks = export_gate_checks(&a, &key_drift);
        assert!(!by_name(&checks, "gate.keys_roundtrip"));
        assert!(by_name(&checks, "gate.content_roundtrip"));

        // 插入顺序漂移
        let mut order_drift = compiled.character.clone();
        order_drift.embedded_world_info.as_mut().unwrap().entries[0].order = 999;
        let checks = export_gate_checks(&a, &order_drift);
        assert!(!by_name(&checks, "gate.insertion_order_roundtrip"));
        assert!(by_name(&checks, "gate.content_roundtrip"));

        // CRLF 差异是记法不是漂移：归一后应放行
        let mut crlf_only = compiled.character.clone();
        {
            let entry = &mut crlf_only.embedded_world_info.as_mut().unwrap().entries[0];
            entry.content = entry.content.replace('\n', "\r\n") + "\r\n";
        }
        let checks = export_gate_checks(&a, &crlf_only);
        assert!(by_name(&checks, "gate.content_roundtrip"));
    }

    #[test]
    fn export_gate_detects_script_component_pollution() {
        let a = sample_ok_artifacts();
        let compiled = compile_artifacts(&a).expect("compile");
        let mut polluted = compiled.character.clone();
        polluted.extensions.as_object_mut().map(|m| {
            m.insert(
                "regex_scripts".into(),
                serde_json::json!([{"scriptName": "sneaky", "replaceString": ""}]),
            )
        });
        let checks = export_gate_checks(&a, &polluted);
        let comp = checks
            .iter()
            .find(|c| c.name == "gate.no_script_components")
            .expect("check exists");
        assert!(!comp.pass);
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
        assert!(
            system.contains("角色基础")
                || system.contains("template_basic")
                || system.contains("外貌")
        );
        assert!(
            system.contains("只输出一个 JSON")
                || system.contains("JSON 对象")
                || system.contains("输出契约")
        );
        assert!(
            system.len() > 2000,
            "system prompt should embed full stage pack, got {}",
            system.len()
        );
        assert!(user.contains("雨夜车站"));
        assert!(user.contains("当前产物"));
    }

    #[test]
    fn personality_prompt_defaults_to_guided_not_freewrite() {
        let p = CardProject::new_from_scratch("x", "冷淡少年");
        let (system, _) = build_stage_prompt(STAGE_PERSONALITY, &p, None).unwrap();
        assert!(
            system.contains("guided") || system.contains("协作") || system.contains("待用户手写")
        );
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

    #[test]
    fn run_checks_flags_description_personality_bleed_and_bagua() {
        let mut a = sample_ok_artifacts();
        a.description = "性格：温柔体贴。她仿佛小兽一样。".into();
        a.personality = "底色：冷\n衍生：【待用户手写】".into();
        a.personality_mode = Some("guided".into());
        let report = run_checks(&a);
        assert!(report.ok, "no hard errors expected");
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.code == "description_has_personality")
        );
        assert!(report.issues.iter().any(|i| i.code == "bagua_wording"));
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.code == "personality_pending_handwrite")
        );
        assert!(report.score.is_some());
    }

    #[test]
    fn build_review_prompt_includes_selfcheck_assets() {
        let p = CardProject::new_from_scratch("x", "y");
        let report = run_checks(&p.artifacts);
        let (system, user) = build_review_prompt(&p, &report, Some("重点看世界书"));
        assert!(system.contains("审查") || system.contains("自查") || system.contains("评估"));
        assert!(system.len() > 3000);
        assert!(user.contains("规则检查结果"));
        assert!(user.contains("重点看世界书"));
    }

    #[test]
    fn merge_review_reports_demotes_soft_llm_errors() {
        let base = run_checks(&sample_ok_artifacts());
        let llm = serde_json::json!({
            "ok": false,
            "score": 70,
            "summary": "建议润色",
            "issues": [
                {"code": "style", "severity": "error", "message": "文风可更白描", "field": "general", "suggestion": "删形容词"}
            ]
        });
        let merged = merge_review_reports(base, &llm);
        assert!(merged.ok);
        assert!(merged.issues.iter().any(|i| i.code.starts_with("llm_")));
        assert!(merged
            .issues
            .iter()
            .any(|i| i.code.starts_with("llm_") && matches!(i.severity, CheckSeverity::Warning)));
        assert_eq!(merged.source.as_deref(), Some("hybrid"));
    }

    fn sample_character() -> Character {
        let arts = sample_ok_artifacts();
        compile_artifacts(&arts).unwrap().character
    }

    #[test]
    fn reverse_parse_roundtrip_core_fields() {
        let ch = sample_character();
        let arts = reverse_parse_character(&ch);
        assert_eq!(arts.name, "秋青");
        assert!(!arts.description.is_empty());
        assert_eq!(arts.worldview_entries.len(), 2);
        assert!(arts.worldview_entries.iter().any(|e| e.constant));
        assert!(
            arts.worldview_entries
                .iter()
                .any(|e| !e.constant && e.keys.iter().any(|k| k == "车站"))
        );
    }

    #[test]
    fn new_from_existing_sets_mode_and_source() {
        let ch = sample_character();
        let p = CardProject::new_from_existing_character(&ch, Some("stored-1".into()), "");
        assert_eq!(p.mode, CardProjectMode::FromExistingCard);
        assert_eq!(p.source_stored_id.as_deref(), Some("stored-1"));
        assert_eq!(p.source_character_id.as_deref(), Some(ch.id.as_str()));
        assert_eq!(p.current_stage, STAGE_REVIEW);
        assert_eq!(p.artifacts.name, "秋青");
        assert!(p.name.contains("修订"));
        assert!(p.imported_character_id.is_none());
        assert_eq!(
            p.stage_status.get(STAGE_PERSONALITY),
            Some(&StageStatus::Ready)
        );
    }

    #[test]
    fn reverse_parse_recompile_is_new_character_id() {
        let ch = sample_character();
        let arts = reverse_parse_character(&ch);
        let compiled = compile_artifacts(&arts).expect("recompile");
        // 另存语义：编译产物必须是新 domain id，不能回写源卡 id
        assert_ne!(compiled.character.id.as_str(), ch.id.as_str());
        assert_eq!(compiled.character.name, ch.name);
        assert!(compiled.character.tags.iter().any(|t| t == "card-studio"));
    }

    #[test]
    fn sample_novel_excerpts_head_mid_tail() {
        let text = "甲".repeat(30_000);
        let ex = sample_novel_excerpts(&text, 1000);
        assert!(ex.len() >= 2);
        assert!(ex[0].chars().count() <= 1000);
        let short = sample_novel_excerpts("短篇", 1000);
        assert_eq!(short, vec!["短篇".to_string()]);
    }

    #[test]
    fn new_from_novel_sets_mode_and_excerpts() {
        let novel = format!(
            "第一章\n{}\n中间转折\n{}\n结局\n{}",
            "雨".repeat(4000),
            "雪".repeat(4000),
            "风".repeat(4000)
        );
        let p = CardProject::new_from_novel("改编试作", "想做旅人卡", "夜行录", novel);
        assert_eq!(p.mode, CardProjectMode::FromNovel);
        assert_eq!(p.current_stage, STAGE_BASIC);
        assert!(!p.novel_excerpts.is_empty());
        assert_eq!(p.novel_title.as_deref(), Some("夜行录"));
        // 1.2 万字级仍保留全文
        assert!(p.novel_text.is_some());
        let prompt = build_novel_prefill_prompt(&p).expect("prefill prompt");
        assert!(prompt.contains("小说改编写卡预填师") || prompt.contains("预填"));
        assert!(prompt.contains("夜行录"));
    }

    #[test]
    fn large_novel_drops_full_text_keeps_excerpts() {
        let novel = "章".repeat(100_000);
        let p = CardProject::new_from_novel("大书", "", "巨著", novel);
        assert!(p.novel_text.is_none());
        assert!(!p.novel_excerpts.is_empty());
        assert!(build_novel_prefill_prompt(&p).is_ok());
    }

    /// 字节预算截断必须落在 char 边界（CRLF/LF 漂移下任意偏移都不 panic）。
    #[test]
    fn truncate_at_char_boundary_never_splits_multibyte() {
        let text = "设定".repeat(1_000); // 每字 3 字节
        for budget in 0..32 {
            let cut = truncate_at_char_boundary(&text, budget);
            assert!(cut.len() <= budget);
            assert!(text.starts_with(cut));
        }
        assert_eq!(truncate_at_char_boundary("短", 100), "短");
        assert_eq!(truncate_at_char_boundary("", 10), "");
        // 关键回归：审查 prompt 装配对任意资产字节数不 panic
        let report = CheckReport {
            ok: true,
            issues: vec![],
            score: None,
            summary: None,
            source: None,
        };
        let project = CardProject::new_from_scratch("边界", "测试");
        let _ = build_review_prompt(&project, &report, None);
    }

    /// Phase 3 骨架：InitVar 注入走普通世界书条目通道（闸门口径零改动）。
    #[test]
    fn mvu_bootstrap_entry_injects_and_replaces_idempotently() {
        let mut arts = CardArtifacts {
            name: "系统卡".into(),
            worldview_entries: vec![WorldviewDraftEntry {
                content: "常驻设定。".into(),
                constant: true,
                order: 10,
                ..WorldviewDraftEntry::default()
            }],
            ..CardArtifacts::default()
        };
        apply_mvu_bootstrap_entry(&mut arts, "主角:\n  hp: 100\n  好感度: 0\n")
            .expect("合法 YAML 注入成功");
        assert_eq!(arts.worldview_entries.len(), 2);
        let entry = &arts.worldview_entries[1];
        assert!(entry.constant);
        assert!(entry.content.starts_with(MVU_INITVAR_MARKER));
        assert!(entry.content.contains("hp: 100"));
        assert_eq!(entry.order, 20, "order = 现有最大 + 10");

        // 幂等：再注入替换而非追加，order 保持
        apply_mvu_bootstrap_entry(&mut arts, "主角:\n  hp: 50\n").unwrap();
        assert_eq!(arts.worldview_entries.len(), 2);
        assert!(arts.worldview_entries[1].content.contains("hp: 50"));
        assert_eq!(arts.worldview_entries[1].order, 20);

        // 非 mapping / 空 YAML 拒绝
        assert!(apply_mvu_bootstrap_entry(&mut arts, "只是一句话").is_err());
        assert!(apply_mvu_bootstrap_entry(&mut arts, "  ").is_err());

        // 注入后可正常编译（InitVar 条目进 ST 世界书）
        arts.description = "测试描述。".into();
        arts.first_mes = "开场。".into();
        let compiled = compile_artifacts(&arts).expect("编译成功");
        let book = compiled.character.embedded_world_info.expect("世界书存在");
        assert!(
            book.entries
                .iter()
                .any(|e| e.content.starts_with(MVU_INITVAR_MARKER))
        );
    }

    /// Phase 3 骨架：附加定义经 extensions 通道 round-trip。
    #[test]
    fn extra_definitions_round_trip_via_st_extensions() {
        let arts = CardArtifacts {
            name: "多角色卡".into(),
            description: "两个角色。".into(),
            first_mes: "开场。".into(),
            extra_definitions: vec![
                ExtraDefinitionDraft {
                    name: "艾琳".into(),
                    persona_prompt: "剑士，直率。".into(),
                    behavior_rules: "先动手后动口。".into(),
                    group: Some("主角团".into()),
                },
                ExtraDefinitionDraft {
                    name: "  ".into(), // 空名应被提取端过滤
                    ..ExtraDefinitionDraft::default()
                },
            ],
            ..CardArtifacts::default()
        };
        let compiled = compile_artifacts(&arts).expect("编译成功");
        let extensions = &compiled.st_card_json["data"]["extensions"];
        let extracted = extra_definitions_from_st_extensions(extensions);
        assert_eq!(extracted.len(), 1, "空名草案被过滤");
        assert_eq!(extracted[0].name, "艾琳");
        assert_eq!(extracted[0].group.as_deref(), Some("主角团"));

        // 无附加定义时 extensions 不带该键、提取返回空
        let plain = CardArtifacts {
            name: "单角色".into(),
            description: "d".into(),
            first_mes: "f".into(),
            ..CardArtifacts::default()
        };
        let plain_compiled = compile_artifacts(&plain).expect("编译成功");
        let ext = &plain_compiled.st_card_json["data"]["extensions"];
        assert!(ext["storyforge"].get("extra_definitions").is_none());
        assert!(extra_definitions_from_st_extensions(ext).is_empty());
    }

    /// Phase 3 骨架：世界书高级策略字段进 entry.extensions。
    #[test]
    fn worldview_advanced_fields_land_in_entry_extensions() {
        let arts = CardArtifacts {
            name: "策略卡".into(),
            description: "d".into(),
            first_mes: "f".into(),
            worldview_entries: vec![WorldviewDraftEntry {
                keys: vec!["暗号".into()],
                content: "概率触发条目。".into(),
                constant: false,
                order: 10,
                probability: Some(30),
                exclude_recursion: Some(true),
                group: Some("支线".into()),
            }],
            ..CardArtifacts::default()
        };
        let compiled = compile_artifacts(&arts).expect("编译成功");
        let book = compiled.character.embedded_world_info.expect("世界书");
        let ext = &book.entries[0].extensions;
        assert_eq!(ext["probability"], 30);
        assert_eq!(ext["useProbability"], true);
        assert_eq!(ext["exclude_recursion"], true);
        assert_eq!(ext["group"], "支线");
    }

    #[test]
    fn apply_novel_prefill_json_fills_worldview_and_style() {
        let mut arts = CardArtifacts::default();
        let v = serde_json::json!({
            "name": "旅人",
            "description": "斗笠遮脸的旅人。",
            "personality": "寡言，但守诺。",
            "scenario": "山道客栈。",
            "first_mes": "他将湿斗笠挂到一边：…坐。",
            "style_notes": "短句，白描，少形容词。",
            "worldview_entries": [
                {"keys": [], "content": "此世灵雨按节气降落。", "constant": true, "order": 10},
                {"keys": ["客栈"], "content": "山道客栈收留失名旅人。", "constant": false, "order": 20}
            ],
            "secondary_characters": ["掌柜：只问来处不问去处"]
        });
        apply_novel_prefill_json(&mut arts, &v).unwrap();
        assert_eq!(arts.name, "旅人");
        assert_eq!(arts.worldview_entries.len(), 2);
        assert!(arts.style_notes.as_deref().unwrap_or("").contains("白描"));
        assert!(arts.notes.contains("配角"));
    }
}
