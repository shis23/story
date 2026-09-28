use std::collections::HashSet;

use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::character::Character;
use storyforge_domain::character_knowledge::PropagationPolicy;
use storyforge_domain::story_task::StoryTask;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KnowledgeVisibility {
    Open,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DossierKnowledge {
    pub id: String,
    pub text: String,
    pub visibility: KnowledgeVisibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RosterEntry {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActorDossier {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub behavior: String,
    pub agenda: Option<String>,
    pub variables: Vec<(String, String)>,
    pub knowledge: Vec<DossierKnowledge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CognitiveBoundary {
    pub fact_id: String,
    pub fact_text: String,
    pub knower_id: String,
    pub excluded_actor_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledTurnDossier {
    pub intent: String,
    pub roster: Vec<RosterEntry>,
    pub actors: Vec<ActorDossier>,
    pub cognitive_boundaries: Vec<CognitiveBoundary>,
    pub pending_tasks: String,
}

impl CompiledTurnDossier {
    pub(crate) fn render_for_writer(&self) -> String {
        let mut sections = vec![format!("## 用户意图\n{}", self.intent.trim())];
        let roster = self
            .roster
            .iter()
            .map(|actor| format!("- {}（{}）", actor.name, actor.id))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("## 全名册（每人仅一行）\n{roster}"));

        let mut actor_blocks = Vec::new();
        for actor in &self.actors {
            let mut lines = vec![format!("### {}（{}）", actor.name, actor.id)];
            if !actor.persona.trim().is_empty() {
                lines.push(format!("人设：{}", actor.persona.trim()));
            }
            if !actor.behavior.trim().is_empty() {
                lines.push(format!("行为准则：{}", actor.behavior.trim()));
            }
            if let Some(agenda) = &actor.agenda {
                lines.push(format!("当前议程：{agenda}"));
            }
            if !actor.variables.is_empty() {
                lines.push(format!(
                    "关键变量：{}",
                    actor
                        .variables
                        .iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect::<Vec<_>>()
                        .join("；")
                ));
            }
            for fact in &actor.knowledge {
                let label = match fact.visibility {
                    KnowledgeVisibility::Open => format!("[{}知道]", actor.name),
                    KnowledgeVisibility::Private => {
                        format!("[仅{}知道·秘密]", actor.name)
                    }
                };
                lines.push(format!("{label} {}", fact.text));
            }
            actor_blocks.push(lines.join("\n"));
        }
        if !actor_blocks.is_empty() {
            sections.push(format!(
                "## 预期在场角色档案\n{}",
                actor_blocks.join("\n\n")
            ));
        }

        let mut boundary_lines = Vec::new();
        for boundary in &self.cognitive_boundaries {
            for excluded_id in &boundary.excluded_actor_ids {
                let excluded_name = self
                    .roster
                    .iter()
                    .find(|actor| &actor.id == excluded_id)
                    .map(|actor| actor.name.as_str())
                    .unwrap_or(excluded_id);
                boundary_lines.push(format!(
                    "- {excluded_name}不知道：{}（仅 {} 持有；事实 id={}）",
                    boundary.fact_text, boundary.knower_id, boundary.fact_id
                ));
            }
        }
        if !boundary_lines.is_empty() {
            sections.push(format!(
                "## 认知边界（硬约束）\n{}\n不得让无知者在被公开告知前说出、确认或据此行动。",
                boundary_lines.join("\n")
            ));
        }
        if !self.pending_tasks.trim().is_empty() {
            sections.push(format!("## 未了任务与伏笔\n{}", self.pending_tasks.trim()));
        }
        sections.join("\n\n")
    }
}

fn json_value_text(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn normalized_fact(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace() && !character.is_ascii_punctuation())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `story_clock`：真实故事时钟（D-04，不得传空串，否则 StoryTime 触发器恒不命中）。
pub(crate) fn compile_turn_dossier(
    intent: &str,
    runtime: &CampaignRuntimeContext,
    pending_tasks: &[StoryTask],
    story_clock: &str,
    max_full_actors: usize,
) -> CompiledTurnDossier {
    let roster = runtime
        .instances
        .iter()
        .map(|instance| RosterEntry {
            id: instance.id.as_str().to_string(),
            name: instance.name.clone(),
        })
        .collect::<Vec<_>>();

    let mentioned_ids = runtime
        .instances
        .iter()
        .filter(|instance| intent.contains(&instance.name) || intent.contains(instance.id.as_str()))
        .map(|instance| instance.id.as_str().to_string())
        .collect::<HashSet<_>>();
    let task_related_ids = pending_tasks
        .iter()
        .filter(|task| task.status.is_injectable())
        .flat_map(|task| task.related_characters.iter())
        .map(|id| id.as_str().to_string())
        .collect::<HashSet<_>>();

    let mut ranked = runtime.instances.iter().enumerate().collect::<Vec<_>>();
    ranked.sort_by_key(|(original_index, instance)| {
        let id = instance.id.as_str();
        let priority = if mentioned_ids.contains(id) {
            0
        } else if task_related_ids.contains(id) {
            1
        } else {
            2
        };
        (priority, *original_index)
    });
    let selected = ranked
        .into_iter()
        .take(max_full_actors)
        .map(|(_, instance)| instance)
        .collect::<Vec<_>>();
    let selected_ids = selected
        .iter()
        .map(|instance| instance.id.as_str().to_string())
        .collect::<HashSet<_>>();

    let actors = selected
        .iter()
        .map(|instance| {
            let agenda = instance
                .variables
                .iter()
                .find(|value| {
                    matches!(
                        value.key.as_str(),
                        "agenda" | "current_desire" | "goal" | "ongoing_action"
                    )
                })
                .map(|value| json_value_text(&value.value))
                .filter(|value| !value.trim().is_empty());
            let variables = instance
                .variables
                .iter()
                .filter(|value| {
                    !matches!(
                        value.key.as_str(),
                        "agenda" | "current_desire" | "goal" | "ongoing_action"
                    )
                })
                .map(|value| (value.key.clone(), json_value_text(&value.value)))
                .collect();
            let knowledge = runtime
                .knowledge_for_instance(instance)
                .into_iter()
                .map(|fact| DossierKnowledge {
                    id: fact.id.as_str().to_string(),
                    text: fact.knowledge_text.clone(),
                    visibility: if matches!(fact.propagation, PropagationPolicy::Private) {
                        KnowledgeVisibility::Private
                    } else {
                        KnowledgeVisibility::Open
                    },
                })
                .collect();
            ActorDossier {
                id: instance.id.as_str().to_string(),
                name: instance.name.clone(),
                persona: runtime
                    .resolved_persona_for(instance)
                    .unwrap_or_default()
                    .to_string(),
                behavior: runtime
                    .resolved_behavior_for(instance)
                    .unwrap_or_default()
                    .to_string(),
                agenda,
                variables,
                knowledge,
            }
        })
        .collect::<Vec<_>>();

    let cognitive_boundaries = runtime
        .knowledge
        .iter()
        .filter(|fact| {
            selected_ids.contains(fact.character_id.as_str())
                && matches!(fact.propagation, PropagationPolicy::Private)
        })
        .filter_map(|fact| {
            let normalized = normalized_fact(&fact.knowledge_text);
            let excluded_actor_ids = selected
                .iter()
                .filter(|instance| instance.id != fact.character_id)
                .filter(|instance| {
                    !runtime
                        .knowledge_for_instance(instance)
                        .iter()
                        .any(|known| normalized_fact(&known.knowledge_text) == normalized)
                })
                .map(|instance| instance.id.as_str().to_string())
                .collect::<Vec<_>>();
            (!excluded_actor_ids.is_empty()).then(|| CognitiveBoundary {
                fact_id: fact.id.as_str().to_string(),
                fact_text: fact.knowledge_text.clone(),
                knower_id: fact.character_id.as_str().to_string(),
                excluded_actor_ids,
            })
        })
        .collect();

    let pending_tasks = storyforge_domain::story_task::render_tasks_for_injection(
        pending_tasks,
        runtime.turn,
        story_clock,
    );
    CompiledTurnDossier {
        intent: intent.to_string(),
        roster,
        actors,
        cognitive_boundaries,
        pending_tasks,
    }
}

pub(crate) fn compile_legacy_turn_dossier(
    intent: &str,
    characters: &[std::sync::Arc<Character>],
    max_full_actors: usize,
) -> CompiledTurnDossier {
    let roster = characters
        .iter()
        .map(|character| RosterEntry {
            id: character.id.as_str().to_string(),
            name: character.name.clone(),
        })
        .collect::<Vec<_>>();
    let mut ranked = characters.iter().enumerate().collect::<Vec<_>>();
    ranked.sort_by_key(|(index, character)| {
        let mentioned = intent.contains(&character.name) || intent.contains(character.id.as_str());
        (usize::from(!mentioned), *index)
    });
    let actors = ranked
        .into_iter()
        .take(max_full_actors)
        .map(|(_, character)| ActorDossier {
            id: character.id.as_str().to_string(),
            name: character.name.clone(),
            persona: [
                character.description.trim(),
                character.personality.trim(),
                character.scenario.trim(),
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
            behavior: character.post_history_instructions.clone(),
            agenda: None,
            variables: vec![],
            knowledge: vec![],
        })
        .collect();
    CompiledTurnDossier {
        intent: intent.to_string(),
        roster,
        actors,
        cognitive_boundaries: vec![],
        pending_tasks: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use storyforge_domain::Id;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
    use storyforge_domain::character_knowledge::{
        CharacterKnowledgeEntry, KnowledgeSource, PropagationPolicy,
    };
    use storyforge_domain::variables::VariableValue;

    use super::*;

    fn runtime() -> CampaignRuntimeContext {
        let campaign = Campaign::new(Id::from_str("card"), "案卷测试");
        let mut a = CharacterInstance::temporary(campaign.id.clone(), "林如");
        a.id = Id::from_str("actor-a");
        a.persona_override = Some("冷静的调查员".into());
        a.behavior_override = Some("先观察再开口".into());
        a.variables = vec![
            VariableValue::new("agenda", serde_json::json!("找出内鬼"), 2),
            VariableValue::new("mood", serde_json::json!("警惕"), 2),
        ];
        let mut b = CharacterInstance::temporary(campaign.id.clone(), "陈默");
        b.id = Id::from_str("actor-b");
        let mut c = CharacterInstance::temporary(campaign.id.clone(), "周岚");
        c.id = Id::from_str("actor-c");
        let mut d = CharacterInstance::temporary(campaign.id.clone(), "路人");
        d.id = Id::from_str("actor-d");

        let private = CharacterKnowledgeEntry {
            id: Id::from_str("knowledge-secret"),
            campaign_id: campaign.id.clone(),
            character_id: a.id.clone(),
            knowledge_text: "钥匙藏在钟里".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            source_knowledge_id: None,
            turn_number: 2,
            event_id: None,
            pinned: false,
            propagation: PropagationPolicy::Private,
        };
        let open = CharacterKnowledgeEntry {
            id: Id::from_str("knowledge-open"),
            campaign_id: campaign.id.clone(),
            character_id: b.id.clone(),
            knowledge_text: "今晚会下雨".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(a.id.clone()),
            source_knowledge_id: None,
            turn_number: 2,
            event_id: None,
            pinned: false,
            propagation: PropagationPolicy::Open,
        };

        CampaignRuntimeContext {
            campaign,
            instances: vec![a, b, c, d],
            definitions_by_id: HashMap::new(),
            knowledge: vec![private, open],
            tasks: vec![],
            turn: 3,
        }
    }

    /// 域1 `render_tasks_for_injection` 的两个分组标题（D-04 裁定：两组不得混用）。
    const SATISFIED_TITLE: &str = "【已满足条件的任务/伏笔】";
    const PENDING_TITLE: &str = "【待判断的任务/伏笔】";

    /// 取某分组标题覆盖的文本片段（已满足组到待判断标题为止；待判断组到结尾）。
    fn group_text<'a>(text: &'a str, title: &str) -> &'a str {
        let Some(start) = text.find(title) else {
            return "";
        };
        let rest = &text[start..];
        if title == SATISFIED_TITLE
            && let Some(end) = rest.find(PENDING_TITLE)
        {
            return &rest[..end];
        }
        rest
    }

    /// D-04：StoryTime 触发器必须用**真实** story_clock 判定。
    ///
    /// 根因修复：旧实现把 `""` 当故事时钟传给 `render_tasks_for_injection`，导致
    /// 匹配的伏笔无法确定性命中（不匹配的也可能被误判）。现在：
    /// 匹配 → `Satisfied`（注入"已满足"组）；不匹配 → `NotSatisfied`（不注入）。
    #[test]
    fn story_time_task_injected_only_with_matching_real_clock() {
        let runtime = runtime();
        let task = StoryTask::user_planned(
            runtime.campaign.id.clone(),
            "钟楼约定",
            "第2天夜里在钟楼碰头",
            vec![storyforge_domain::story_task::TaskTrigger::StoryTime {
                target: "第2天".into(),
            }],
            1,
        );
        let tasks = vec![task];

        // 真实时钟匹配 → 注入「已满足」组
        let matched = compile_turn_dossier("林如质问陈默", &runtime, &tasks, "第2天", 3);
        assert!(
            matched.pending_tasks.contains("钟楼约定"),
            "时钟匹配应注入: {}",
            matched.pending_tasks
        );
        assert!(
            group_text(&matched.pending_tasks, SATISFIED_TITLE).contains("钟楼约定"),
            "确定性命中的任务必须在「已满足」组: {}",
            matched.pending_tasks
        );
        assert!(
            !group_text(&matched.pending_tasks, PENDING_TITLE).contains("钟楼约定"),
            "确定性命中的任务不得混进「待判断」组: {}",
            matched.pending_tasks
        );

        // 不匹配的时钟 → 不注入（旧实现在这里会误注入/误判）
        let mismatched = compile_turn_dossier("林如质问陈默", &runtime, &tasks, "第3天", 3);
        assert!(
            !mismatched.pending_tasks.contains("钟楼约定"),
            "不匹配时钟不得命中: {}",
            mismatched.pending_tasks
        );
    }

    /// D-04：时钟缺失（调用方没接故事时钟）不是"确定性判否"，也不是"无条件注入"：
    /// 域侧 `check_trigger` 返回 `NeedsAgentJudgment`，该任务进入**待判断**分组，
    /// 由 Agent 判断是否适用（渲染分组标题由域1 的 `render_tasks_for_injection` 负责，
    /// 本域只保证传真实时钟 + 不改分类语义）。
    #[test]
    fn story_time_task_without_clock_goes_to_pending_judgment_group() {
        use storyforge_domain::story_task::TriggerCheck;

        let runtime = runtime();
        let task = StoryTask::user_planned(
            runtime.campaign.id.clone(),
            "钟楼约定",
            "第2天夜里在钟楼碰头",
            vec![storyforge_domain::story_task::TaskTrigger::StoryTime {
                target: "第2天".into(),
            }],
            1,
        );

        // 分类语义（域侧接口约定）：空时钟 = 需 Agent 判断；真实时钟匹配 = 确定性满足
        assert_eq!(task.check_trigger(3, ""), TriggerCheck::NeedsAgentJudgment);
        assert_ne!(task.check_trigger(3, ""), TriggerCheck::Satisfied);
        assert_eq!(task.check_trigger(3, "第2天"), TriggerCheck::Satisfied);

        // 必须仍出现在待注入集合里（否则长程伏笔静默蒸发），且只能落在「待判断」组
        let tasks = vec![task];
        let dossier = compile_turn_dossier("林如质问陈默", &runtime, &tasks, "", 3);
        assert!(
            dossier.pending_tasks.contains("钟楼约定"),
            "空时钟应进入待判断分组并注入: {}",
            dossier.pending_tasks
        );
        assert!(
            group_text(&dossier.pending_tasks, PENDING_TITLE).contains("钟楼约定"),
            "空时钟的任务必须在「待判断」组: {}",
            dossier.pending_tasks
        );
        assert!(
            !group_text(&dossier.pending_tasks, SATISFIED_TITLE).contains("钟楼约定"),
            "空时钟不得被当作「已满足」提前触发: {}",
            dossier.pending_tasks
        );
        assert!(
            dossier.pending_tasks.contains("不要提前揭示"),
            "「待判断」组必须带防剧透指令: {}",
            dossier.pending_tasks
        );
    }

    #[test]
    fn compiler_prioritizes_named_actors_and_caps_full_dossiers_at_three() {
        let dossier = compile_turn_dossier("陈默逼问林如，周岚在门口旁听", &runtime(), &[], "", 3);

        assert_eq!(dossier.actors.len(), 3);
        assert_eq!(
            dossier
                .actors
                .iter()
                .map(|actor| actor.name.as_str())
                .collect::<Vec<_>>(),
            vec!["林如", "陈默", "周岚"]
        );
        assert_eq!(dossier.roster.len(), 4, "全名册仍应保留一行/人");
    }

    #[test]
    fn private_knowledge_is_owner_labeled_and_creates_conservative_boundary() {
        let dossier = compile_turn_dossier("林如质问陈默", &runtime(), &[], "", 3);
        let owner = dossier
            .actors
            .iter()
            .find(|actor| actor.id == "actor-a")
            .unwrap();
        let other = dossier
            .actors
            .iter()
            .find(|actor| actor.id == "actor-b")
            .unwrap();

        assert!(owner.knowledge.iter().any(|fact| {
            fact.text == "钥匙藏在钟里" && fact.visibility == KnowledgeVisibility::Private
        }));
        assert!(
            other
                .knowledge
                .iter()
                .all(|fact| fact.text != "钥匙藏在钟里")
        );
        assert!(dossier.cognitive_boundaries.iter().any(|boundary| {
            boundary.fact_id == "knowledge-secret"
                && boundary.knower_id == "actor-a"
                && boundary.excluded_actor_ids.contains(&"actor-b".to_string())
        }));
    }

    #[test]
    fn missing_open_fact_never_becomes_an_ignorance_boundary() {
        let dossier = compile_turn_dossier("林如质问陈默", &runtime(), &[], "", 3);

        assert!(
            dossier
                .cognitive_boundaries
                .iter()
                .all(|boundary| boundary.fact_id != "knowledge-open")
        );
    }

    #[test]
    fn rendered_dossier_carries_ids_agenda_variables_and_ownership_labels() {
        let dossier = compile_turn_dossier("林如质问陈默", &runtime(), &[], "", 3);
        let rendered = dossier.render_for_writer();

        assert!(rendered.contains("林如（actor-a）"));
        assert!(rendered.contains("当前议程：找出内鬼"));
        assert!(rendered.contains("mood=警惕"));
        assert!(rendered.contains("[仅林如知道·秘密] 钥匙藏在钟里"));
        assert!(rendered.contains("陈默不知道：钥匙藏在钟里"));
        assert!(rendered.contains("路人（actor-d）"), "全名册应完整");
    }
}
