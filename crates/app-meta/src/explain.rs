//! 解释本轮生成（确定性，零 LLM）
//!
//! 从 Provenance 提取可读的"生成溯源"文本，供前端展示"为什么这轮这样写"。
//! 纯数据拼接，不调 LLM，不依赖任何运行时状态。

use serde::{Deserialize, Serialize};
use storyforge_domain::conversation::Provenance;

/// 单个子 Agent 的解释条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentExplain {
    /// 角色 ID（character_id）
    pub character_id: String,
    /// 显示名（Campaign 模式下为 instance.name，旧路径与 character_id 相同）
    pub display_name: String,
    /// 该子 Agent 的任务 brief（来自 Plan.subagent_tasks）
    pub task_brief: Option<String>,
    /// 子 Agent 输出的前 300 字摘要
    pub output_preview: String,
    /// fallback 原因（若有，如 "instance not found, fell back to context_package"）
    pub fallback_reason: Option<String>,
    /// 供应商实际返回的子 Agent reasoning/thinking。
    pub reasoning_content: Option<String>,
}

/// 本轮生成的确定性解释（纯数据，零 LLM）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationExplanation {
    /// 场景简述（来自 Plan.scene_brief）
    pub scene_brief: Option<String>,
    /// 各子 Agent 的产出摘要
    pub subagents: Vec<SubagentExplain>,
    /// 上次重 roll 时附加的 hint（若有）
    pub last_hint: Option<String>,
    /// 提示词预设 ID（若有）
    pub profile_id: Option<String>,
    /// 随机种子
    pub seed: u64,
    /// 供应商实际返回的导演 reasoning/thinking。
    pub director_reasoning: Option<String>,
    /// 供应商实际返回的编剧 reasoning/thinking。
    pub editor_reasoning: Option<String>,
}

impl GenerationExplanation {
    /// 给外部 Meta Agent 的安全投影。保留生成结构与摘要，但不把原始 reasoning
    /// 再发送给另一个模型；原文只通过本地显式审计命令展示。
    pub fn without_reasoning(mut self) -> Self {
        self.director_reasoning = None;
        self.editor_reasoning = None;
        for subagent in &mut self.subagents {
            subagent.reasoning_content = None;
        }
        self
    }
}

