use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use storyforge_domain::Id;
use storyforge_domain::conversation::{
    Conversation, MessageNode, MessageVariant, Provenance, Role as ConversationRole, VariantStatus,
};
use storyforge_domain::preset::{RegexPlacement, RegexScript};
use storyforge_infra_regex::{
    RegexExecutionTarget, apply_reasoning_regex_to_think_blocks_at_depth,
    apply_regex_scripts_for_target_at_depth,
};

use crate::AppState;
use crate::error::TauriCommandError;
use crate::storage_backend::BackendCapability;
use crate::{
    collect_scoped_regex_scripts, get_conn_store, get_global_regex_store, get_preset_store,
    merge_runtime_regex_scripts,
};
use storyforge_app_conversation::ConversationStore;

#[tauri::command]
pub(crate) async fn archive_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    archive_conversation_impl(conversation_id, state).await
}

pub(crate) async fn archive_conversation_impl(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);
    let config = state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .ok_or("未配置嵌入 API，请先在设置中配置")?;
    let llm = state.require_active_llm()?;
    let vector_store = state.vector_store.clone();
    let embedder = Arc::new(
        storyforge_infra_llm::Embedder::new(config)
            .map_err(|e| TauriCommandError::internal(e.to_string()))?,
    );
    let model = get_conn_store()
        .active_connection()
        .map(|c| c.model)
        .unwrap_or_else(|| "deepseek-chat".into());
    let archiver = storyforge_app_memory::MemoryArchiver::new(
        llm,
        embedder,
        vector_store,
        storyforge_app_memory::ArchiveConfig::default(),
        model,
    );

    let count = run_archive_with_watermark(&state, &conv_id, &archiver, true)
        .await
        .map_err(TauriCommandError::internal)?;
    Ok(count)
}

fn archivable_messages_from_conversation(conv: &Conversation) -> Vec<String> {
    conv.nodes
        .iter()
        .filter_map(|node| {
            let v = node.active()?;
            if v.status == storyforge_domain::conversation::VariantStatus::Discarded {
                None
            } else {
                Some(v.content.clone())
            }
        })
        .collect()
}

/// 归档快照：消息列表 + 水位 + campaign 标签。
pub(crate) struct ArchiveSnapshot {
    pub(crate) messages: Vec<String>,
    pub(crate) archived_upto: usize,
    pub(crate) campaign_id: Option<String>,
    pub(crate) conversation_id: String,
}

pub(crate) async fn load_archive_snapshot(
    conv_store: Arc<ConversationStore>,
    conv_id: Id,
) -> Result<ArchiveSnapshot, TauriCommandError> {
    tokio::task::spawn_blocking(move || {
        let conv = conv_store.get(&conv_id).ok_or_else(|| {
            TauriCommandError::from(storyforge_app_conversation::ConversationError::NotFound(
                conv_id.to_string(),
            ))
        })?;
        Ok::<ArchiveSnapshot, TauriCommandError>(ArchiveSnapshot {
            messages: archivable_messages_from_conversation(&conv),
            archived_upto: conv.archived_upto,
            campaign_id: conv.campaign_id.map(|id| id.to_string()),
            conversation_id: conv.id.to_string(),
        })
    })
    .await
    .map_err(|e| TauriCommandError::internal(format!("读取归档消息任务失败: {e}")))?
}

