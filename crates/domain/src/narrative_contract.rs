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
    /// 第二人称（「你」）
    Second,
}

/// 开场白确立的叙事风格。
///
/// 对话首条开场白（卡的 first_mes / 所选 greeting）决定两件事：
/// - 叙述人称：第一/第二人称开场的卡，正文沿用同一人称；
/// - 卡自带标记：开场白里出现的 HTML 标签（状态栏等）与代码围栏是卡的既定格式，
///   质量门不把它们当格式泄漏。`<think>` 永远不算卡的格式。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpeningStyle {
    /// 推断出的人称；None = 判断不出（沿用默认限知第三人称）
    pub pov: Option<NarrativePov>,
    /// 小写 HTML 标签名，代码围栏记为 "```"；去重排序
    pub card_markup: Vec<String>,
}

/// 人称判定的最低出现次数（叙述部分）
const OPENING_POV_MIN_HITS: usize = 3;

impl OpeningStyle {
    pub fn from_opening(text: &str) -> Self {
        Self {
            pov: infer_opening_pov(text),
            card_markup: collect_card_markup(text),
        }
    }
}

fn collect_card_markup(text: &str) -> Vec<String> {
    let mut tags = std::collections::BTreeSet::new();
    if text.contains("```") {
        tags.insert("```".to_string());
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        if j < bytes.len() && bytes[j] == b'/' {
            j += 1;
        }
        let start = j;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'-') {
            j += 1;
        }
        let terminated =
            j < bytes.len() && matches!(bytes[j], b'>' | b'/' | b' ' | b'\t' | b'\r' | b'\n');
        if j > start && bytes[start].is_ascii_alphabetic() && terminated {
            let name = text[start..j].to_ascii_lowercase();
            if name != "think" {
                tags.insert(name);
            }
        }
        i = j.max(i + 1);
    }
    tags.into_iter().collect()
}

/// 只留叙述：去掉 HTML 注释与标签、引号内对白（按行复位，防止不成对的引号吞掉全文）。
fn opening_narration(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);

    let mut narration = String::with_capacity(out.len());
    let mut in_tag = false;
    let mut depth = 0i32;
    let mut ascii_quote = false;
    for ch in out.chars() {
        if in_tag {
            in_tag = ch != '>';
            continue;
        }
        match ch {
            '<' => in_tag = true,
            '\n' => {
                depth = 0;
                ascii_quote = false;
                narration.push('\n');
            }
            '「' | '『' | '“' => depth += 1,
            '」' | '』' | '”' => depth = (depth - 1).max(0),
            '"' => ascii_quote = !ascii_quote,
            _ if depth == 0 && !ascii_quote => narration.push(ch),
            _ => {}
        }
    }
    narration
}

fn infer_opening_pov(text: &str) -> Option<NarrativePov> {
    let narration = opening_narration(text);
    let mut first = narration.matches('我').count();
    let mut second = narration.matches('你').count();
    let mut third = narration.matches('他').count() + narration.matches('她').count();
    for word in narration
        .split(|c: char| !c.is_ascii_alphabetic() && c != '\'')
        .filter(|w| !w.is_empty())
    {
        match word.to_ascii_lowercase().as_str() {
            "i" | "me" | "my" | "myself" | "i'm" | "i'd" | "i'll" | "i've" => first += 1,
            "you" | "your" | "yourself" | "you're" => second += 1,
            "he" | "she" | "him" | "her" | "his" | "himself" | "herself" => third += 1,
            _ => {}
        }
    }
    // 第一/第二人称叙述里也会频繁出现「他/她」（指别人），所以只要求本人称
    // 不被三人称压倒（≥ 1/3）；限知第三人称里偶发的无引号心声达不到这个比例。
    let dominant = |hits: usize, rival: usize| {
        hits >= OPENING_POV_MIN_HITS && hits >= rival && hits * 3 >= third
    };
    if dominant(first, second) {
        Some(NarrativePov::First)
    } else if dominant(second, first) {
        Some(NarrativePov::Second)
    } else {
        None
    }
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
    /// 显示名（attribution 扫描用；可空）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    /// 稳定探针或短摘要
    pub secret: String,
}

