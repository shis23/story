//! Tauri commands for Card Studio Phase 1 (from-scratch drafting).

use std::sync::Arc;

use crate::card_studio_store::CardStudioStore;
use crate::error::TauriCommandError;
use crate::{AppState, CharacterInfo, CharacterSummary};
use serde::{Deserialize, Serialize};
use storyforge_domain::card_studio::{
    self, CardArtifacts, CardProject, CheckReport, GateCheck, STAGE_BASIC, STAGE_BRIEF,
    STAGE_COMPILE_IMPORT, STAGE_OPENING, STAGE_PERSONALITY, STAGE_REVIEW, STAGE_WORLDVIEW,
    StageStatus, apply_novel_prefill_json, apply_stage_json, build_novel_prefill_prompt,
    build_novel_style_prompt, build_review_prompt, build_stage_prompt, compile_artifacts,
    export_gate_checks, extract_json_object, merge_review_reports, phase1_stage_ids, run_checks,
};
use storyforge_domain::character::{
    CharacterCard, CharacterDefinition, CharacterExtractionStatus, StCharacterCard,
};
use storyforge_domain::llm::{ChatMessage, ChatRequest, SamplingParams};

fn get_card_studio_store() -> &'static CardStudioStore {
    crate::get_card_studio_store()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardProjectSummaryDto {
    pub id: String,
    pub name: String,
    pub brief: String,
    pub current_stage: String,
    pub updated_at: String,
    pub imported_character_id: Option<String>,
    pub mode: String,
    pub source_character_id: Option<String>,
}

impl From<&CardProject> for CardProjectSummaryDto {
    fn from(p: &CardProject) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            brief: p.brief.clone(),
            current_stage: p.current_stage.clone(),
            updated_at: p.updated_at.clone(),
            imported_character_id: p.imported_character_id.clone(),
            mode: match p.mode {
                storyforge_domain::card_studio::CardProjectMode::FromScratch => {
                    "from_scratch".into()
                }
                storyforge_domain::card_studio::CardProjectMode::FromNovel => "from_novel".into(),
                storyforge_domain::card_studio::CardProjectMode::FromExistingCard => {
                    "from_existing_card".into()
                }
            },
            source_character_id: p.source_character_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilePreviewDto {
    pub st_card_json: serde_json::Value,
    pub warnings: Vec<String>,
    pub character_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportCompiledResultDto {
    pub character: CharacterSummary,
    pub card_id: String,
    /// domain Character.id（tool_ctx / extract_characters 的精确主键；
    /// character.id 是 StoredCharacter 存储 id，两者无关）
    pub source_character_id: String,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub fn cardstudio_list_projects() -> Vec<CardProjectSummaryDto> {
    get_card_studio_store()
        .list()
        .iter()
        .map(CardProjectSummaryDto::from)
        .collect()
}

#[tauri::command]
pub fn cardstudio_create_project(
    name: String,
    brief: String,
) -> Result<CardProject, TauriCommandError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(TauriCommandError::validation("项目名不能为空"));
    }
    let project = CardProject::new_from_scratch(name, brief);
    get_card_studio_store()
        .insert(project)
        .map_err(TauriCommandError::storage)
}

/// B path: create a project from novel text (paste/import). Prefill is a separate LLM call.
#[tauri::command]
pub fn cardstudio_create_from_novel(
    name: String,
    brief: String,
    novel_title: String,
    novel_text: String,
) -> Result<CardProject, TauriCommandError> {
    let novel_text = novel_text.trim();
    if novel_text.is_empty() {
        return Err(TauriCommandError::validation("小说正文不能为空"));
    }
    // Soft cap to keep store/prompt sane in MVP; users can truncate or split later.
    if novel_text.chars().count() > 400_000 {
        return Err(TauriCommandError::validation(
            "小说正文过长（>40万字）。MVP 请先粘贴节选，或后续接外置文档切分。",
        ));
    }
    let name = if name.trim().is_empty() {
        if novel_title.trim().is_empty() {
            "小说改编".into()
        } else {
            format!("{}（改编）", novel_title.trim())
        }
    } else {
        name.trim().to_string()
    };
    let project = CardProject::new_from_novel(name, brief, novel_title, novel_text);
    get_card_studio_store()
        .insert(project)
        .map_err(TauriCommandError::storage)
}

/// C path: open an existing imported character as a revise CardProject.
///
/// `character_id` may be CharacterStore id or domain source_character_id.
///
/// 走 facade（双后端等价）：`storage().get_character` 对 JSON 直查 CharacterStore、
/// 对 SQLite 走 `characters` 表，行为与 `list_characters`/`get_character` 命令一致；
/// 不得在 SQLite 激活时直连 JSON-only 的 `json_character_store`，否则命令会失败。
#[tauri::command]
pub fn cardstudio_create_from_character(
    character_id: String,
    brief: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CardProject, TauriCommandError> {
    state
        .storage()
        .require_supported(
            crate::storage_backend::BackendCapability::CharacterCommands,
            "create Card Studio project from character",
        )
        .map_err(TauriCommandError::validation)?;
    let stored = state
        .storage()
        .get_character(&character_id)
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))?;
    let character = crate::stored_info_to_character(&stored);

