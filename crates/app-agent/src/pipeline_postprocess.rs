//! 后处理并行编排（阶段 4，对应 AGENT_INTERFACES §6.3/§6.4，D40-D41/D45）
//!
//! 编剧成文（DraftReady）后并行跑两个 Agent：
//! - **剧情总结 Agent**：产出本轮摘要（200-500 字），存 round_summaries.json
//! - **后处理 Agent**：三合一产出角色知识 + 变量更新 + 任务更新
//!
//! 两者并行不串行，任一失败不影响另一个（best-effort）。照搬 spawn_subagents 的
//! tokio::spawn + watch::Receiver::clone() 模式：主流水线取消时两个 Agent 立即响应。
//!
//! 不在本模块做的事：写 CampaignStore（由调用方 app-pipeline 在拿到结果后落盘）。

use tokio::sync::watch;
use tracing::{info, warn};

use storyforge_domain::agent::PostProcessResult;
use storyforge_domain::agent_profile_config::AgentProfileConfig;
use storyforge_domain::llm::ReasoningMode;
use storyforge_domain::prompt_module::{PromptModule, PromptProfile};

use crate::postprocess::{PostProcessError, run_postprocess};
use crate::runtime::AgentRuntime;
use crate::summarizer::{SummarizerError, run_summarizer_with_prompt};

/// 后处理并行编排的产出
///
/// 两个 Option 独立：任一 Agent 失败，对应字段为 None，另一个照常返回。
/// 调用方拿到结果后自行决定是否落盘 CampaignStore。
#[derive(Debug, Clone, Default)]
pub struct PostProcessOutcome {
    /// 本轮剧情摘要（总结 Agent 成功则有；失败或被 `enable_summarizer=false` 关闭为 None）
    pub summary: Option<String>,
    /// 后处理三件套（后处理 Agent 成功且有产出则有；失败或被 `enable_postprocess=false` 关闭为 None）
    pub post_process: Option<PostProcessResult>,
    /// true 表示本轮确实调用过 Summarizer；用于区分“关闭”与“调用失败”。
    pub summary_attempted: bool,
    /// true 表示本轮确实调用过 PostProcessor；用于区分“关闭”与“调用失败”。
    pub post_process_attempted: bool,
}

/// 并行跑剧情总结 + 后处理 Agent
///
/// 入参 `runtime` 用引用即可——两个子任务共享同一个 runtime（内部 Arc 包装了 LLM client），
/// 子任务的 future 自己 move 进 tokio::spawn。
///
/// 开关语义（来自 `AgentProfileConfig`）：
/// - `enable_summarizer=false`：不调 summarizer LLM，`outcome.summary` 为 None。
/// - `enable_postprocess=false`：不调 postprocess LLM，`outcome.post_process` 为 None。
/// - 两者都 false：安静跳过，两个 future 都不发 LLM 请求。
///
/// `agent_profile_config`（可选）同时覆盖两个 Agent 的 model/rounds 和 PostProcessor 的 tool_whitelist。
///
/// 取消语义：传入的 `cancel` 被 clone 两份分别交给两个子任务，主流水线取消时联动。
#[allow(clippy::too_many_arguments)]
pub async fn run_postprocess_pipeline(
    runtime: &AgentRuntime,
    final_text: &str,
    scene_brief: &str,
    present_characters: &[String],
    variable_keys: &[String],
    turn: u32,
    story_clock: &str,
    cancel: watch::Receiver<bool>,
    enable_postprocess: bool,
    enable_summarizer: bool,
    agent_profile_config: Option<&AgentProfileConfig>,
    recent_summary_block: Option<&str>,
) -> PostProcessOutcome {
    run_postprocess_pipeline_with_prompt(
        runtime,
        final_text,
        scene_brief,
        present_characters,
        variable_keys,
        turn,
        story_clock,
        cancel,
        enable_postprocess,
        enable_summarizer,
        agent_profile_config,
        recent_summary_block,
        None,
        &[],
        &ReasoningMode::Disabled,
        &[],
    )
    .await
}