/// 水位驱动归档：只处理 `archived_upto..` 前缀，成功后推进水位。
///
/// `require_threshold`：true 时与 ArchiveConfig.threshold 对齐（自动归档）；
/// 手动命令也传 true，避免短对话误触发 LLM。
async fn run_archive_with_watermark(
    state: &Arc<AppState>,
    conv_id: &Id,
    archiver: &storyforge_app_memory::MemoryArchiver,
    require_threshold: bool,
) -> Result<usize, String> {
    let snap = load_archive_snapshot(state.conv_store.clone(), conv_id.clone())
        .await
        .map_err(|e| e.to_string())?;

    let total = snap.messages.len();
    let upto = snap.archived_upto.min(total);
    if upto >= total {
        return Ok(0);
    }
    let pending = &snap.messages[upto..];
    if require_threshold
        && pending.len() < storyforge_app_memory::ArchiveConfig::default().threshold
    {
        return Ok(0);
    }
    if pending.is_empty() {
        return Ok(0);
    }

    let meta = storyforge_app_memory::ArchiveMeta {
        campaign_id: snap.campaign_id,
        conversation_id: Some(snap.conversation_id),
    };

    // 水位路径已确认 pending 需要归档：跳过 maybe_archive 的二次 threshold，
    // 直接 archive_prefix；source_range 相对 pending 切片。
    let summaries = archiver
        .archive_prefix(pending, Some(&meta))
        .await
        .map_err(|e| e.to_string())?;

    if summaries.is_empty() {
        return Ok(0);
    }

    // 取最大 end_idx + 1 作为本轮推进量（相对 pending）
    let advanced = summaries
        .iter()
        .map(|s| s.source_range.1.saturating_add(1))
        .max()
        .unwrap_or(0);
    if advanced == 0 {
        return Ok(summaries.len());
    }
    let new_upto = upto.saturating_add(advanced).min(total);
    let conv_store = state.conv_store.clone();
    let conv_id_clone = conv_id.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        conv_store.advance_archived_upto(&conv_id_clone, new_upto)
    })
    .await
    .map_err(|e| format!("推进归档水位任务失败: {e}"))?
    {
        tracing::warn!("推进归档水位失败: {e}");
    } else {
        tracing::info!(
            target: "far_memory",
            "归档水位 {} → {}（+{} 条消息，{} 条总结）",
            upto,
            new_upto,
            advanced,
            summaries.len()
        );
    }
    Ok(summaries.len())
}

// ─── 自动归档辅助 ──────────────────────────────────────────────────────────

/// 检查对话未归档消息是否超过阈值，超过则在后台触发归档。
///
/// 阈值：50 条未归档消息（与 ArchiveConfig.default().threshold 一致）。
/// 归档失败只 warn，不影响用户操作。
pub(crate) async fn auto_archive_if_needed(state: &Arc<AppState>, conv_id: &Id) {
    let config = match state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
    {
        Some(c) => c,
        None => {
            tracing::debug!("未配置嵌入 API，跳过自动归档");
            return;
        }
    };

    // 快速水位检查：未归档不足阈值则跳过（避免无意义构造 archiver）
    match load_archive_snapshot(state.conv_store.clone(), conv_id.clone()).await {
        Ok(snap) => {
            let pending = snap.messages.len().saturating_sub(snap.archived_upto);
            if pending < storyforge_app_memory::ArchiveConfig::default().threshold {
                return;
            }
            tracing::info!(
                "自动归档触发：对话 {} 未归档 {} 条 >= 阈值 {}",
                conv_id,
                pending,
                storyforge_app_memory::ArchiveConfig::default().threshold
            );
        }
        Err(e) => {
            tracing::debug!("读取自动归档消息失败，跳过: {e}");
            return;
        }
    }

    let llm = match state.require_active_llm() {
        Ok(llm) => llm,
        Err(error) => {
            tracing::debug!("自动归档跳过：{error}");
            return;
        }
    };
    let vector_store = state.vector_store.clone();
    let embedder = match storyforge_infra_llm::Embedder::new(config) {
        Ok(e) => Arc::new(e),
        Err(e) => {
            tracing::warn!("构建 Embedder 失败，跳过自动归档: {e}");
            return;
        }
    };
    let model = get_conn_store()
        .active_connection()
        .map(|c| c.model)
        .unwrap_or_else(|| "deepseek-chat".into());
    let archiver = storyforge_app_memory::MemoryArchiver::new(
        llm,
        embedder,
        vector_store,
        storyforge_app_memory::ArchiveConfig::default(),
        model,
    );

    match run_archive_with_watermark(state, conv_id, &archiver, true).await {
        Ok(n) if n > 0 => {
            tracing::info!("自动归档完成：{} 条总结", n);
        }
        Ok(_) => {
            tracing::debug!("自动归档：无需归档");
        }
        Err(e) => {
            tracing::warn!("自动归档失败（不影响用户操作）: {e}");
        }
    }
}

