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
use crate::sqlite_runtime;
use crate::{
    collect_campaign_scoped_regex_scripts, collect_scoped_regex_scripts, get_campaign_store,
    get_global_regex_store, get_preset_store, merge_runtime_regex_scripts,
};

#[tauri::command]
pub(crate) async fn archive_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<usize, TauriCommandError> {
    crate::archive_conversation_impl(conversation_id, state).await
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
) -> Vec<ConversationSummaryDto> {
    // 联查角色卡名。
    // conversation.character_id 存的是 CharacterCard.id（而非 domain Character 的
    // source_character_id），所以必须用 campaign_store 的卡片表按 card.id 联查,
    // 不能用 tool_ctx.characters（那是扁平 Character,id=source_character_id）。
    // 兜底:character_id 联查不到时,走 campaign_id → campaign.card_id → card.name。
    let store = get_campaign_store();
    let cards = store.list_cards();
    let card_by_id: std::collections::HashMap<&Id, &str> = cards
        .iter()
        .map(|sc| (&sc.card.id, sc.card.name.as_str()))
        .collect();
    state
        .conv_store
        .list()
        .into_iter()
        .map(|c| {
            let card_name = c
                .character_id
                .as_ref()
                .and_then(|cid| {
                    // 首选:直接按 character_id(=CharacterCard.id)查卡名
                    let cid_id = Id::from_str(cid);
                    card_by_id.get(&cid_id).map(|n| (*n).to_string())
                })
                .or_else(|| {
                    // 兜底:campaign_id → campaign.card_id → card.name
                    c.campaign_id.as_ref().and_then(|camp_id| {
                        store.get_campaign(camp_id).and_then(|campaign| {
                            card_by_id.get(&campaign.card_id).map(|n| (*n).to_string())
                        })
                    })
                });
            ConversationSummaryDto {
                id: c.id.to_string(),
                character_id: c.character_id,
                campaign_id: c.campaign_id.map(|id| id.to_string()),
                card_name,
                message_count: c.message_count,
                created_at: c.created_at.to_rfc3339(),
                updated_at: c.updated_at.to_rfc3339(),
            }
        })
        .collect()
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
    if sqlite_runtime::is_sqlite_active() {
        return Err(TauriCommandError::validation(
            "conversation/campaign deletion is not available in the SQLite opt-in backend yet"
                .to_string(),
        ));
    }
    let conv_id = Id::from_str(&conversation_id);
    let store = get_campaign_store();

    // 优先：会话自己记录的 campaign_id
    let campaign_id = state
        .conv_store
        .get(&conv_id)
        .and_then(|c| c.campaign_id.clone())
        // 兜底：Campaign.conversation_id 反向指向（悬空/半绑定时）
        .or_else(|| {
            store
                .list_campaigns()
                .into_iter()
                .find(|c| c.conversation_id.as_ref() == Some(&conv_id))
                .map(|c| c.id)
        });

    if let Some(campaign_id) = campaign_id {
        return crate::playthrough_lifecycle::delete_campaign_playthrough_in_store(
            store,
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
    let scoped_scripts = if let Some(campaign_id) = &conversation.campaign_id {
        collect_campaign_scoped_regex_scripts(campaign_id, get_campaign_store())
    } else {
        let tool_snapshot = state.snapshot_tool_ctx();
        collect_scoped_regex_scripts(
            conversation.character_id.as_deref(),
            &tool_snapshot.characters,
        )
    };

    merge_runtime_regex_scripts(scoped_scripts, get_preset_store(), get_global_regex_store())
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