    // Ensure tool_ctx has it (best effort) so later extract/import paths stay consistent.
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        if !ctx
            .characters
            .iter()
            .any(|c| c.id.as_str() == character.id.as_str() || c.name == character.name)
        {
            ctx.characters.push(Arc::new(character.clone()));
        }
    }

    let project = CardProject::new_from_existing_character(
        &character,
        Some(stored.id.clone()),
        brief.unwrap_or_default(),
    );
    get_card_studio_store()
        .insert(project)
        .map_err(TauriCommandError::storage)
}

#[tauri::command]
pub fn cardstudio_get_project(id: String) -> Result<CardProject, TauriCommandError> {
    get_card_studio_store()
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))
}

#[tauri::command]
pub fn cardstudio_delete_project(id: String) -> Result<bool, TauriCommandError> {
    get_card_studio_store()
        .delete(&id)
        .map_err(TauriCommandError::storage)
}

#[tauri::command]
pub fn cardstudio_update_artifacts(
    id: String,
    artifacts: CardArtifacts,
) -> Result<CardProject, TauriCommandError> {
    let store = get_card_studio_store();
    let mut project = store
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;
    project.artifacts = artifacts;
    if !project.artifacts.name.trim().is_empty() {
        project.name = project.artifacts.name.trim().to_string();
    }
    project.touch();
    store.update(project).map_err(TauriCommandError::storage)
}

#[tauri::command]
pub fn cardstudio_set_stage(
    id: String,
    stage_id: String,
) -> Result<CardProject, TauriCommandError> {
    if !phase1_stage_ids().contains(&stage_id.as_str()) {
        return Err(TauriCommandError::validation(format!(
            "未知阶段: {stage_id}"
        )));
    }
    let store = get_card_studio_store();
    let mut project = store
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;
    project.current_stage = stage_id;
    project.touch();
    store.update(project).map_err(TauriCommandError::storage)
}

/// Update project options (methodology switches).
#[tauri::command]
pub fn cardstudio_set_options(
    id: String,
    allow_ai_freewrite: Option<bool>,
    stage_pack_id: Option<String>,
) -> Result<CardProject, TauriCommandError> {
    let store = get_card_studio_store();
    let mut project = store
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;
    if let Some(v) = allow_ai_freewrite {
        project.allow_ai_freewrite = v;
    }
    if let Some(pack) = stage_pack_id {
        let pack = pack.trim();
        if !pack.is_empty() {
            // Phase 1 only ships mingyue_qiuqing_v1; reject unknown packs early.
            if pack != "mingyue_qiuqing_v1" {
                return Err(TauriCommandError::validation(format!(
                    "未知 stage pack: {pack}（当前仅支持 mingyue_qiuqing_v1）"
                )));
            }
            project.stage_pack_id = pack.to_string();
        }
    }
    project.touch();
    store.update(project).map_err(TauriCommandError::storage)
}

#[tauri::command]
pub fn cardstudio_run_checks(id: String) -> Result<CheckReport, TauriCommandError> {
    let project = get_card_studio_store()
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;
    Ok(run_checks(&project.artifacts))
}

