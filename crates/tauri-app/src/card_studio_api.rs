//! Tauri commands for Card Studio Phase 1 (from-scratch drafting).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use storyforge_domain::card_studio::{
    self, apply_stage_json, build_stage_prompt, compile_artifacts, extract_json_object, phase1_stage_ids,
    run_checks, CardArtifacts, CardProject, CheckReport, StageStatus, STAGE_BASIC, STAGE_BRIEF,
    STAGE_COMPILE_IMPORT, STAGE_OPENING, STAGE_PERSONALITY, STAGE_REVIEW, STAGE_WORLDVIEW,
};
use storyforge_domain::character::{CharacterCard, CharacterDefinition, CharacterExtractionStatus};
use storyforge_domain::llm::{ChatMessage, ChatRequest, SamplingParams};
use crate::card_studio_store::CardStudioStore;
use crate::error::TauriCommandError;
use crate::{AppState, CharacterInfo, CharacterSummary};

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
        .map_err(|e| TauriCommandError::storage(e))
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
        .map_err(|e| TauriCommandError::storage(e))
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
    store
        .update(project)
        .map_err(|e| TauriCommandError::storage(e))
}

#[tauri::command]
pub fn cardstudio_set_stage(id: String, stage_id: String) -> Result<CardProject, TauriCommandError> {
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
    store
        .update(project)
        .map_err(|e| TauriCommandError::storage(e))
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
    store
        .update(project)
        .map_err(|e| TauriCommandError::storage(e))
}

#[tauri::command]
pub fn cardstudio_run_checks(id: String) -> Result<CheckReport, TauriCommandError> {
    let project = get_card_studio_store()
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;
    Ok(run_checks(&project.artifacts))
}

#[tauri::command]
pub fn cardstudio_compile(id: String) -> Result<CompilePreviewDto, TauriCommandError> {
    let project = get_card_studio_store()
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;
    let compiled = compile_artifacts(&project.artifacts)
        .map_err(TauriCommandError::validation)?;
    Ok(CompilePreviewDto {
        st_card_json: compiled.st_card_json,
        warnings: compiled.warnings,
        character_name: compiled.character.name,
    })
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
                        .filter(|i| {
                            matches!(
                                i.severity,
                                card_studio::CheckSeverity::Error
                            )
                        })
                        .map(|i| i.message.clone())
                        .collect::<Vec<_>>()
                        .join("; "),
                );
                let _ = store.update(project.clone());
                return Err(TauriCommandError::validation(
                    project.last_error.clone().unwrap_or_else(|| "检查未通过".into()),
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
    store
        .update(project)
        .map_err(|e| TauriCommandError::storage(e))
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

    let llm = state.active_llm_or_mock();
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
    store
        .update(project)
        .map_err(|e| TauriCommandError::storage(e))
}

#[tauri::command]
pub fn cardstudio_import_compiled(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<ImportCompiledResultDto, TauriCommandError> {
    let store = get_card_studio_store();
    let mut project = store
        .get(&id)
        .ok_or_else(|| TauriCommandError::not_found(format!("写卡项目不存在: {id}")))?;

    let compiled =
        compile_artifacts(&project.artifacts).map_err(TauriCommandError::validation)?;
    let character = compiled.character;
    let warnings = compiled.warnings;

    // Persist into CharacterStore (same path shape as import_character).
    let info = CharacterInfo::from(&character);
    let stored = crate::get_store()
        .save(info)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?;

    // Sync tool_ctx so extract_characters / campaign can see the card.
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.characters.retain(|c| c.name != character.name);
        let world_info = character.embedded_world_info.clone();
        ctx.characters.push(Arc::new(character.clone()));
        if let Some(wi) = world_info {
            ctx.world_info = Some(Arc::new(wi));
        }
    }

    // Seed a fallback CharacterCard so list_cards can open campaigns without forced LLM extract.
    let mut card = CharacterCard::from_character(&character);
    let def = CharacterDefinition::fallback_from_character(&character, &[]);
    let definitions =
        storyforge_app_agent::attach_definitions_to_card(vec![def], &card.id);
    card.character_definitions = definitions;
    card.extraction_status = CharacterExtractionStatus::Fallback;
    card.extraction_message = Some("Card Studio 从零导入，已生成单主角定义，可稍后重新识别。".into());
    let stored_card = crate::get_campaign_store()
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
