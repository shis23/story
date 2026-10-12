use super::super::*;

// ─── Meta Agent 命令 ──────────────────────────────────────────────────────

/// Meta 对话流式事件（Tauri Channel 用）
///
/// 复用 WritingEvent 的扁平模式。目前只有 token 增量一种事件；
/// 命令的最终聚合结果（agent_message/messages/new_patch）仍由命令返回值携带。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaStreamEvent {
    pub event_type: String,
    pub data: serde_json::Value,
}

impl MetaStreamEvent {
    /// token 增量事件
    pub fn progress(delta: String) -> Self {
        Self {
            event_type: "meta_progress".into(),
            data: serde_json::json!({ "delta": delta }),
        }
    }
}

// ─── P3：Meta Agent 多轮对话 / MVU 五合一分析 / ST 预设 LLM 分类 ────────────

/// GenerationExplainer 的 tauri-app 实现：从 ConversationStore 查 Provenance
/// 并调 `explain_generation`，让 Meta 多轮对话的 inspect_generation 工具
/// 能引用真实生成溯源（而非 app-meta 层的 None 占位）。
///
/// 实现 app-meta 的 GenerationExplainer trait，注入到 MetaSession。
/// 持有 Arc<ConversationStore>（与 AppState.conv_store 共享同一份）。
/// 查不到对话/节点/变体/溯源时返回 None，由工具层转成错误信息，不 panic。
pub struct ConvGenerationExplainer {
    pub conv_store: Arc<storyforge_app_conversation::ConversationStore>,
}

impl storyforge_app_meta::meta_conversation::GenerationExplainer for ConvGenerationExplainer {
    fn explain(
        &self,
        conversation_id: String,
        node_id: String,
    ) -> storyforge_app_meta::meta_conversation::GenerationExplainFuture {
        let conv_store = self.conv_store.clone();
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || {
                explain_generation_from_conversation_store(conv_store, conversation_id, node_id)
                    .map(storyforge_app_meta::GenerationExplanation::without_reasoning)
            })
            .await
            {
                Ok(explanation) => explanation,
                Err(e) => {
                    tracing::warn!("解释生成溯源任务失败: {e}");
                    None
                }
            }
        })
    }
}

pub fn explain_generation_from_conversation_store(
    conv_store: Arc<ConversationStore>,
    conversation_id: String,
    node_id: String,
) -> Option<storyforge_app_meta::GenerationExplanation> {
    let conv_id = Id::from_str(&conversation_id);
    let nid = Id::from_str(&node_id);
    let conv = conv_store.get(&conv_id)?;
    let node = conv.find_node(&nid)?;
    let variant = node.active()?;
    let provenance = variant.provenance.as_ref()?;
    Some(storyforge_app_meta::explain_generation(provenance))
}