/// Rule checks + optional methodology LLM review (hybrid).
#[tauri::command]
pub async fn cardstudio_run_review(
    id: String,
    user_note: Option<String>,
    use_llm: Option<bool>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CheckReport, TauriCommandError> {
    let store = get_card_studio_store();
    let mut project = store
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;

    let rule_report = run_checks(&project.artifacts);
    let use_llm = use_llm.unwrap_or(true);
    if !use_llm {
        project.last_stage_output =
            Some(serde_json::to_string_pretty(&rule_report).unwrap_or_else(|_| "{}".into()));
        project.touch();
        let _ = store.update(project);
        return Ok(rule_report);
    }

    let (system, user) = build_review_prompt(&project, &rule_report, user_note.as_deref());
    let llm = state.require_active_llm()?;
    let params = crate::get_conn_store()
        .active_connection()
        .map(|c| c.params)
        .unwrap_or_else(|| SamplingParams {
            temperature: Some(0.3),
            max_tokens: Some(4096),
            max_tokens_explicit: true,
            ..Default::default()
        });
    let model = crate::get_conn_store()
        .active_connection()
        .map(|c| c.model)
        .unwrap_or_else(|| "mock".into());
    let req = ChatRequest {
        messages: vec![ChatMessage::system(system), ChatMessage::user(user)],
        tools: None,
        params,
        model,
    };
    let resp = llm
        .chat(&req)
        .await
        .map_err(|e| TauriCommandError::llm(format!("写卡审查 LLM 调用失败: {e}"), false))?;
    project.last_stage_output = Some(resp.content.clone());

    let merged = match extract_json_object(&resp.content) {
        Ok(v) => merge_review_reports(rule_report, &v),
        Err(_) => {
            let mut report = rule_report;
            report.summary = Some(format!(
                "{}（LLM 审查输出无法解析，已回退规则检查）",
                report.summary.unwrap_or_default()
            ));
            report.source = Some("rule".into());
            report
        }
    };

    // Persist last review summary into notes lightly
    if let Some(summary) = &merged.summary {
        project.last_error = if merged.ok {
            None
        } else {
            Some(summary.clone())
        };
    }
    project.touch();
    let _ = store.update(project);
    Ok(merged)
}

#[tauri::command]
pub fn cardstudio_compile(id: String) -> Result<CompilePreviewDto, TauriCommandError> {
    let project = get_card_studio_store()
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;
    let compiled = compile_artifacts(&project.artifacts).map_err(TauriCommandError::validation)?;
    Ok(CompilePreviewDto {
        st_card_json: compiled.st_card_json,
        warnings: compiled.warnings,
        character_name: compiled.character.name,
    })
}

/// 出卡质量闸门报告：JSON 与 PNG 两条导出路径各自 round-trip 后的确定性检查。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportGateReportDto {
    pub pass: bool,
    pub character_name: String,
    pub warnings: Vec<String>,
    pub json_checks: Vec<GateCheck>,
    pub png_checks: Vec<GateCheck>,
}

/// 编译产物 → ST JSON / PNG 字节 → 真实 infra-import 导入路径 round-trip →
/// domain 确定性检查。与 harness card_translation 的确定性验收同语义，
/// 作为写卡线的出卡质量闸门（不依赖 harness crate）。
fn run_export_gate(project: &CardProject) -> Result<ExportGateReportDto, String> {
    let compiled = compile_artifacts(&project.artifacts)?;
    let st_card: StCharacterCard = serde_json::from_value(compiled.st_card_json.clone())
        .map_err(|e| format!("ST 卡 JSON 反序列化失败: {e}"))?;

    let json_bytes = serde_json::to_vec(&compiled.st_card_json)
        .map_err(|e| format!("ST 卡 JSON 序列化失败: {e}"))?;
    let json_reimported = storyforge_infra_import::import_character(&json_bytes)
        .map_err(|e| format!("JSON round-trip 导入失败: {e}"))?;
    let json_checks = export_gate_checks(&project.artifacts, &json_reimported);

    let png_bytes = storyforge_infra_import::png::write_st_card_png(&st_card, None)
        .map_err(|e| format!("PNG 导出失败: {e}"))?;
    let png_reimported = storyforge_infra_import::import_character(&png_bytes)
        .map_err(|e| format!("PNG round-trip 导入失败: {e}"))?;
    let png_checks = export_gate_checks(&project.artifacts, &png_reimported);

    let pass = json_checks.iter().all(|c| c.pass) && png_checks.iter().all(|c| c.pass);
    Ok(ExportGateReportDto {
        pass,
        character_name: compiled.character.name,
        warnings: compiled.warnings,
        json_checks,
        png_checks,
    })
}