/// 活动 Turn 的质量门禁摘要（供前端刷新后回填 ProcessReview）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTurnQualityDto {
    pub turn_id: String,
    pub attempt_id: String,
    pub status: String,
    pub passed: bool,
    pub warning_count: usize,
    pub error_count: usize,
    pub warnings: Vec<String>,
}

/// Accept 前展示给用户的单条候选状态变化。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnReceiptItemDto {
    /// 对应 Prepared MutationBatch 中的稳定下标，确认时原样回传。
    pub mutation_index: usize,
    /// chronicle / knowledge / variable / task
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub selected_by_default: bool,
}

/// Campaign Turn 的 Accept-before 小票。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTurnReceiptDto {
    pub turn_id: String,
    pub attempt_id: String,
    pub variant_id: String,
    pub status: String,
    pub ready: bool,
    pub derivation_failed: bool,
    pub can_retry: bool,
    pub can_degraded_accept: bool,
    pub notice: Option<String>,
    pub items: Vec<TurnReceiptItemDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ConversationSummaryDto {
    id: String,
    character_id: Option<String>,
    campaign_id: Option<String>,
    /// 关联角色卡名（前端列表显示用）
    card_name: Option<String>,
    message_count: usize,
    created_at: String,
    updated_at: String,
}

#[tauri::command]
pub(crate) fn list_conversations(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<ConversationSummaryDto>, TauriCommandError> {
    // 联查角色卡名由 backend adapter 统一处理（Gate 3）：
    // JSON 一次 list_cards 快照建卡名表（character_id 直查 → campaign 兜底）；
    // SQLite 无卡名来源，card_name 恒 None（文档化 Gate 4 缺口，不静默回退）。
    let summaries = state.conv_store.list();
    let card_names =
        crate::backend_workflows::conversation_card_names_for_backend(state.storage(), &summaries)
            .map_err(TauriCommandError::internal)?;
    Ok(summaries
        .into_iter()
        .map(|c| ConversationSummaryDto {
            id: c.id.to_string(),
            character_id: c.character_id,
            campaign_id: c.campaign_id.map(|id| id.to_string()),
            card_name: card_names.get(c.id.as_str()).cloned(),
            message_count: c.message_count,
            created_at: c.created_at.to_rfc3339(),
            updated_at: c.updated_at.to_rfc3339(),
        })
        .collect())
}

/// 删除整个会话。
///
/// 一 Campaign 一对话：若该会话绑定了 Campaign（或某 Campaign 的 conversation_id 指向它），
/// 则按 **整局活动** 级联删除（实例/知识/任务/总结 + 会话），而不是只清消息树。
#[tauri::command]
pub(crate) fn delete_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let conv_id = Id::from_str(&conversation_id);

    // 优先：会话自己记录的 campaign_id
    let campaign_id = state
        .conv_store
        .get(&conv_id)
        .and_then(|c| c.campaign_id.clone())
        // 兜底：Campaign.conversation_id 反向指向（悬空/半绑定时）
        .or_else(|| {
            state
                .storage()
                .list_campaigns(None)
                .ok()
                .and_then(|records| {
                    records
                        .into_iter()
                        .find(|record| record.campaign.conversation_id.as_ref() == Some(&conv_id))
                        .map(|record| record.campaign.id)
                })
        });

    if let Some(campaign_id) = campaign_id {
        return crate::playthrough_lifecycle::delete_campaign_playthrough_in_store(
            state.storage().as_ref(),
            state.conv_store.as_ref(),
            state.inner().as_ref(),
            &campaign_id,
        );
    }

    // 无 Campaign 的遗留/孤儿会话：只删对话
    state
        .conv_store
        .delete(&conv_id)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

#[tauri::command]
pub(crate) fn get_conversation(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    let conv_id = storyforge_domain::Id::from_str(&id);
    let conversation = state
        .conv_store
        .get(&conv_id)
        .ok_or_else(|| TauriCommandError::not_found(format!("对话不存在: {id}")))?;
    let regex_scripts = collect_conversation_regex_scripts(&conversation, state.inner().as_ref());
    Ok(
        serde_json::to_value(conversation_display_dto(&conversation, &regex_scripts))
            .unwrap_or_default(),
    )
}

#[derive(Debug, Clone, Serialize)]
struct ConversationDisplayDto {
    id: Id,
    character_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    campaign_id: Option<Id>,
    nodes: Vec<MessageNodeDisplayDto>,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
struct MessageNodeDisplayDto {
    id: Id,
    parent_id: Option<Id>,
    variants: Vec<MessageVariantDisplayDto>,
    active_variant: usize,
}

#[derive(Debug, Clone, Serialize)]
struct MessageVariantDisplayDto {
    id: Id,
    role: ConversationRole,
    content: String,
    display_content: String,
    created_at: chrono::DateTime<Utc>,
    status: VariantStatus,
    provenance: Option<Provenance>,
}

fn collect_conversation_regex_scripts(
    conversation: &Conversation,
    state: &AppState,
) -> Vec<RegexScript> {
    // campaign-scoped 正则按 backend 分发（Gate 3）：JSON 从 CampaignStore 读；
    // SQLite 无 campaign-scoped 来源（Ok(None)），保持 character 维度降级。
    let scoped_scripts = match conversation.campaign_id.as_ref() {
        Some(campaign_id) => {
            match crate::backend_workflows::campaign_scoped_regex_scripts_for_backend(
                state.storage(),
                campaign_id,
            ) {
                Ok(Some(scripts)) => scripts,
                Ok(None) => collect_character_scoped_regex_scripts(conversation, state),
                Err(error) => {
                    tracing::warn!("conversation regex context unavailable: {error}");
                    Vec::new()
                }
            }
        }
        None => collect_character_scoped_regex_scripts(conversation, state),
    };

    merge_runtime_regex_scripts(scoped_scripts, get_preset_store(), get_global_regex_store())
}

fn collect_character_scoped_regex_scripts(
    conversation: &Conversation,
    state: &AppState,
) -> Vec<RegexScript> {
    let tool_snapshot = state.snapshot_tool_ctx();
    collect_scoped_regex_scripts(
        conversation.character_id.as_deref(),
        &tool_snapshot.characters,
        state
            .storage()
            .json_character_store(
                BackendCapability::CharacterCommands,
                "conversation scoped regex context",
            )
            .ok(),
    )
}

fn conversation_display_dto(
    conversation: &Conversation,
    regex_scripts: &[RegexScript],
) -> ConversationDisplayDto {
    let display_scripts = display_only_regex_scripts(regex_scripts);
    ConversationDisplayDto {
        id: conversation.id.clone(),
        character_id: conversation.character_id.clone(),
        campaign_id: conversation.campaign_id.clone(),
        nodes: conversation
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let depth = conversation.nodes.len().saturating_sub(index + 1);
                message_node_display_dto(node, &display_scripts, depth)
            })
            .collect(),
        created_at: conversation.created_at,
        updated_at: conversation.updated_at,
    }
}