impl PrivateBinding {
    /// 用于归属扫描的标签：owner_id + 可选 name
    pub fn owner_labels(&self) -> Vec<&str> {
        let mut v = vec![self.owner_id.as_str()];
        if let Some(n) = self
            .owner_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            v.push(n);
        }
        v
    }
}

/// 从私密知识文本中提取可用于门禁扫描的稳定探针。
///
/// 优先 `SF_SECRET_*` 形 token；否则短文本（≤48 字）整段作为探针；
/// 过长自然语言不自动生成硬探针（避免误拦）。
pub fn extract_gate_probes(secret_text: &str) -> Vec<String> {
    let t = secret_text.trim();
    if t.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    let bytes = t.as_bytes();
    let needle = b"SF_SECRET_";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let start = i;
            i += needle.len();
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                    i += 1;
                } else {
                    break;
                }
            }
            let token = &t[start..i];
            if token.chars().count() >= 8 {
                out.push(token.to_string());
            }
        } else {
            i += 1;
        }
    }
    if !out.is_empty() {
        out.sort();
        out.dedup();
        return out;
    }
    if t.chars().count() <= 48 {
        out.push(t.to_string());
    }
    out
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
    /// 正文不得错误泄露的探针（稳定 token 清单；Error 决策优先 attribution）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_not_reveal: Vec<String>,
    /// 本场不得一次性解决的问题（来自 ScenePlan）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub must_not_resolve: Option<String>,
    #[serde(default)]
    pub style_constraints: StyleConstraints,
    /// pov 由开场白推断（渲染时注明沿用开场白）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pov_from_opening: bool,
    /// 卡开场白确立的格式标记（见 [`OpeningStyle::card_markup`]）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub card_markup: Vec<String>,
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
            pov_from_opening: false,
            card_markup: vec![],
        }
    }
}

impl NarrativeContract {
    /// 套用开场白风格：推断出人称时覆盖默认视角，并登记卡自带的格式标记。
    pub fn with_opening_style(mut self, style: &OpeningStyle) -> Self {
        if let Some(pov) = style.pov {
            self.pov = pov;
            self.pov_from_opening = true;
        }
        self.card_markup = style.card_markup.clone();
        self
    }

    /// 卡开场白是否用过该标记（小写标签名，或 "```"）
    pub fn allows_card_markup(&self, key: &str) -> bool {
        self.card_markup.iter().any(|m| m == key)
    }

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
            // 仅收集本场焦点角色的私密归属。
            // 稳定探针进 must_not_reveal 清单；Error 决策在 gate 走 attribution-aware。
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
                            owner_name: Some(inst.name.clone()),
                            secret: secret.to_string(),
                        });
                    }
                }
            }
        }

        // 生产探针：从 private_bindings 提取稳定 token（SF_SECRET_* 或短文本）。
        let mut must_not_reveal = Vec::new();
        for b in &private_bindings {
            for p in extract_gate_probes(&b.secret) {
                if !must_not_reveal.contains(&p) {
                    must_not_reveal.push(p);
                }
            }
        }

        Self {
            pov: NarrativePov::LimitedThird,
            focalizers,
            public_facts,
            private_bindings,
            must_not_reveal,
            must_not_resolve,
            style_constraints: StyleConstraints::default(),
            pov_from_opening: false,
            card_markup: vec![],
        }
    }

    /// 某条 secret 是否属于指定 owner
    ///
    /// §5.2-1：空串必须直接返回 None——`contains("")` 恒真，缺这个守卫会
    /// 把"没有 secret"误判成"属于第一个 binding"。今天调用方有长度前置守卫，
    /// 这里补上以免未来新调用方踩坑。
    pub fn owner_of_secret(&self, secret: &str) -> Option<&str> {
        let secret = secret.trim();
        if secret.is_empty() {
            return None;
        }
        self.private_bindings
            .iter()
            .find(|b| b.secret == secret || secret.contains(&b.secret) || b.secret.contains(secret))
            .map(|b| b.owner_id.as_str())
    }

    /// 查找绑定：优先 exact secret，再 probes / contains
    ///
    /// §5.2-1：空 probe 同样必须返回 None（`contains("")` 恒真）。
    pub fn binding_for_probe(&self, probe: &str) -> Option<&PrivateBinding> {
        let probe = probe.trim();
        if probe.is_empty() {
            return None;
        }
        self.private_bindings.iter().find(|b| {
            if b.secret == probe {
                return true;
            }
            extract_gate_probes(&b.secret).iter().any(|p| p == probe)
                || b.secret.contains(probe)
                || probe.contains(&b.secret)
        })
    }

    /// 渲染为 Editor/Director 短约束
    pub fn render_for_prompt(&self) -> String {
        let mut lines = vec!["【叙事契约 NarrativeContract】".to_string()];
        let pov = self.pov.label();
        if self.pov_from_opening {
            lines.push(format!(
                "- 视角：{pov}（沿用开场白的叙述人称，优先于其它人称设定）"
            ));
        } else {
            lines.push(format!("- 视角：{pov}"));
        }
        if let Some(markup) = self.card_markup_instruction() {
            lines.push(format!("- {markup}"));
        }
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
                "- 本场稳定私密探针：{} 条（非拥有者口中/叙述出现即质量 Error）",
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

    /// 卡开场白自带 HTML 标记（状态栏等）时给写作/修订的格式说明；没有则 None。
    pub fn card_markup_instruction(&self) -> Option<String> {
        if !self.card_markup.iter().any(|m| m != "```") {
            return None;
        }
        Some(
            "卡格式：开场白里的 HTML 块（状态栏等）是这张卡的既定格式；正文带有同样的块时保持其结构、更新其中内容，不要删除它、改成纯文本或另造别的 HTML。"
                .into(),
        )
    }
}