/// 同 `run_postprocess_pipeline`，可注入 Prompt Module（Summarizer 的 Prompted CoT）
/// 与卡翻译产物的 MVU 变量更新规则（PostProcessor 的【卡片变量更新规则】区块）。
#[allow(clippy::too_many_arguments)]
pub async fn run_postprocess_pipeline_with_prompt(
    runtime: &AgentRuntime,
    final_text: &str,
    scene_brief: &str,
    present_characters: &[String],
    variable_keys: &[String],
    turn: u32,
    story_clock: &str,
    cancel: watch::Receiver<bool>,
    enable_postprocess: bool,
    enable_summarizer: bool,
    agent_profile_config: Option<&AgentProfileConfig>,
    recent_summary_block: Option<&str>,
    prompt_profile: Option<&PromptProfile>,
    prompt_modules: &[PromptModule],
    reasoning: &ReasoningMode,
    mvu_update_rules: &[String],
) -> PostProcessOutcome {
    // 各 clone 一份 cancel 给两个子任务
    let cancel_summary = cancel.clone();
    let cancel_postproc = cancel.clone();

    // 两个子任务需要的、spawn 后跨 await 边界 move 的数据，全部 owned
    let final_for_summary = final_text.to_string();
    let final_for_postproc = final_text.to_string();
    let scene_for_summary = scene_brief.to_string();
    let chars_for_postproc = present_characters.to_vec();
    let keys_for_postproc = variable_keys.to_vec();
    let clock_for_postproc = story_clock.to_string();
    let summary_block_for_postproc = recent_summary_block.map(str::to_string);
    let rules_for_postproc = mvu_update_rules.to_vec();

    // runtime 需要在两个 spawn 里被引用——它在 run_tool_loop 里只借用 &self，
    // 但 spawn 要求 'static，所以这里靠 Arc 包装一份。runtime 内部的 llm/tool_ctx 本就是 Arc。
    // 最简方案：直接用裸指针不行（不 Send）；改用 runtime 的 LLM/ctx 重建不可行（私有字段）。
    // 这里改为：两个 future 在当前 task 里 join，而非 spawn。依然并行（tokio::join! 并发调度）。
    //
    // 注：spawn_subagents 用 spawn 是因为要支持「丢弃超额任务」+ 索引对齐；本场景固定两个任务，
    // 用 tokio::try_join 不合适（任一 Err 会短路，违背「互不影响」），改用 join! + 内部 catch。

    let summary_fut = async {
        if !enable_summarizer {
            info!(target: "postprocess-pipeline", "剧情总结已被 AgentProfileConfig 关闭，跳过");
            return None;
        }
        match run_summarizer_with_prompt(
            runtime,
            &final_for_summary,
            &scene_for_summary,
            turn,
            cancel_summary,
            agent_profile_config,
            prompt_profile,
            prompt_modules,
            reasoning,
        )
        .await
        {
            Ok(s) if s.is_empty() => {
                warn!(target: "postprocess-pipeline", "剧情总结返回空文本");
                None
            }
            Ok(s) => {
                info!(
                    target: "postprocess-pipeline",
                    "剧情总结完成：{} 字",
                    s.chars().count()
                );
                Some(s)
            }
            Err(SummarizerError::Agent(e)) => {
                warn!(target: "postprocess-pipeline", "剧情总结失败（best-effort 跳过）: {e}");
                None
            }
        }
    };

    let postproc_fut = async {
        if !enable_postprocess {
            info!(target: "postprocess-pipeline", "后处理已被 AgentProfileConfig 关闭，跳过");
            return None;
        }
        match run_postprocess(
            runtime,
            &final_for_postproc,
            &chars_for_postproc,
            &keys_for_postproc,
            turn,
            &clock_for_postproc,
            cancel_postproc,
            agent_profile_config,
            summary_block_for_postproc.as_deref(),
            &rules_for_postproc,
        )
        .await
        {
            Ok(r) if !r.parse_succeeded => {
                warn!(
                    target: "postprocess-pipeline",
                    "后处理解析失败（5 层兜底全 miss，best-effort 跳过）"
                );
                None
            }
            Ok(r) if r.is_empty() => {
                info!(target: "postprocess-pipeline", "后处理返回空（LLM 正常返回但无更新）");
                // 空 result 也算成功，返回 Some 让调用方知道「跑过了但没产出」
                Some(r)
            }
            Ok(r) => {
                info!(
                    target: "postprocess-pipeline",
                    "后处理完成：知识 {} / 变量 {} / 任务 {}",
                    r.knowledge_updates.len(),
                    r.variable_updates.len(),
                    r.task_updates.len()
                );
                Some(r)
            }
            Err(PostProcessError::Agent(e)) => {
                warn!(target: "postprocess-pipeline", "后处理失败（best-effort 跳过）: {e}");
                None
            }
        }
    };

    // 并发执行两个 future（同一 task 内多路复用，tokio 调度器并发推进）
    let (summary, post_process) = tokio::join!(summary_fut, postproc_fut);

    PostProcessOutcome {
        summary,
        post_process,
        summary_attempted: enable_summarizer,
        post_process_attempted: enable_postprocess,
    }
}