fn message_node_display_dto(
    node: &MessageNode,
    display_scripts: &[RegexScript],
    depth: usize,
) -> MessageNodeDisplayDto {
    MessageNodeDisplayDto {
        id: node.id.clone(),
        parent_id: node.parent_id.clone(),
        variants: node
            .variants
            .iter()
            .map(|variant| message_variant_display_dto(variant, display_scripts, depth))
            .collect(),
        active_variant: node.active_variant,
    }
}

fn message_variant_display_dto(
    variant: &MessageVariant,
    display_scripts: &[RegexScript],
    depth: usize,
) -> MessageVariantDisplayDto {
    MessageVariantDisplayDto {
        id: variant.id.clone(),
        role: variant.role.clone(),
        content: variant.content.clone(),
        display_content: render_variant_display_content(variant, display_scripts, depth),
        created_at: variant.created_at,
        status: variant.status.clone(),
        // 普通会话读取只返回重 roll 所需的非敏感溯源。reasoning 原文仅由
        // meta_explain_generation 显式审计命令按需返回，避免页面加载即下发。
        provenance: variant.provenance.clone().map(|mut provenance| {
            provenance.director_reasoning = None;
            provenance.writer_reasoning = None;
            provenance.editor_reasoning = None;
            for subagent in &mut provenance.subagent_results {
                subagent.reasoning_content = None;
            }
            provenance
        }),
    }
}