#[tauri::command]
pub fn cardstudio_export_gate(id: String) -> Result<ExportGateReportDto, TauriCommandError> {
    let project = get_card_studio_store()
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;
    run_export_gate(&project).map_err(TauriCommandError::validation)
}

/// 导出编译产物为 ST PNG 卡（与 export_st_card_png 同一 PNG 写入层，
/// 但源头是 Studio 编译产物而非 CharacterStore 已存卡）。
#[tauri::command]
pub fn cardstudio_export_png(id: String) -> Result<Vec<u8>, TauriCommandError> {
    let project = get_card_studio_store()
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;
    let compiled = compile_artifacts(&project.artifacts).map_err(TauriCommandError::validation)?;
    let st_card: StCharacterCard = serde_json::from_value(compiled.st_card_json)
        .map_err(|e| TauriCommandError::internal(format!("ST 卡 JSON 反序列化失败: {e}")))?;
    storyforge_infra_import::png::write_st_card_png(&st_card, None)
        .map_err(|e| TauriCommandError::internal(format!("PNG 导出失败: {e}")))
}

/// Mark non-LLM stages done / ready transitions without model calls.
#[tauri::command]
pub fn cardstudio_complete_manual_stage(
    id: String,
    stage_id: String,
) -> Result<CardProject, TauriCommandError> {
    let store = get_card_studio_store();
    let mut project = store
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;

    match stage_id.as_str() {
        STAGE_BRIEF => {
            if project.brief.trim().is_empty() && project.artifacts.notes.trim().is_empty() {
                return Err(TauriCommandError::validation("请先填写 brief / 创作意图"));
            }
            project.set_stage_status(STAGE_BRIEF, StageStatus::Done);
            project.set_stage_status(STAGE_BASIC, StageStatus::Ready);
            project.current_stage = STAGE_BASIC.to_string();
        }
        STAGE_REVIEW => {
            let report = run_checks(&project.artifacts);
            if !report.ok {
                project.set_stage_status(STAGE_REVIEW, StageStatus::Failed);
                project.last_error = Some(
                    report
                        .issues
                        .iter()
                        .filter(|i| matches!(i.severity, card_studio::CheckSeverity::Error))
                        .map(|i| i.message.clone())
                        .collect::<Vec<_>>()
                        .join("; "),
                );
                let _ = store.update(project.clone());
                return Err(TauriCommandError::validation(
                    project
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "检查未通过".into()),
                ));
            }
            project.set_stage_status(STAGE_REVIEW, StageStatus::Done);
            project.set_stage_status(STAGE_COMPILE_IMPORT, StageStatus::Ready);
            project.current_stage = STAGE_COMPILE_IMPORT.to_string();
            project.last_error = None;
        }
        other => {
            return Err(TauriCommandError::validation(format!(
                "阶段 {other} 请使用 run_stage 或 compile/import"
            )));
        }
    }
    project.touch();
    store.update(project).map_err(TauriCommandError::storage)
}

