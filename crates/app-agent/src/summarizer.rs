//! 剧情总结 Agent 编排（对应 AGENT_INTERFACES §6.3）
//!
//! 编剧成文后并行跑。每轮产出一条本轮摘要（200-500 字），存进 round_summaries.json。
//! 独立于 archiver 的批量归档（archiver 把多条摘要压成远记忆）。

use tokio::sync::watch;
use tracing::info;

use storyforge_domain::agent_profile_config::AgentProfileConfig;
use storyforge_domain::llm::ChatResponse;

use crate::prompts::{build_summarizer_user_msg, make_summarizer_config};
use crate::runtime::AgentRuntime;
use crate::tools::ToolRegistry;
use crate::{AgentConfig, AgentError};

#[derive(Debug, thiserror::Error)]
pub enum SummarizerError {
    #[error("LLM 调用失败: {0}")]
    Agent(#[from] AgentError),
}

/// 跑剧情总结 Agent，返回摘要文本
///
/// `agent_profile_config`（可选）用于覆盖 Summarizer 的 model/rounds。
/// 传 None = 当前硬编码默认值，向后兼容。Summarizer 无工具，tool_whitelist 不适用。
pub async fn run_summarizer(
    runtime: &AgentRuntime,
    final_text: &str,
    scene_brief: &str,
    turn: u32,
    cancel: watch::Receiver<bool>,
    agent_profile_config: Option<&AgentProfileConfig>,
) -> Result<String, SummarizerError> {
    let config: AgentConfig = make_summarizer_config(agent_profile_config);
    let user_msg = build_summarizer_user_msg(final_text, scene_brief, turn);
    let registry = ToolRegistry::new(); // 总结 Agent 无工具，纯输出文本

    info!(target: "summarizer", "开始本轮剧情总结（第 {turn} 轮）");

    let resp: ChatResponse = runtime
        .run_tool_loop(&config, user_msg, &registry, cancel)
        .await?;

    let summary = resp.content.trim().to_string();
    info!(
        target: "summarizer",
        "本轮摘要完成：{} 字",
        summary.chars().count()
    );
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_run_summarizer_with_mock_llm() {
        use crate::AgentRuntime;
        use crate::tools::ToolContext;
        use storyforge_infra_llm::LlmClient;
        use storyforge_infra_llm::mock_client::MockLlmClient;

        let client =
            std::sync::Arc::new(MockLlmClient::with_defaults()) as std::sync::Arc<dyn LlmClient>;
        let ctx = std::sync::Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let runtime = AgentRuntime::new(client, ctx);
        let (_tx, rx) = tokio::sync::watch::channel(false);

        let summary = run_summarizer(&runtime, "测试成文", "测试场景", 1, rx, None)
            .await
            .unwrap();
        assert!(!summary.is_empty());
    }
}
