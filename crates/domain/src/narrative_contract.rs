//! NarrativeContract（阶段 B）
//!
//! 每轮/每场戏必须遵守的叙事契约：视角、限知焦点、公开事实与私密绑定。
//! 吸收梁元「反全知」思想为结构化字段，不把整份预设塞进 system。

use crate::agent::{Plan, ScenePlan};
use crate::campaign_runtime::CampaignRuntimeContext;
use crate::character_knowledge::PropagationPolicy;
use serde::{Deserialize, Serialize};

/// 叙事视角
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NarrativePov {
    /// 限知第三人称（默认）
    #[default]
    LimitedThird,
    /// 第一人称
    First,
    /// 多焦点/群像（仍禁止跨角色私密全知）
    Ensemble,
}

/// 文风侧短开关（由 DraftQualityGate 执行）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleConstraints {
    /// 禁止正文破折号（—— / —）
    #[serde(default = "default_true")]
    pub ban_em_dash: bool,
    /// 禁止「不是……而是……」否后肯结构
    #[serde(default = "default_true")]
    pub ban_negation_affirmation: bool,
}

fn default_true() -> bool {
    true
}

impl Default for StyleConstraints {
    fn default() -> Self {
        Self {
            ban_em_dash: true,
            ban_negation_affirmation: true,
        }
    }
}

/// 一条私密绑定：某角色拥有、他人不得当众知晓的内容探针/摘要
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateBinding {
    /// 拥有者 instance_id / character_id
    pub owner_id: String,
    /// 稳定探针或短摘要
    pub secret: String,
}

/// 叙事契约
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NarrativeContract {
    pub pov: NarrativePov,
    /// 本场允许的焦点角色 id
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub focalizers: Vec<String>,
    /// 可公开叙述的事实
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_facts: Vec<String>,
    /// 私密绑定
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub private_bindings: Vec<PrivateBinding>,
    /// 正文不得错误泄露的探针
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_not_reveal: Vec<String>,
    /// 本场不得一次性解决的问题（来自 ScenePlan）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub must_not_resolve: Option<String>,
    #[serde(default)]
    pub style_constraints: StyleConstraints,
}

impl Default for NarrativeContract {
    fn default() -> Self {
        Self {
            pov: NarrativePov::LimitedThird,
            focalizers: vec![],
            public_facts: vec![],
            private_bindings: vec![],
            must_not_reveal: vec![],
            must_not_resolve: None,
            style_constraints: StyleConstraints::default(),
        }
    }
}

impl NarrativeContract {
    /// 从 Plan + 可选 CampaignRuntime 构造契约。
    pub fn from_plan_and_runtime(plan: &Plan, runtime: Option<&CampaignRuntimeContext>) -> Self {
        let focalizers: Vec<String> = plan
            .subagent_tasks
            .iter()
            .map(|t| t.character_id.clone())
            .filter(|s| !s.is_empty())
            .collect();

        let scene: Option<&ScenePlan> = plan.scene_plan.as_ref();
        let must_not_resolve = scene.and_then(|s| s.must_not_resolve.clone());
        let mut public_facts = Vec::new();
        if let Some(s) = scene {
            if let Some(c) = s.conflict.as_ref().filter(|x| !x.trim().is_empty()) {
                public_facts.push(format!("冲突：{c}"));
            }
            if let Some(st) = s.stakes.as_ref().filter(|x| !x.trim().is_empty()) {
                public_facts.push(format!("赌注：{st}"));
            }
        }

        let mut private_bindings = Vec::new();
        if let Some(cr) = runtime {
            // 仅收集本场焦点角色的私密归属；不把全文 secret 自动塞进 must_not_reveal。
            // must_not_reveal 留给显式探针（如测试/导演硬约束），避免拥有者合法回忆也被 Error 拦截。
            for inst in &cr.instances {
                let id = inst.id.as_str();
                if !focalizers.is_empty() && !focalizers.iter().any(|f| f == id || f == &inst.name)
                {
                    continue;
                }
                for entry in cr.knowledge_for_instance(inst) {
                    if matches!(entry.propagation, PropagationPolicy::Private) {
                        let secret = entry.knowledge_text.trim();
                        if secret.is_empty() {
                            continue;
                        }
                        private_bindings.push(PrivateBinding {
                            owner_id: id.to_string(),
                            secret: secret.to_string(),
                        });
                    }
                }
            }
        }

        Self {
            pov: NarrativePov::LimitedThird,
            focalizers,
            public_facts,
            private_bindings,
            // 不从 private_bindings 自动复制：无说话者归因时，硬拦会误伤拥有者合法使用。
            must_not_reveal: vec![],
            must_not_resolve,
            style_constraints: StyleConstraints::default(),
        }
    }

