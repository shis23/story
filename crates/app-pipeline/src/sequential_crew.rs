use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use storyforge_app_agent::{AgentConfig, AgentError, AgentRuntime, spawn_subagents};
use storyforge_domain::agent::{Performance, PipelineEvent, SubagentTask};
use storyforge_domain::agent_profile_config::AgentProfileConfig;
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use tokio::sync::{mpsc, watch};

const MAX_ATTEMPTS_PER_ACTOR: u8 = 2;

#[derive(Debug, Deserialize)]
struct SequentialPerformanceOutput {
    #[serde(default)]
    narrative: String,
    #[serde(default)]
    dialogue: String,
    #[serde(default)]
    inner_thoughts: String,
    #[serde(default)]
    scene_close: bool,
}

#[derive(Debug)]
pub(crate) struct ParsedSequentialPerformance {
    pub performance: Performance,
    pub scene_close: bool,
}

fn public_performance_text(narrative: &str, dialogue: &str) -> String {
    [narrative.trim(), dialogue.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_json_fence(content: &str) -> &str {
    let trimmed = content.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    rest.trim().strip_suffix("```").unwrap_or(rest).trim()
}

pub(crate) fn parse_sequential_performance(
    actor_id: &str,
    content: &str,
    reasoning_content: Option<String>,
) -> Result<ParsedSequentialPerformance, String> {
    let output: SequentialPerformanceOutput = serde_json::from_str(strip_json_fence(content))
        .map_err(|error| format!("顺序剧组表演必须是结构化 JSON：{error}"))?;
    let full_text = public_performance_text(&output.narrative, &output.dialogue);
    if full_text.is_empty() {
        return Err("顺序剧组表演缺少公开 narrative/dialogue".into());
    }
    Ok(ParsedSequentialPerformance {
        performance: Performance {
            character_id: actor_id.to_string(),
            narrative: output.narrative,
            dialogue: output.dialogue,
            inner_thoughts: output.inner_thoughts,
            full_text,
            reasoning_content,
        },
        scene_close: output.scene_close,
    })
}

pub(crate) fn build_sequential_actor_instruction(
    actor_id: &str,
    task: &str,
    record: &SequentialStageRecord,
    beat_index: usize,
    beat_total: usize,
) -> String {
    format!(
        "你是顺序剧组演员 {actor_id}，正在执行第 {beat_index}/{beat_total} 拍。\n\
         只表演自己的言行，不替其他角色决定，不复述自己的设定。\n\
         你可以根据公开场记回应先前演员，但绝不能读取或泄露其他角色的私密知识与内心。\n\
         本拍任务：{task}\n\n\
         【公开场记】\n{}\n\n\
         只输出一个 JSON 对象，不要 Markdown 围栏：\n\
         {{\"narrative\":\"公开动作与叙述\",\"dialogue\":\"公开对白\",\"inner_thoughts\":\"仅供溯源、不会传给下一位演员\",\"scene_close\":false}}",
        record.render_for_actor(actor_id),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicBeat {
    actor_id: String,
    text: String,
}

/// Monotonic public stage record shared by sequential performers. Private
/// thoughts and provider reasoning never enter this structure.
#[derive(Debug, Clone, Default)]
pub struct SequentialStageRecord {
    opening: String,
    beats: Vec<PublicBeat>,
    failures: HashMap<String, u8>,
}

impl SequentialStageRecord {
    pub fn new(opening: impl Into<String>) -> Self {
        Self {
            opening: opening.into(),
            beats: Vec::new(),
            failures: HashMap::new(),
        }
    }

    pub fn push_performance(&mut self, performance: &Performance) {
        let public_text = public_performance_text(&performance.narrative, &performance.dialogue);
        if !public_text.is_empty() {
            self.beats.push(PublicBeat {
                actor_id: performance.character_id.clone(),
                text: public_text,
            });
        }
    }

    pub fn render_for_actor(&self, _actor_id: &str) -> String {
        let mut sections = Vec::with_capacity(self.beats.len() + 1);
        if !self.opening.trim().is_empty() {
            sections.push(format!("[用户/开场]\n{}", self.opening.trim()));
        }
        sections.extend(
            self.beats
                .iter()
                .map(|beat| format!("[{}]\n{}", beat.actor_id, beat.text)),
        );
        sections.join("\n\n")
    }

    pub fn record_failure(&mut self, actor_id: &str, _safe_class: &str) {
        let count = self.failures.entry(actor_id.to_string()).or_default();
        *count = count.saturating_add(1);
    }

    #[cfg(test)]
    pub fn failure_count(&self, actor_id: &str) -> u8 {
        self.failures.get(actor_id).copied().unwrap_or(0)
    }

    #[cfg(test)]
    pub fn should_stop_actor(&self, actor_id: &str) -> bool {
        self.failure_count(actor_id) >= 2
    }
}

/// Runs character performances one at a time. Only the monotonic public stage
/// record is forwarded to the next actor; private thoughts and provider
/// reasoning remain in the returned provenance payload.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_sequential_crew(
    tasks: Vec<SubagentTask>,
    runtime: Arc<AgentRuntime>,
    director_config: &AgentConfig,
    base_system_prompt: &str,
    cancel: watch::Receiver<bool>,
    event_tx: mpsc::UnboundedSender<PipelineEvent>,
    campaign_runtime: Option<Arc<CampaignRuntimeContext>>,
    agent_profile_config: Option<&AgentProfileConfig>,
    recent_summary_block: Option<&str>,
    far_memory_block: Option<&str>,
    opening: &str,
) -> Vec<Result<Performance, AgentError>> {
    let total = tasks.len();
    run_sequential_crew_suffix(
        tasks,
        &[],
        0,
        total,
        runtime,
        director_config,
        base_system_prompt,
        cancel,
        event_tx,
        campaign_runtime,
        agent_profile_config,
        recent_summary_block,
        far_memory_block,
        opening,
    )
    .await
}

/// Replays a dependency-safe suffix of a sequential performance. Accepted
/// prefix performances seed only the public stage record; the selected actor
/// and every downstream actor run again with their original beat numbers.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_sequential_crew_suffix(
    tasks: Vec<SubagentTask>,
    prefix_performances: &[Performance],
    beat_offset: usize,
    beat_total: usize,
    runtime: Arc<AgentRuntime>,
    director_config: &AgentConfig,
    base_system_prompt: &str,
    cancel: watch::Receiver<bool>,
    event_tx: mpsc::UnboundedSender<PipelineEvent>,
    campaign_runtime: Option<Arc<CampaignRuntimeContext>>,
    agent_profile_config: Option<&AgentProfileConfig>,
    recent_summary_block: Option<&str>,
    far_memory_block: Option<&str>,
    opening: &str,
) -> Vec<Result<Performance, AgentError>> {
    let total = beat_total.max(beat_offset.saturating_add(tasks.len()));
    let mut stage = SequentialStageRecord::new(opening);
    for performance in prefix_performances {
        stage.push_performance(performance);
    }
    let mut results = Vec::with_capacity(tasks.len());
    let sequential_system = format!(
        "{base_system_prompt}\n\n你正在执行顺序剧组演员协议：严格服从本拍任务和结构化输出契约。"
    );
    let mut scene_closed = false;

    for (index, original_task) in tasks.into_iter().enumerate() {
        if *cancel.borrow() || scene_closed {
            results.push(Err(AgentError::Cancelled));
            continue;
        }

        let actor_id = original_task.character_id.clone();
        let original_instruction = original_task.context_package.task.clone();
        let mut accepted: Option<ParsedSequentialPerformance> = None;
        let mut last_error = String::new();

        for attempt in 1..=MAX_ATTEMPTS_PER_ACTOR {
            if *cancel.borrow() {
                last_error = "流水线已取消".into();
                break;
            }

            let mut task = original_task.clone();
            task.context_package.task = build_sequential_actor_instruction(
                &actor_id,
                &original_instruction,
                &stage,
                beat_offset + index + 1,
                total,
            );
            if attempt > 1 {
                task.context_package.task.push_str(
                    "\n\n上次输出无法安全解析。请只返回符合契约的 JSON，并确保 narrative 或 dialogue 至少一项非空。",
                );
            }

            let (attempt_event_tx, mut attempt_event_rx) =
                mpsc::unbounded_channel::<PipelineEvent>();
            let forwarded_event_tx = event_tx.clone();
            let forwarded_actor_id = actor_id.clone();
            let forward = tokio::spawn(async move {
                while let Some(event) = attempt_event_rx.recv().await {
                    if let PipelineEvent::SubagentProgress { delta, .. } = event {
                        let _ = forwarded_event_tx.send(PipelineEvent::SubagentProgress {
                            character_id: forwarded_actor_id.clone(),
                            index: beat_offset + index,
                            delta,
                        });
                    }
                }
            });

            let call_event_tx = attempt_event_tx.clone();
            let mut call_results = spawn_subagents(
                vec![task],
                runtime.clone(),
                director_config,
                &sequential_system,
                cancel.clone(),
                call_event_tx,
                campaign_runtime.clone(),
                1,
                agent_profile_config,
                recent_summary_block,
                far_memory_block,
            )
            .await;
            drop(attempt_event_tx);
            let _ = forward.await;

            let Some(call_result) = call_results.pop() else {
                last_error = "顺序剧组未返回演员结果".into();
                stage.record_failure(&actor_id, "missing_result");
                continue;
            };
            match call_result {
                Ok(performance) => match parse_sequential_performance(
                    &actor_id,
                    &performance.full_text,
                    performance.reasoning_content,
                ) {
                    Ok(parsed) => {
                        accepted = Some(parsed);
                        break;
                    }
                    Err(error) => {
                        last_error = error;
                        stage.record_failure(&actor_id, "parse");
                    }
                },
                Err(error) => {
                    last_error = error.to_string();
                    stage.record_failure(&actor_id, "agent");
                }
            }
        }

        match accepted {
            Some(parsed) => {
                stage.push_performance(&parsed.performance);
                scene_closed = parsed.scene_close;
                results.push(Ok(parsed.performance));
            }
            None => results.push(Err(AgentError::SubagentFailed(format!(
                "顺序剧组演员 {actor_id} 在 {MAX_ATTEMPTS_PER_ACTOR} 次尝试后失败: {last_error}"
            )))),
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use storyforge_app_agent::{AgentConfig, AgentRuntime, ToolContext};
    use storyforge_domain::agent::{AgentRole, ContextPackage, PipelineEvent, SubagentTask};
    use storyforge_domain::llm::ChatMessage;
    use storyforge_infra_llm::LlmClient;
    use storyforge_infra_llm::mock_client::{MockLlmClient, MockScript};
    use tokio::sync::{mpsc, watch};

    type CapturedPrompts = Arc<Mutex<Vec<(AgentRole, Vec<ChatMessage>)>>>;

    fn performance(actor: &str, narrative: &str, dialogue: &str, inner: &str) -> Performance {
        Performance {
            character_id: actor.to_string(),
            narrative: narrative.to_string(),
            dialogue: dialogue.to_string(),
            inner_thoughts: inner.to_string(),
            full_text: format!("{narrative}\n{dialogue}\n{inner}"),
            reasoning_content: None,
        }
    }

    fn task(actor: &str) -> SubagentTask {
        SubagentTask {
            character_id: actor.to_string(),
            brief: format!("让 {actor} 回应现场"),
            context_package: ContextPackage {
                character_brief: format!("{actor} 的人设"),
                scene_brief: "密室中的对峙".into(),
                relevant_lore: vec![],
                constant_lore: vec![],
                recent_window: vec![],
                task: format!("让 {actor} 回应现场"),
            },
            current_desire: None,
            ongoing_action: None,
            emotion_stage: None,
        }
    }

    #[tokio::test]
    async fn runner_passes_only_prior_public_performance_to_next_actor() {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::new(vec![MockScript {
            match_keyword: "顺序剧组演员".into(),
            response_content: serde_json::json!({
                "narrative": "演员把信封推到桌面中央。",
                "dialogue": "你自己看。",
                "inner_thoughts": "绝不能让下一位演员知道。",
                "scene_close": false
            })
            .to_string(),
            tool_calls: vec![],
            stream: false,
        }]));
        let captured: CapturedPrompts = Arc::new(Mutex::new(Vec::new()));
        let captured_for_hook = captured.clone();
        let hook = Arc::new(
            move |ctx: storyforge_app_agent::runtime::PromptHookContext| {
                let captured = captured_for_hook.clone();
                Box::pin(async move {
                    captured
                        .lock()
                        .unwrap()
                        .push((ctx.role, ctx.messages.clone()));
                    Ok(ctx.messages)
                }) as storyforge_app_agent::runtime::PromptHookFuture
            },
        );
        let runtime = Arc::new(AgentRuntime::with_prompt_hook(
            llm,
            Arc::new(ToolContext::empty()),
            hook,
        ));
        let director_config = AgentConfig {
            role: AgentRole::Director,
            system_prompt: String::new(),
            max_tool_rounds: 1,
            model: "mock".into(),
            tools: vec![],
            terminal_tools: vec![],
        };
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let results = run_sequential_crew(
            vec![task("actor-a"), task("actor-b")],
            runtime,
            &director_config,
            "角色表演基础约束",
            cancel_rx,
            event_tx,
            None,
            None,
            None,
            None,
            "用户把灯关掉。",
        )
        .await;

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(Result::is_ok));
        let calls = captured.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let second_prompt = calls[1]
            .1
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(second_prompt.contains("演员把信封推到桌面中央"));
        assert!(second_prompt.contains("你自己看"));
        assert!(!second_prompt.contains("绝不能让下一位演员知道"));
    }

    #[tokio::test]
    async fn suffix_replay_reuses_public_prefix_and_keeps_original_beat_numbers() {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::new(vec![MockScript {
            match_keyword: "顺序剧组演员".into(),
            response_content: serde_json::json!({
                "narrative": "重演演员改变了站位。",
                "dialogue": "继续。",
                "inner_thoughts": "新的私密判断。",
                "scene_close": false
            })
            .to_string(),
            tool_calls: vec![],
            stream: false,
        }]));
        let captured: CapturedPrompts = Arc::new(Mutex::new(Vec::new()));
        let captured_for_hook = captured.clone();
        let hook = Arc::new(
            move |ctx: storyforge_app_agent::runtime::PromptHookContext| {
                let captured = captured_for_hook.clone();
                Box::pin(async move {
                    captured
                        .lock()
                        .unwrap()
                        .push((ctx.role, ctx.messages.clone()));
                    Ok(ctx.messages)
                }) as storyforge_app_agent::runtime::PromptHookFuture
            },
        );
        let runtime = Arc::new(AgentRuntime::with_prompt_hook(
            llm,
            Arc::new(ToolContext::empty()),
            hook,
        ));
        let director_config = AgentConfig {
            role: AgentRole::Director,
            system_prompt: String::new(),
            max_tool_rounds: 1,
            model: "mock".into(),
            tools: vec![],
            terminal_tools: vec![],
        };
        let prefix = performance(
            "actor-a",
            "A 已经把信封推到桌面中央。",
            "轮到你了。",
            "旧的私密判断绝不能进入重演场记。",
        );
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<PipelineEvent>();
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let results = run_sequential_crew_suffix(
            vec![task("actor-b"), task("actor-c")],
            &[prefix],
            1,
            3,
            runtime,
            &director_config,
            "角色表演基础约束",
            cancel_rx,
            event_tx,
            None,
            None,
            None,
            None,
            "用户要求重演后半场。",
        )
        .await;

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(Result::is_ok));
        let calls = captured.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let first_prompt = calls[0]
            .1
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let second_prompt = calls[1]
            .1
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(first_prompt.contains("第 2/3 拍"));
        assert!(first_prompt.contains("A 已经把信封推到桌面中央"));
        assert!(!first_prompt.contains("旧的私密判断"));
        assert!(second_prompt.contains("第 3/3 拍"));
        assert!(second_prompt.contains("重演演员改变了站位"));
        assert!(!second_prompt.contains("新的私密判断"));
    }

    #[test]
    fn stage_record_contains_only_public_narrative_and_dialogue() {
        let mut record = SequentialStageRecord::new("用户把信封放在桌上");
        record.push_performance(&performance(
            "actor-a",
            "A 把信封推向 B。",
            "你自己看。",
            "A 其实希望 B 拒绝。",
        ));

        let rendered = record.render_for_actor("actor-b");

        assert!(rendered.contains("用户把信封放在桌上"));
        assert!(rendered.contains("A 把信封推向 B。"));
        assert!(rendered.contains("你自己看。"));
        assert!(!rendered.contains("希望 B 拒绝"));
    }

    #[test]
    fn stage_record_keeps_strict_execution_order() {
        let mut record = SequentialStageRecord::new("第 0 拍");
        record.push_performance(&performance("a", "第 1 拍", "", "秘密一"));
        record.push_performance(&performance("b", "第 2 拍", "", "秘密二"));
        record.push_performance(&performance("c", "第 3 拍", "", "秘密三"));

        let rendered = record.render_for_actor("d");
        let first = rendered.find("第 1 拍").unwrap();
        let second = rendered.find("第 2 拍").unwrap();
        let third = rendered.find("第 3 拍").unwrap();

        assert!(first < second && second < third);
        assert!(!rendered.contains("秘密一"));
        assert!(!rendered.contains("秘密二"));
        assert!(!rendered.contains("秘密三"));
    }

    #[test]
    fn failed_actor_does_not_erase_existing_public_stage_record() {
        let mut record = SequentialStageRecord::new("开场");
        record.push_performance(&performance("a", "A 已经开口", "你好", "不要泄露"));

        let before = record.render_for_actor("b");
        record.record_failure("b", "timeout");
        let after = record.render_for_actor("c");

        assert!(after.contains("A 已经开口"));
        assert!(after.contains("你好"));
        assert_eq!(record.failure_count("b"), 1);
        assert!(after.len() >= before.len());
        assert!(!after.contains("timeout"));
    }

    #[test]
    fn actor_is_stopped_after_two_failures() {
        let mut record = SequentialStageRecord::new("开场");
        record.record_failure("b", "network");
        assert!(!record.should_stop_actor("b"));
        record.record_failure("b", "parse");
        assert!(record.should_stop_actor("b"));
    }

    #[test]
    fn structured_performance_keeps_inner_thoughts_out_of_public_full_text() {
        let parsed = parse_sequential_performance(
            "actor-a",
            r#"{"narrative":"A 推出信封。","dialogue":"你自己看。","inner_thoughts":"希望他拒绝。","scene_close":false}"#,
            Some("provider reasoning".into()),
        )
        .unwrap();

        assert_eq!(parsed.performance.narrative, "A 推出信封。");
        assert_eq!(parsed.performance.dialogue, "你自己看。");
        assert_eq!(parsed.performance.inner_thoughts, "希望他拒绝。");
        assert!(parsed.performance.full_text.contains("A 推出信封"));
        assert!(!parsed.performance.full_text.contains("希望他拒绝"));
        assert!(!parsed.scene_close);
    }

    #[test]
    fn structured_performance_accepts_json_code_fence() {
        let parsed = parse_sequential_performance(
            "actor-b",
            "```json\n{\"narrative\":\"B 按住信封。\",\"dialogue\":\"\",\"inner_thoughts\":\"迟疑\",\"scene_close\":true}\n```",
            None,
        )
        .unwrap();

        assert!(parsed.scene_close);
        assert_eq!(parsed.performance.full_text, "B 按住信封。");
    }

    #[test]
    fn unstructured_performance_is_rejected_instead_of_leaking_private_text() {
        let error = parse_sequential_performance("actor-a", "A 表面答应，心里却决定背叛。", None)
            .unwrap_err();

        assert!(error.contains("结构化"));
    }

    #[test]
    fn next_actor_instruction_contains_public_stage_and_output_contract_only() {
        let mut record = SequentialStageRecord::new("用户质问信封来源");
        record.push_performance(&performance(
            "actor-a",
            "A 推出信封。",
            "你自己看。",
            "希望 B 不敢拆。",
        ));

        let instruction = build_sequential_actor_instruction("actor-b", "追问 A", &record, 2, 3);

        assert!(instruction.contains("A 推出信封"));
        assert!(instruction.contains("你自己看"));
        assert!(instruction.contains("第 2/3 拍"));
        assert!(instruction.contains("scene_close"));
        assert!(!instruction.contains("希望 B 不敢拆"));
    }
}