/// B path: LLM prefill card artifacts from novel excerpts.
#[tauri::command]
pub async fn cardstudio_prefill_from_novel(
    id: String,
    user_note: Option<String>,
    include_style: Option<bool>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CardProject, TauriCommandError> {
    let store = get_card_studio_store();
    let mut project = store
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;

    if !matches!(
        project.mode,
        storyforge_domain::card_studio::CardProjectMode::FromNovel
    ) {
        return Err(TauriCommandError::validation(
            "仅小说改编项目可执行 prefill_from_novel",
        ));
    }

    let mut prompt = build_novel_prefill_prompt(&project).map_err(TauriCommandError::validation)?;
    if let Some(note) = user_note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        prompt.push_str("\n\n## 用户补充\n");
        prompt.push_str(note);
    }

    let llm = state.require_active_llm()?;
    let params = crate::get_conn_store()
        .active_connection()
        .map(|c| c.params)
        .unwrap_or_else(|| SamplingParams {
            temperature: Some(0.7),
            max_tokens: Some(8192),
            max_tokens_explicit: true,
            ..Default::default()
        });
    let model = crate::get_conn_store()
        .active_connection()
        .map(|c| c.model)
        .unwrap_or_else(|| "mock".into());
    let req = ChatRequest {
        messages: vec![
            ChatMessage::system(
                "你是 StoryForge Card Studio 的小说改编预填助手。严格按用户提示中的 <content> JSON 协议输出。",
            ),
            ChatMessage::user(prompt),
        ],
        tools: None,
        params: params.clone(),
        model: model.clone(),
    };
    let resp = llm
        .chat(&req)
        .await
        .map_err(|e| TauriCommandError::llm(format!("小说预填失败: {e}"), false))?;
    let raw = resp.content;
    project.last_stage_output = Some(raw.clone());
    let json = extract_json_object(&raw).map_err(|e| {
        project.last_error = Some(e.clone());
        let _ = store.update(project.clone());
        TauriCommandError::validation(format!("预填 JSON 解析失败: {e}"))
    })?;
    if let Err(e) = apply_novel_prefill_json(&mut project.artifacts, &json) {
        project.last_error = Some(e.clone());
        let _ = store.update(project.clone());
        return Err(TauriCommandError::validation(e));
    }

    // Optional style sample (best-effort; failures do not abort prefill).
    if include_style.unwrap_or(true)
        && let Ok(style_prompt) = build_novel_style_prompt(&project)
    {
        let style_req = ChatRequest {
            messages: vec![
                ChatMessage::system("你是文风蒸馏师。只输出 <content> 内的文风公式。"),
                ChatMessage::user(style_prompt),
            ],
            tools: None,
            params: SamplingParams {
                temperature: Some(0.5),
                max_tokens: Some(4096),
                max_tokens_explicit: true,
                ..Default::default()
            },
            model,
        };
        if let Ok(style_resp) = llm.chat(&style_req).await {
            let style_raw = style_resp.content;
            let style_body = style_raw
                .split("<content>")
                .nth(1)
                .and_then(|s| s.split("</content>").next())
                .unwrap_or(style_raw.as_str())
                .trim()
                .to_string();
            if !style_body.is_empty() {
                project.artifacts.style_notes = Some(style_body.clone());
                if !project.artifacts.notes.contains("[文风笔记]") {
                    project.artifacts.notes = format!(
                        "{}\n\n[文风笔记]\n{}",
                        project.artifacts.notes.trim(),
                        style_body
                    )
                    .trim()
                    .to_string();
                }
            }
        }
    }

    if !project.artifacts.name.trim().is_empty() {
        project.name = project.artifacts.name.trim().to_string();
    }
    // After prefill, generative stages are ready for selective polish.
    for sid in [
        STAGE_BASIC,
        STAGE_PERSONALITY,
        STAGE_WORLDVIEW,
        STAGE_OPENING,
    ] {
        project.set_stage_status(sid, StageStatus::Ready);
    }
    project.set_stage_status(STAGE_REVIEW, StageStatus::Ready);
    project.current_stage = STAGE_REVIEW.to_string();
    project.last_error = None;
    project.touch();
    store.update(project).map_err(TauriCommandError::storage)
}