    /// 某条 secret 是否属于指定 owner
    pub fn owner_of_secret(&self, secret: &str) -> Option<&str> {
        let secret = secret.trim();
        self.private_bindings
            .iter()
            .find(|b| b.secret == secret || secret.contains(&b.secret) || b.secret.contains(secret))
            .map(|b| b.owner_id.as_str())
    }

    /// 渲染为 Editor/Director 短约束
    pub fn render_for_prompt(&self) -> String {
        let mut lines = vec!["【叙事契约 NarrativeContract】".to_string()];
        let pov = match self.pov {
            NarrativePov::LimitedThird => "限知第三人称",
            NarrativePov::First => "第一人称",
            NarrativePov::Ensemble => "多焦点群像（仍禁止跨角色私密全知）",
        };
        lines.push(format!("- 视角：{pov}"));
        if !self.focalizers.is_empty() {
            lines.push(format!("- 焦点角色：{}", self.focalizers.join("、")));
        }
        if !self.public_facts.is_empty() {
            lines.push(format!("- 可公开事实：{}", self.public_facts.join("；")));
        }
        if !self.private_bindings.is_empty() {
            // 不把完整自然语言秘密塞进 Editor/Director prompt；只给归属计数与 owner id。
            lines.push(format!(
                "- 私密知识：{} 条（仅拥有者可用；叙述不得替他人全知；勿在正文替非拥有者泄露）",
                self.private_bindings.len()
            ));
            let owners: Vec<&str> = self
                .private_bindings
                .iter()
                .map(|b| b.owner_id.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            if !owners.is_empty() {
                lines.push(format!("- 私密归属角色：{}", owners.join("、")));
            }
        }
        if !self.must_not_reveal.is_empty() {
            lines.push(format!(
                "- 本场硬禁探针：{} 条（正文出现即质量 Error）",
                self.must_not_reveal.len()
            ));
        }
        if let Some(m) = self
            .must_not_resolve
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            lines.push(format!("- 本场不得一次性解决：{m}"));
        }
        lines
            .push("- 硬规则：角色对话/心理只能使用其已知信息；禁止作者视角剧透未公开秘密。".into());
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Id;
    use crate::agent::{ContextPackage, SubagentTask};
    use crate::campaign::{Campaign, CharacterInstance};
    use crate::character_knowledge::{CharacterKnowledgeEntry, PropagationPolicy};
    use std::collections::HashMap;

    fn empty_pkg(scene: &str) -> ContextPackage {
        ContextPackage {
            character_brief: String::new(),
            scene_brief: scene.into(),
            relevant_lore: vec![],
            constant_lore: vec![],
            recent_window: vec![],
            task: String::new(),
        }
    }

    #[test]
    fn from_plan_without_runtime_uses_focalizers_and_scene_plan() {
        let plan = Plan {
            scene_brief: "雨夜诊所".into(),
            subagent_tasks: vec![SubagentTask::new("inst-lin", "沉默", empty_pkg("雨夜诊所"))],
            scene_plan: Some(ScenePlan {
                conflict: Some("是否透露真相".into()),
                must_not_resolve: Some("主线谜底".into()),
                stakes: Some("信任破裂".into()),
                ..Default::default()
            }),
        };
        let c = NarrativeContract::from_plan_and_runtime(&plan, None);
        assert_eq!(c.focalizers, vec!["inst-lin".to_string()]);
        assert_eq!(c.must_not_resolve.as_deref(), Some("主线谜底"));
        assert!(c.private_bindings.is_empty());
        assert!(c.render_for_prompt().contains("限知第三人称"));
        assert!(c.render_for_prompt().contains("主线谜底"));
    }

    #[test]
    fn from_plan_with_runtime_collects_private_bindings() {
        let lin = Id::from_str("inst-lin");
        let chen = Id::from_str("inst-chen");
        let campaign = Campaign::new(Id::from_str("card-1"), "test");
        let mut k_lin = CharacterKnowledgeEntry::witnessed(
            campaign.id.clone(),
            lin.clone(),
            "SF_SECRET_LIN_VAULT_0427",
            1,
        );
        k_lin.set_propagation(PropagationPolicy::Private);
        let mut k_chen = CharacterKnowledgeEntry::witnessed(
            campaign.id.clone(),
            chen.clone(),
            "SF_SECRET_CHEN_BADGE_X91",
            1,
        );
        k_chen.set_propagation(PropagationPolicy::Private);

        let runtime = CampaignRuntimeContext {
            campaign: campaign.clone(),
            instances: vec![
                CharacterInstance {
                    id: lin,
                    campaign_id: campaign.id.clone(),
                    definition_id: None,
                    name: "林秋".into(),
                    persona_override: None,
                    behavior_override: None,
                    variables: vec![],
                    is_temporary: false,
                },
                CharacterInstance {
                    id: chen,
                    campaign_id: campaign.id.clone(),
                    definition_id: None,
                    name: "陈警官".into(),
                    persona_override: None,
                    behavior_override: None,
                    variables: vec![],
                    is_temporary: false,
                },
            ],
            definitions_by_id: HashMap::new(),
            knowledge: vec![k_lin, k_chen],
            tasks: vec![],
            turn: 1,
        };

        let plan = Plan {
            scene_brief: "急诊室".into(),
            subagent_tasks: vec![
                SubagentTask::new("inst-lin", "诊治", empty_pkg("急诊室")),
                SubagentTask::new("inst-chen", "问话", empty_pkg("急诊室")),
            ],
            scene_plan: None,
        };
        let c = NarrativeContract::from_plan_and_runtime(&plan, Some(&runtime));
        assert_eq!(c.private_bindings.len(), 2);
        // private_bindings 不自动进入 must_not_reveal（避免拥有者合法使用被 Error）
        assert!(c.must_not_reveal.is_empty());
        assert_eq!(
            c.owner_of_secret("SF_SECRET_LIN_VAULT_0427"),
            Some("inst-lin")
        );
        let rendered = c.render_for_prompt();
        assert!(
            !rendered.contains("SF_SECRET_LIN_VAULT_0427"),
            "prompt must not dump full secret text: {rendered}"
        );
        assert!(rendered.contains("私密归属角色") || rendered.contains("私密知识"));
    }

    #[test]
    fn old_plan_json_deserializes_without_scene_plan() {
        let json = r#"{
            "scene_brief": "旧场景",
            "subagent_tasks": [{
                "character_id": "A",
                "brief": "演",
                "context_package": {
                    "character_brief": "",
                    "scene_brief": "旧场景",
                    "relevant_lore": [],
                    "constant_lore": [],
                    "recent_window": [],
                    "task": "演"
                }
            }]
        }"#;
        let plan: Plan = serde_json::from_str(json).expect("old plan should deserialize");
        assert_eq!(plan.scene_brief, "旧场景");
        assert!(plan.scene_plan.is_none());
        assert!(plan.subagent_tasks[0].current_desire.is_none());
        assert!(plan.subagent_tasks[0].emotion_stage.is_none());
    }
}