impl NarrativePov {
    pub fn label(self) -> &'static str {
        match self {
            NarrativePov::LimitedThird => "限知第三人称",
            NarrativePov::First => "第一人称",
            NarrativePov::Ensemble => "多焦点群像（仍禁止跨角色私密全知）",
            NarrativePov::Second => "第二人称",
        }
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
        // 生产探针：SF_SECRET_* 稳定 token 进入 must_not_reveal 清单
        assert!(
            c.must_not_reveal
                .iter()
                .any(|p| p == "SF_SECRET_LIN_VAULT_0427")
        );
        assert!(
            c.must_not_reveal
                .iter()
                .any(|p| p == "SF_SECRET_CHEN_BADGE_X91")
        );
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

    /// 第一人称开场 + 卡自带 HTML 状态栏（对话里的「我」不计入叙述）
    const FIRST_PERSON_OPENING: &str = "我推开教室门的时候，她已经坐在窗边了。\n\
        “你迟到了。”她头也没抬。\n\
        我把书包塞进桌肚，假装没听见。她的笔尖停了一下，又继续写。\n\
        我知道她在等我先开口，可我偏不。\n\n\
        <details><summary>📓 状态栏</summary>\n\
        <!-- 当前情境 -->\n\
        <div style=\"line-height:1.6\"><ul><li><b>时间:</b> 周一上午</li></ul></div>\n\
        </details>\n<StatusPlaceHolderImpl/>";

    #[test]
    fn opening_style_detects_first_person_and_card_markup() {
        let style = OpeningStyle::from_opening(FIRST_PERSON_OPENING);
        assert_eq!(style.pov, Some(NarrativePov::First));
        for tag in [
            "details",
            "summary",
            "div",
            "ul",
            "li",
            "b",
            "statusplaceholderimpl",
        ] {
            assert!(
                style.card_markup.iter().any(|m| m == tag),
                "missing {tag}: {:?}",
                style.card_markup
            );
        }
        assert!(!style.card_markup.iter().any(|m| m == "```"));
    }

    #[test]
    fn opening_style_third_person_narration_keeps_default() {
        // 对白里大量「我」「你」，叙述是第三人称：不得判成第一/第二人称
        let opening = "林秋把伞收好，抬头看了陈警官一眼。\n\
            “我不知道你在说什么，我那天根本没去过诊所，你问我也没用。”\n\
            他没有接话，只是把照片推到她面前。她盯着照片，手指慢慢收紧。\n\
            *我该怎么解释*，她想。";
        let style = OpeningStyle::from_opening(opening);
        assert_eq!(style.pov, None);
        assert!(style.card_markup.is_empty());
    }

    #[test]
    fn opening_style_detects_second_person() {
        let opening = "你醒来的时候，窗外在下雨。你摸到枕边的手机，屏幕亮着一条未读消息。\n\
            你犹豫了一下，还是点开了它。";
        assert_eq!(
            OpeningStyle::from_opening(opening).pov,
            Some(NarrativePov::Second)
        );
    }