#[tauri::command]
pub async fn cardstudio_run_stage(
    id: String,
    stage_id: String,
    user_note: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CardProject, TauriCommandError> {
    match stage_id.as_str() {
        STAGE_BASIC | STAGE_PERSONALITY | STAGE_WORLDVIEW | STAGE_OPENING => {}
        STAGE_BRIEF | STAGE_REVIEW => {
            return Err(TauriCommandError::validation(
                "该阶段请使用 cardstudio_complete_manual_stage",
            ));
        }
        STAGE_COMPILE_IMPORT => {
            return Err(TauriCommandError::validation(
                "请使用 cardstudio_import_compiled",
            ));
        }
        other => {
            return Err(TauriCommandError::validation(format!("未知阶段: {other}")));
        }
    }

    let store = get_card_studio_store();
    let mut project = store
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;

    let (system, user) = build_stage_prompt(&stage_id, &project, user_note.as_deref())
        .map_err(TauriCommandError::validation)?;

    let llm = state.require_active_llm()?;
    let params = crate::get_conn_store()
        .active_connection()
        .map(|c| c.params)
        .unwrap_or_else(|| SamplingParams {
            temperature: Some(0.7),
            max_tokens: Some(4096),
            max_tokens_explicit: true,
            ..Default::default()
        });
    let model = crate::get_conn_store()
        .active_connection()
        .map(|c| c.model)
        .unwrap_or_else(|| "mock".into());

    let req = ChatRequest {
        messages: vec![ChatMessage::system(system), ChatMessage::user(user)],
        tools: None,
        params,
        model,
    };

    let resp = llm
        .chat(&req)
        .await
        .map_err(|e| TauriCommandError::llm(format!("写卡阶段 LLM 调用失败: {e}"), false))?;

    project.last_stage_output = Some(resp.content.clone());

    let json = match extract_json_object(&resp.content) {
        Ok(v) => v,
        Err(e) => {
            project.set_stage_status(&stage_id, StageStatus::Failed);
            project.last_error = Some(e.clone());
            let _ = store.update(project);
            return Err(TauriCommandError::validation(e));
        }
    };

    if let Err(e) = apply_stage_json(&stage_id, &mut project.artifacts, &json) {
        project.set_stage_status(&stage_id, StageStatus::Failed);
        project.last_error = Some(e.clone());
        let _ = store.update(project);
        return Err(TauriCommandError::validation(e));
    }

    if !project.artifacts.name.trim().is_empty() {
        project.name = project.artifacts.name.trim().to_string();
    }

    project.set_stage_status(&stage_id, StageStatus::Done);
    project.last_error = None;

    // Advance current stage pointer
    let next = match stage_id.as_str() {
        STAGE_BASIC => Some(STAGE_PERSONALITY),
        STAGE_PERSONALITY => Some(STAGE_WORLDVIEW),
        STAGE_WORLDVIEW => Some(STAGE_OPENING),
        STAGE_OPENING => Some(STAGE_REVIEW),
        _ => None,
    };
    if let Some(n) = next {
        project.set_stage_status(n, StageStatus::Ready);
        project.current_stage = n.to_string();
    }

    project.touch();
    store.update(project).map_err(TauriCommandError::storage)
}

#[tauri::command]
pub fn cardstudio_import_compiled(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<ImportCompiledResultDto, TauriCommandError> {
    state
        .storage()
        .require_supported(
            crate::storage_backend::BackendCapability::CharacterCommands,
            "import compiled Card Studio character",
        )
        .map_err(TauriCommandError::validation)?;
    let store = get_card_studio_store();
    let mut project = store
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;

    let compiled = compile_artifacts(&project.artifacts).map_err(TauriCommandError::validation)?;
    let character = compiled.character;
    let warnings = compiled.warnings;

    // Always 另存: CharacterStore::save allocates a new stored id; never overwrite source.
    let is_revise = matches!(
        project.mode,
        storyforge_domain::card_studio::CardProjectMode::FromExistingCard
    );
    let info = CharacterInfo::from(&character);
    let stored = state
        .storage()
        .save_character(info)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;

    // Sync tool_ctx so extract_characters / campaign can see the card.
    // Dedup by domain id only — never drop another card merely because names match.
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        let new_id = character.id.as_str().to_string();
        ctx.characters.retain(|c| c.id.as_str() != new_id.as_str());
        let world_info = character.embedded_world_info.clone();
        ctx.characters.push(Arc::new(character.clone()));
        if let Some(wi) = world_info {
            ctx.world_info = Some(Arc::new(wi));
        }
    }

    // Seed a fallback CharacterCard so list_cards can open campaigns without forced LLM extract.
    // CharacterCard::from_character uses a fresh card id + this character.id as source_character_id,
    // so revise re-import lands as a new playable card (另存), not an overwrite of the source card.
    let mut card = CharacterCard::from_character(&character);
    let def = CharacterDefinition::fallback_from_character(&character, &[]);
    let definitions = storyforge_app_agent::attach_definitions_to_card(vec![def], &card.id);
    card.character_definitions = definitions;
    card.extraction_status = CharacterExtractionStatus::Fallback;
    card.extraction_message = Some(if is_revise {
        "Card Studio 修订另存导入，已生成单主角定义；原卡未覆盖，可稍后重新识别。".into()
    } else {
        "Card Studio 从零导入，已生成单主角定义，可稍后重新识别。".into()
    });
    let stored_card = state
        .storage()
        .save_card(card)
        .map_err(|e| TauriCommandError::storage(format!("保存 CharacterCard 失败: {e}")))?;

    project.imported_character_id = Some(stored.id.clone());
    project.set_stage_status(STAGE_COMPILE_IMPORT, StageStatus::Done);
    project.current_stage = STAGE_COMPILE_IMPORT.to_string();
    project.last_error = None;
    project.touch();
    let _ = store.update(project);

    Ok(ImportCompiledResultDto {
        character: CharacterSummary::from(stored),
        card_id: stored_card.card.id.as_str().to_string(),
        source_character_id: character.id.as_str().to_string(),
        warnings,
    })
}

#[tauri::command]
pub fn cardstudio_list_stages() -> Vec<String> {
    phase1_stage_ids()
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::card_studio::{CardProjectMode, WorldviewDraftEntry};

    fn gate_test_project() -> CardProject {
        let mut project = CardProject::new_from_scratch("gate-demo", "出卡闸门测试");
        project.mode = CardProjectMode::FromScratch;
        project.artifacts = CardArtifacts {
            name: "闸门测试卡".into(),
            description: "用于出卡质量闸门 round-trip 的最小卡。".into(),
            personality: "沉稳，先观察后开口。".into(),
            scenario: "黄昏的天台。".into(),
            first_mes: "风把她的话吹散了一半：……你来了。".into(),
            tags: vec!["测试".into()],
            creator: "gate-test".into(),
            worldview_entries: vec![
                WorldviewDraftEntry {
                    keys: vec![],
                    content: "常驻设定：城市上空漂着看不见的岛。".into(),
                    constant: true,
                    order: 10,
                    ..WorldviewDraftEntry::default()
                },
                WorldviewDraftEntry {
                    keys: vec!["天台".into(), "岛".into()],
                    content: "天台是离岛最近的地方。".into(),
                    constant: false,
                    order: 20,
                    ..WorldviewDraftEntry::default()
                },
            ],
            ..Default::default()
        };
        project
    }

    #[test]
    fn export_gate_roundtrips_json_and_png_through_real_import() {
        let project = gate_test_project();
        let report = run_export_gate(&project).expect("gate 应能运行");
        let failed: Vec<_> = report
            .json_checks
            .iter()
            .chain(report.png_checks.iter())
            .filter(|c| !c.pass)
            .map(|c| format!("{}: {}", c.name, c.detail))
            .collect();
        assert!(report.pass, "出卡闸门未过:\n{}", failed.join("\n"));
        assert_eq!(report.character_name, "闸门测试卡");
        assert!(!report.json_checks.is_empty());
        assert!(!report.png_checks.is_empty());
    }

    #[test]
    fn export_gate_rejects_uncompilable_project() {
        let mut project = gate_test_project();
        project.artifacts.first_mes.clear();
        let err = run_export_gate(&project).expect_err("缺 first_mes 应编译失败");
        assert!(err.contains("编译前检查失败"), "err={err}");
    }

    #[test]
    fn export_png_bytes_reimport_as_same_card() {
        let project = gate_test_project();
        let compiled = compile_artifacts(&project.artifacts).expect("compile");
        let st_card: StCharacterCard =
            serde_json::from_value(compiled.st_card_json).expect("st card json");
        let png =
            storyforge_infra_import::png::write_st_card_png(&st_card, None).expect("png export");
        let reimported = storyforge_infra_import::import_character(&png).expect("png reimport");
        assert_eq!(reimported.name, "闸门测试卡");
        assert_eq!(
            reimported
                .embedded_world_info
                .as_ref()
                .map(|b| b.entries.len()),
            Some(2)
        );
    }
}