fn display_only_regex_scripts(regex_scripts: &[RegexScript]) -> Vec<RegexScript> {
    regex_scripts
        .iter()
        .filter(|script| script.markdown_only.unwrap_or(false))
        .cloned()
        .collect()
}

fn render_variant_display_content(
    variant: &MessageVariant,
    display_scripts: &[RegexScript],
    depth: usize,
) -> String {
    if variant.role != ConversationRole::Assistant || display_scripts.is_empty() {
        return variant.content.clone();
    }

    let reasoning_applied = apply_reasoning_regex_to_think_blocks_at_depth(
        &variant.content,
        display_scripts,
        RegexExecutionTarget::Display,
        depth,
    )
    .and_then(|text| {
        apply_regex_scripts_for_target_at_depth(
            &text,
            display_scripts,
            RegexPlacement::Output,
            RegexExecutionTarget::Display,
            depth,
        )
    });

    reasoning_applied.unwrap_or_else(|e| {
        tracing::warn!("展示正则执行失败，使用原始消息内容: {e}");
        variant.content.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::preset::RegexScriptSource;
    use storyforge_domain::preset::{ST_REGEX_PLACEMENT_AI_OUTPUT, ST_REGEX_PLACEMENT_REASONING};

    fn regex_script(id: &str, source: RegexScriptSource) -> RegexScript {
        RegexScript {
            id: id.to_string(),
            script_name: id.to_string(),
            find_regex: id.to_string(),
            replace_string: String::new(),
            placement: RegexPlacement::Output,
            placement_codes: vec![ST_REGEX_PLACEMENT_AI_OUTPUT],
            source,
            disabled: false,
            flags: String::new(),
            only_format_formatting: None,
            markdown_only: None,
            prompt_only: None,
            run_on_edit: None,
            substitute_regex: None,
            trim_strings: vec![],
            min_depth: None,
            max_depth: None,
        }
    }

    #[test]
    fn display_dto_applies_markdown_only_output_without_mutating_content() {
        let mut conversation = Conversation::new(Some("source-lin".into()), None);
        conversation.append_message(
            storyforge_domain::conversation::Role::User,
            "<data_block>user</data_block>".into(),
        );
        conversation.append_ai_draft("<data_block>hp=5</data_block> scene".into(), None);

        let mut script = regex_script("display-hp", RegexScriptSource::Preset);
        script.find_regex = r"<data_block>hp=5</data_block>".into();
        script.replace_string = "[HP:5]".into();
        script.markdown_only = Some(true);

        let dto = conversation_display_dto(&conversation, &[script]);

        let user_variant = &dto.nodes[0].variants[0];
        assert_eq!(user_variant.content, "<data_block>user</data_block>");
        assert_eq!(
            user_variant.display_content,
            "<data_block>user</data_block>"
        );

        let assistant_variant = &dto.nodes[1].variants[0];
        assert_eq!(
            assistant_variant.content,
            "<data_block>hp=5</data_block> scene"
        );
        assert_eq!(assistant_variant.display_content, "[HP:5] scene");
        assert_eq!(
            conversation.nodes[1].variants[0].content,
            "<data_block>hp=5</data_block> scene"
        );
    }

    #[test]
    fn display_dto_applies_markdown_only_reasoning_without_mutating_content() {
        let mut conversation = Conversation::new(Some("source-lin".into()), None);
        conversation.append_ai_draft("<think>raw chain</think> final raw".into(), None);

        let mut script = regex_script("display-reasoning", RegexScriptSource::Preset);
        script.find_regex = r"raw".into();
        script.replace_string = "pretty".into();
        script.placement = RegexPlacement::Reasoning;
        script.placement_codes = vec![ST_REGEX_PLACEMENT_REASONING];
        script.markdown_only = Some(true);
        script.flags = "g".into();

        let dto = conversation_display_dto(&conversation, &[script]);

        let assistant_variant = &dto.nodes[0].variants[0];
        assert_eq!(
            assistant_variant.content,
            "<think>raw chain</think> final raw"
        );
        assert_eq!(
            assistant_variant.display_content,
            "<think>pretty chain</think> final raw"
        );
        assert_eq!(
            conversation.nodes[0].variants[0].content,
            "<think>raw chain</think> final raw"
        );
    }

    #[test]
    fn ordinary_display_dto_strips_captured_reasoning() {
        let mut conversation = Conversation::new(Some("source-lin".into()), None);
        conversation.append_ai_draft(
            "visible".into(),
            Some(Provenance {
                session_id: Id::from_str("session-audit"),
                plan: None,
                subagent_results: vec![storyforge_domain::conversation::SubagentSnapshot {
                    character_id: "lin".into(),
                    full_text: "performance".into(),
                    character_instance_id: None,
                    display_name: None,
                    fallback_reason: None,
                    reasoning_content: Some("subagent secret reasoning".into()),
                }],
                profile_id: None,
                generation_mode: None,
                seed: 1,
                last_hint: None,
                director_reasoning: Some("director secret reasoning".into()),
                writer_reasoning: Some("writer secret reasoning".into()),
                editor_reasoning: Some("editor secret reasoning".into()),
            }),
        );

        let dto = conversation_display_dto(&conversation, &[]);
        let displayed = dto.nodes[0].variants[0]
            .provenance
            .as_ref()
            .expect("non-reasoning provenance remains available");
        assert!(displayed.director_reasoning.is_none());
        assert!(displayed.writer_reasoning.is_none());
        assert!(displayed.editor_reasoning.is_none());
        assert!(displayed.subagent_results[0].reasoning_content.is_none());

        let stored = conversation.nodes[0].variants[0]
            .provenance
            .as_ref()
            .expect("stored provenance remains intact for explicit audit");
        assert_eq!(
            stored.editor_reasoning.as_deref(),
            Some("editor secret reasoning")
        );
    }

    #[test]
    fn display_dto_does_not_reapply_persisted_output_regex() {
        let mut conversation = Conversation::new(Some("source-lin".into()), None);
        conversation.append_ai_draft("persisted bar".into(), None);

        let mut script = regex_script("persisted-output", RegexScriptSource::Preset);
        script.find_regex = "bar".into();
        script.replace_string = "baz".into();

        let dto = conversation_display_dto(&conversation, &[script]);

        let variant = &dto.nodes[0].variants[0];
        assert_eq!(variant.content, "persisted bar");
        assert_eq!(variant.display_content, "persisted bar");
    }

    #[test]
    fn display_dto_respects_display_regex_depth() {
        let mut conversation = Conversation::new(Some("source-lin".into()), None);
        conversation.append_ai_draft("<status>old</status>".into(), None);
        conversation.append_message(
            storyforge_domain::conversation::Role::User,
            "continue".into(),
        );
        conversation.append_ai_draft("<status>new</status>".into(), None);

        let mut script = regex_script("recent-status", RegexScriptSource::Preset);
        script.find_regex = r"<status>(.*?)</status>".into();
        script.replace_string = "[$1]".into();
        script.markdown_only = Some(true);
        script.min_depth = Some(0);
        script.max_depth = Some(1);

        let dto = conversation_display_dto(&conversation, &[script]);

        let old_variant = &dto.nodes[0].variants[0];
        let new_variant = &dto.nodes[2].variants[0];
        assert_eq!(old_variant.display_content, "<status>old</status>");
        assert_eq!(new_variant.display_content, "[new]");
    }
}