    #[test]
    fn opening_style_english_first_person_and_code_fence() {
        let opening = "I pushed the door open. She was already there, and I knew she had \
            been waiting for me.\n```\nHP: 10\n```";
        let style = OpeningStyle::from_opening(opening);
        assert_eq!(style.pov, Some(NarrativePov::First));
        assert_eq!(style.card_markup, vec!["```".to_string()]);
    }

    #[test]
    fn opening_markup_never_includes_think_and_unclosed_quote_does_not_swallow_text() {
        // 不成对的引号按行复位，后面几行的第一人称叙述仍要计入
        let opening = "“半句没说完的话\n我站在门口。\n我听见楼下有人喊。\n我没动。<think>x</think>";
        let style = OpeningStyle::from_opening(opening);
        assert_eq!(style.pov, Some(NarrativePov::First));
        assert!(style.card_markup.is_empty(), "{:?}", style.card_markup);
    }

    #[test]
    fn with_opening_style_overrides_pov_and_renders_card_format() {
        let contract = NarrativeContract::default()
            .with_opening_style(&OpeningStyle::from_opening(FIRST_PERSON_OPENING));
        assert_eq!(contract.pov, NarrativePov::First);
        assert!(contract.pov_from_opening);
        assert!(contract.allows_card_markup("div"));
        assert!(!contract.allows_card_markup("span"));
        let rendered = contract.render_for_prompt();
        assert!(
            rendered.contains("视角：第一人称（沿用开场白"),
            "{rendered}"
        );
        assert!(rendered.contains("卡格式"), "{rendered}");

        // 开场白没有信号：契约保持旧行为，渲染不变
        let plain = NarrativeContract::default().with_opening_style(&OpeningStyle::default());
        assert_eq!(plain, NarrativeContract::default());
        assert!(!plain.render_for_prompt().contains("沿用开场白"));
        assert!(plain.card_markup_instruction().is_none());
    }

    #[test]
    fn contract_without_opening_fields_serializes_as_before() {
        let json = serde_json::to_value(NarrativeContract::default()).unwrap();
        assert!(json.get("pov_from_opening").is_none());
        assert!(json.get("card_markup").is_none());
        let old: NarrativeContract = serde_json::from_str(r#"{"pov":"limited_third"}"#).unwrap();
        assert_eq!(old, NarrativeContract::default());
    }

    #[test]
    fn extract_gate_probes_prefers_sf_secret_token() {
        let probes = extract_gate_probes("我藏了 SF_SECRET_LIN_VAULT_0427 在保险柜");
        assert_eq!(probes, vec!["SF_SECRET_LIN_VAULT_0427".to_string()]);
    }

    #[test]
    fn extract_gate_probes_short_text_whole() {
        let probes = extract_gate_probes("徽章号X91");
        assert_eq!(probes, vec!["徽章号X91".to_string()]);
    }

    #[test]
    fn extract_gate_probes_long_text_skipped() {
        let long = "这是一段很长的自然语言私密回忆，没有稳定 token，也不应该整段自动变成硬禁探针，否则会误伤大量正文描写。";
        assert!(extract_gate_probes(long).is_empty());
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

    #[test]
    fn empty_secret_or_probe_never_matches_a_binding() {
        // §5.2-1：`contains("")` 恒真，空输入必须 fail closed 返回 None
        let contract = NarrativeContract {
            private_bindings: vec![PrivateBinding {
                owner_id: "inst-lin".into(),
                owner_name: None,
                secret: "SF_SECRET_LIN_VAULT_0427".into(),
            }],
            ..Default::default()
        };

        assert_eq!(contract.owner_of_secret(""), None);
        assert_eq!(contract.owner_of_secret("   "), None);
        assert!(contract.binding_for_probe("").is_none());
        assert!(contract.binding_for_probe("  ").is_none());
        // 正常路径不受影响
        assert_eq!(
            contract.owner_of_secret("SF_SECRET_LIN_VAULT_0427"),
            Some("inst-lin")
        );
        assert!(
            contract
                .binding_for_probe("SF_SECRET_LIN_VAULT_0427")
                .is_some()
        );
    }
}