// ─── 测试 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;
    use std::sync::Arc;
    use storyforge_infra_llm::LlmClient;
    use storyforge_infra_llm::mock_client::MockLlmClient;

    fn make_runtime() -> Arc<AgentRuntime> {
        let client = Arc::new(MockLlmClient::with_defaults()) as Arc<dyn LlmClient>;
        let ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        Arc::new(AgentRuntime::new(client, ctx))
    }

    #[tokio::test]
    async fn test_pipeline_both_succeed_with_mock() {
        let runtime = make_runtime();
        let (_tx, rx) = watch::channel(false);

        let outcome = run_postprocess_pipeline(
            &runtime,
            "林医生走进急诊室，看到地上的尸体。他检查了伤口，确认是刀伤。",
            "急诊室的一场戏",
            &["林医生".into()],
            &["hp".into(), "state".into()],
            1,
            "第1天",
            rx,
            true,
            true,
            None,
            None,
        )
        .await;

        // mock 脚本两个 Agent 都应产出（summary 非空，postprocess 非空）
        assert!(outcome.summary.is_some(), "mock 总结应成功");
        assert!(outcome.post_process.is_some(), "mock 后处理应成功");
        if let Some(pp) = &outcome.post_process {
            // mock 后处理脚本产出至少一条更新（知识/变量/任务任一）
            assert!(
                !pp.is_empty(),
                "mock 后处理应产出非空三件套，实际: 知识{} 变量{} 任务{}",
                pp.knowledge_updates.len(),
                pp.variable_updates.len(),
                pp.task_updates.len()
            );
        }
    }

    #[tokio::test]
    async fn test_pipeline_cancellation_returns_partial() {
        // 立即取消：两个 Agent 都应在第一次 await 前看到 cancel=true
        let runtime = make_runtime();
        let (_tx, rx) = watch::channel(true); // 一开始就 true

        let outcome = run_postprocess_pipeline(
            &runtime,
            "成文",
            "场景",
            &["角色A".into()],
            &["hp".into()],
            1,
            "第1天",
            rx,
            true,
            true,
            None,
            None,
        )
        .await;

        // 取消时 best-effort：两个都失败 → 都为 None
        assert!(outcome.summary.is_none(), "取消后总结应为 None");
        assert!(outcome.post_process.is_none(), "取消后后处理应为 None");
    }

    // ── enable_postprocess / enable_summarizer 开关测试 ──

    #[tokio::test]
    async fn test_pipeline_summarizer_disabled_skips_llm() {
        let runtime = make_runtime();
        let (_tx, rx) = watch::channel(false);

        let outcome = run_postprocess_pipeline(
            &runtime,
            "成文",
            "场景",
            &["角色A".into()],
            &["hp".into()],
            1,
            "第1天",
            rx,
            true,  // postprocess 开
            false, // summarizer 关
            None,
            None,
        )
        .await;

        assert!(outcome.summary.is_none(), "summarizer 关闭应返回 None");
        assert!(!outcome.summary_attempted);
        assert!(outcome.post_process_attempted);
        assert!(outcome.post_process.is_some(), "postprocess 开启应正常产出");
    }

    #[tokio::test]
    async fn test_pipeline_postprocess_disabled_skips_llm() {
        let runtime = make_runtime();
        let (_tx, rx) = watch::channel(false);

        let outcome = run_postprocess_pipeline(
            &runtime,
            "成文",
            "场景",
            &["角色A".into()],
            &["hp".into()],
            1,
            "第1天",
            rx,
            false, // postprocess 关
            true,  // summarizer 开
            None,
            None,
        )
        .await;

        assert!(
            outcome.post_process.is_none(),
            "postprocess 关闭应返回 None"
        );
        assert!(outcome.summary.is_some(), "summarizer 开启应正常产出");
        assert!(outcome.summary_attempted);
        assert!(!outcome.post_process_attempted);
    }

    #[tokio::test]
    async fn test_pipeline_both_disabled_returns_all_none() {
        let runtime = make_runtime();
        let (_tx, rx) = watch::channel(false);

        let outcome = run_postprocess_pipeline(
            &runtime,
            "成文",
            "场景",
            &["角色A".into()],
            &["hp".into()],
            1,
            "第1天",
            rx,
            false,
            false,
            None,
            None,
        )
        .await;

        assert!(outcome.summary.is_none(), "summarizer 关闭应返回 None");
        assert!(
            outcome.post_process.is_none(),
            "postprocess 关闭应返回 None"
        );
    }
}
