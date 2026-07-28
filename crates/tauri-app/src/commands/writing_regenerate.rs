use serde::{Deserialize, Serialize};
use storyforge_app_conversation::PartialRollTarget;
use storyforge_domain::Id;
use storyforge_domain::conversation::Conversation;

use crate::error::TauriCommandError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RegenerateTargetDto {
    /// "director" / "editor" / "subagent:<角色名>"
    pub kind: String,
}

/// 重 roll 请求 DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RegenerateRequestDto {
    pub conversation_id: String,
    pub node_id: String,
    /// 目标列表（空 = 整体重 roll）
    pub targets: Vec<RegenerateTargetDto>,
    /// 当前产品写作模式；缺省时走旧版大场面兼容路径。
    pub generation_mode: Option<storyforge_domain::generation::GenerationMode>,
    /// 附加提示词（可选）
    pub hint: Option<String>,
    /// 随机种子（可选）
    pub seed: Option<u64>,
}

/// 把 DTO 的 target 字符串解析为 PartialRollTarget
pub(crate) fn parse_target_dto(
    target: &RegenerateTargetDto,
) -> Result<PartialRollTarget, TauriCommandError> {
    match target.kind.as_str() {
        "director" => Ok(PartialRollTarget::Director),
        "editor" => Ok(PartialRollTarget::Editor),
        s if s.starts_with("subagent:") => {
            let id = s.trim_start_matches("subagent:");
            if id.is_empty() {
                Err("subagent 目标缺少角色名".into())
            } else {
                Ok(PartialRollTarget::Subagent(id.to_string()))
            }
        }
        other => Err(TauriCommandError::validation(format!(
            "未知重 roll 目标: {other}"
        ))),
    }
}

/// Refuse a regenerate request unless the target conversation and the active
/// Turn belong to the same selected Campaign. This check must happen before
/// `PipelineOrchestrator::regenerate`, because that pipeline mutates the
/// requested conversation's draft in place.
pub(crate) fn validate_regenerate_campaign_scope(
    active_campaign: Option<&Id>,
    conversation: &Conversation,
    active_turn: Option<&storyforge_domain::turn::TurnRecord>,
) -> Result<(), TauriCommandError> {
    match (active_campaign, conversation.campaign_id.as_ref()) {
        (None, None) => Ok(()),
        (None, Some(conversation_campaign)) => Err(TauriCommandError::validation(format!(
            "campaign conversation {} requires selecting campaign {} before regenerate",
            conversation.id, conversation_campaign
        ))),
        (Some(active), Some(conversation_campaign)) if active == conversation_campaign => {
            let turn = active_turn.ok_or_else(|| {
                TauriCommandError::validation(format!(
                    "campaign {} has no active turn for regenerate",
                    active
                ))
            })?;
            if turn.campaign_id != *active || turn.conversation_id != conversation.id {
                return Err(TauriCommandError::validation(format!(
                    "regenerate scope mismatch: conversation {} is not the active turn conversation",
                    conversation.id
                )));
            }
            Ok(())
        }
        (Some(active), Some(conversation_campaign)) => Err(TauriCommandError::validation(format!(
            "regenerate campaign mismatch: selected {active}, conversation belongs to {conversation_campaign}"
        ))),
        (Some(active), None) => Err(TauriCommandError::validation(format!(
            "legacy conversation {} cannot regenerate while campaign {} is active",
            conversation.id, active
        ))),
    }
}