/// 从 Provenance 确定性生成解释（零 LLM）
///
/// Provenance 的真实字段（确认自 `crates/domain/src/conversation.rs`）：
/// - `session_id: Id` — 写作流水线会话 ID
/// - `plan: Option<Plan>` — Plan 有 `scene_brief: String` + `subagent_tasks: Vec<SubagentTask>`
/// - `subagent_results: Vec<SubagentSnapshot>` — SubagentSnapshot 有 `character_id`, `full_text`,
///   `character_instance_id: Option<String>`, `display_name: Option<String>`,
///   `fallback_reason: Option<String>`
/// - `profile_id: Option<Id>` — 提示词预设 ID
/// - `seed: u64` — 随机种子
/// - `last_hint: Option<String>` — 上次重 roll hint
///
/// Provenance 没有的字段（标 None）：
/// - 无 postprocess 计数/摘要字段 → `postprocess_summary` 在本结构中省略
/// - 无 editor_hint 字段 → 从 Provenance 无法获取 editor 输出
pub fn explain_generation(provenance: &Provenance) -> GenerationExplanation {
    // 场景简述
    let scene_brief = provenance.plan.as_ref().map(|p| p.scene_brief.clone());

    // 构建 character_id → brief 查找表（从 plan.subagent_tasks）
    let task_briefs: std::collections::HashMap<&str, &str> = provenance
        .plan
        .as_ref()
        .map(|p| {
            p.subagent_tasks
                .iter()
                .map(|t| (t.character_id.as_str(), t.brief.as_str()))
                .collect()
        })
        .unwrap_or_default();

    // 子 Agent 产出摘要
    let subagents: Vec<SubagentExplain> = provenance
        .subagent_results
        .iter()
        .map(|s| {
            let char_count = s.full_text.chars().count();
            let output_preview = if char_count > 300 {
                let truncated: String = s.full_text.chars().take(300).collect();
                format!("{truncated}…")
            } else {
                s.full_text.clone()
            };
            SubagentExplain {
                character_id: s.character_id.clone(),
                display_name: s
                    .display_name
                    .clone()
                    .unwrap_or_else(|| s.character_id.clone()),
                task_brief: task_briefs
                    .get(s.character_id.as_str())
                    .map(|b| b.to_string()),
                output_preview,
                fallback_reason: s.fallback_reason.clone(),
                reasoning_content: s.reasoning_content.clone(),
            }
        })
        .collect();

    GenerationExplanation {
        scene_brief,
        subagents,
        last_hint: provenance.last_hint.clone(),
        profile_id: provenance
            .profile_id
            .as_ref()
            .map(|id| id.as_str().to_string()),
        seed: provenance.seed,
        director_reasoning: provenance.director_reasoning.clone(),
        editor_reasoning: provenance.editor_reasoning.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::Id;
    use storyforge_domain::agent::{ContextPackage, Plan, SubagentTask};
    use storyforge_domain::conversation::{Provenance, SubagentSnapshot};

    fn make_context_package() -> ContextPackage {
        ContextPackage {
            character_brief: "".into(),
            scene_brief: "".into(),
            relevant_lore: vec![],
            constant_lore: vec![],
            recent_window: vec![],
            task: "".into(),
        }
    }

    fn make_provenance() -> Provenance {
        Provenance {
            session_id: Id::from_str("sess-001"),
            plan: Some(Plan {
                scene_brief: "雨夜告别场景".into(),
                subagent_tasks: vec![
                    SubagentTask::new(
                        "alice",
                        "扮演 Alice，表达离别的不舍",
                        make_context_package(),
                    ),
                    SubagentTask::new("bob", "扮演 Bob，沉默但内心翻涌", make_context_package()),
                ],
                scene_plan: None,
            }),
            subagent_results: vec![
                SubagentSnapshot {
                    character_id: "alice".into(),
                    full_text: "Alice 望着窗外的雨，手指无意识地摩挲着那封未寄出的信。".into(),
                    character_instance_id: None,
                    display_name: Some("Alice".into()),
                    fallback_reason: None,
                    reasoning_content: Some("alice reasoning".into()),
                },
                SubagentSnapshot {
                    character_id: "bob".into(),
                    full_text:
                        "Bob 站在门框边，雨水顺着屋檐滴落。他没有说话，但攥紧的拳头出卖了他。"
                            .into(),
                    character_instance_id: None,
                    display_name: Some("Bob".into()),
                    fallback_reason: None,
                    reasoning_content: Some("bob reasoning".into()),
                },
            ],
            profile_id: Some(Id::from_str("profile-1")),
            seed: 42,
            last_hint: None,
            director_reasoning: Some("director reasoning".into()),
            editor_reasoning: Some("editor reasoning".into()),
        }
    }

    #[test]
    fn test_explain_generation_basic() {
        let prov = make_provenance();
        let explanation = explain_generation(&prov);

        assert_eq!(explanation.scene_brief.as_deref(), Some("雨夜告别场景"));
        assert_eq!(explanation.subagents.len(), 2);
        assert_eq!(explanation.subagents[0].character_id, "alice");
        assert_eq!(explanation.subagents[0].display_name, "Alice");
        assert_eq!(
            explanation.subagents[0].task_brief.as_deref(),
            Some("扮演 Alice，表达离别的不舍")
        );
        assert!(explanation.subagents[0].output_preview.contains("Alice"));
        assert_eq!(explanation.seed, 42);
        assert_eq!(explanation.profile_id.as_deref(), Some("profile-1"));
        assert_eq!(
            explanation.director_reasoning.as_deref(),
            Some("director reasoning")
        );
        assert_eq!(
            explanation.editor_reasoning.as_deref(),
            Some("editor reasoning")
        );
        assert_eq!(
            explanation.subagents[0].reasoning_content.as_deref(),
            Some("alice reasoning")
        );
    }

    #[test]
    fn test_explain_generation_no_plan() {
        let mut prov = make_provenance();
        prov.plan = None;
        let explanation = explain_generation(&prov);

        assert!(explanation.scene_brief.is_none());
        // 子 Agent 仍有产出，但 task_brief 为 None
        assert_eq!(explanation.subagents.len(), 2);
        assert!(explanation.subagents[0].task_brief.is_none());
    }

    #[test]
    fn test_explain_generation_empty_subagents() {
        let mut prov = make_provenance();
        prov.subagent_results = vec![];
        let explanation = explain_generation(&prov);

        assert!(explanation.subagents.is_empty());
        assert_eq!(explanation.scene_brief.as_deref(), Some("雨夜告别场景"));
    }

    #[test]
    fn test_explain_generation_with_last_hint() {
        let mut prov = make_provenance();
        prov.last_hint = Some("上一轮 Alice 的台词太生硬，请更含蓄".into());
        let explanation = explain_generation(&prov);

        assert_eq!(
            explanation.last_hint.as_deref(),
            Some("上一轮 Alice 的台词太生硬，请更含蓄")
        );
    }

    #[test]
    fn test_explain_generation_long_text_truncated() {
        let mut prov = make_provenance();
        prov.subagent_results[0].full_text = "字".repeat(500);
        let explanation = explain_generation(&prov);

        // 300 字 + "…" = 301 chars
        assert_eq!(explanation.subagents[0].output_preview.chars().count(), 301);
        assert!(explanation.subagents[0].output_preview.ends_with('…'));
    }

    #[test]
    fn test_explain_generation_fallback_reason_preserved() {
        let mut prov = make_provenance();
        prov.subagent_results[0].fallback_reason =
            Some("instance not found, fell back to context_package".into());
        let explanation = explain_generation(&prov);

        assert_eq!(
            explanation.subagents[0].fallback_reason.as_deref(),
            Some("instance not found, fell back to context_package")
        );
    }

    #[test]
    fn test_explain_generation_display_name_fallback() {
        let mut prov = make_provenance();
        prov.subagent_results[0].display_name = None;
        let explanation = explain_generation(&prov);

        // display_name 为 None 时回退到 character_id
        assert_eq!(explanation.subagents[0].display_name, "alice");
    }
}
