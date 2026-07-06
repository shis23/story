//! 角色可见信息（character_knowledge）
//!
//! 对应设计 §16 / INTENT D30-D31, D39。
//!
//! 每个角色维护一张「可见信息列表」，记录它知道什么。
//! 来源四元分类（witnessed/told_by_other/inferred/backstory）+ pinned（是否进稳定前缀）。
//! 进现有 infra-vector 向量库，打 owner_character_id + campaign_id 标签。

use crate::Id;
use serde::{Deserialize, Serialize};

// ─── 知识来源分类（D31）─────────────────────────────────────────────────────

/// 角色获取信息的来源
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSource {
    /// 亲眼所见（在场发生的事）
    Witnessed,
    /// 被其他人告知（记来源角色 ID）
    ToldByOther,
    /// 自己推断的
    Inferred,
    /// 背景设定（游戏开始前就知道，导入时建立）
    Backstory,
}

/// 知识传播策略。
///
/// `Open` 保持既有行为；`Private` 表示该知识只给拥有者注入，并阻止后处理写回层继续传播；
/// `GroupRestricted` 预留给后续更细的身份组封口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PropagationPolicy {
    #[default]
    Open,
    Private,
    GroupRestricted(String),
}

impl PropagationPolicy {
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open)
    }
}

// ─── 知识条目 ──────────────────────────────────────────────────────────────

/// 一条角色可见信息（"这个角色知道什么"）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterKnowledgeEntry {
    pub id: Id,
    /// 哪个 Campaign（会话隔离，D36）
    pub campaign_id: Id,
    /// 谁知道这条信息
    pub character_id: Id,
    /// 信息摘要（用该角色的第一人称视角表述）
    pub knowledge_text: String,
    /// 来源分类
    pub source: KnowledgeSource,
    /// 如果是被告知（ToldByOther），记录谁告诉的
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_character_id: Option<Id>,
    /// 如果该知识来自另一条已持久化知识，记录上游知识条目，形成 A→B→C 的传话链。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_knowledge_id: Option<Id>,
    /// 第几轮知道的（backstory = 0）
    pub turn_number: u32,
    /// 关联的全局事件（可为空）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<Id>,
    /// 是否 pin 到子 Agent 的稳定前缀（D46 §22.5）
    ///
    /// true = 进 system 慢变层（backstory 知识 + 重大揭示）；
    /// false = 走向量检索按需召回，或仅进末尾注入。
    /// pin 总量有上限，超了挤掉最老的，防止 system 膨胀。
    #[serde(default)]
    pub pinned: bool,
    /// 传播策略。默认 open，兼容旧 knowledge.json。
    #[serde(default, skip_serializing_if = "PropagationPolicy::is_open")]
    pub propagation: PropagationPolicy,
}

impl CharacterKnowledgeEntry {
    /// 创建 backstory 知识（自动 pin，turn=0）
    pub fn backstory(campaign_id: Id, character_id: Id, text: impl Into<String>) -> Self {
        Self {
            id: Id::new(),
            campaign_id,
            character_id,
            knowledge_text: text.into(),
            source: KnowledgeSource::Backstory,
            source_character_id: None,
            source_knowledge_id: None,
            turn_number: 0,
            event_id: None,
            pinned: true, // backstory 默认 pin
            propagation: PropagationPolicy::Open,
        }
    }

    /// 创建 witnessed 知识
    pub fn witnessed(
        campaign_id: Id,
        character_id: Id,
        text: impl Into<String>,
        turn: u32,
    ) -> Self {
        Self {
            id: Id::new(),
            campaign_id,
            character_id,
            knowledge_text: text.into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            source_knowledge_id: None,
            turn_number: turn,
            event_id: None,
            pinned: false,
            propagation: PropagationPolicy::Open,
        }
    }

    /// 创建 told_by_other 知识（必须提供来源角色）
    pub fn told_by(
        campaign_id: Id,
        character_id: Id,
        text: impl Into<String>,
        told_by: Id,
        turn: u32,
    ) -> Self {
        Self {
            id: Id::new(),
            campaign_id,
            character_id,
            knowledge_text: text.into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(told_by),
            source_knowledge_id: None,
            turn_number: turn,
            event_id: None,
            pinned: false,
            propagation: PropagationPolicy::Open,
        }
    }

    /// 创建 inferred 知识
    pub fn inferred(campaign_id: Id, character_id: Id, text: impl Into<String>, turn: u32) -> Self {
        Self {
            id: Id::new(),
            campaign_id,
            character_id,
            knowledge_text: text.into(),
            source: KnowledgeSource::Inferred,
            source_character_id: None,
            source_knowledge_id: None,
            turn_number: turn,
            event_id: None,
            pinned: false,
            propagation: PropagationPolicy::Open,
        }
    }

    /// pin / unpin（标记为重大信息或取消）
    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    pub fn set_propagation(&mut self, propagation: PropagationPolicy) {
        self.propagation = propagation;
    }
}

// ─── 广播目标（方向 1：显式广播语义）────────────────────────────────────────

/// 广播目标类型
///
/// `None` = 单角色定向（默认）；`Some(All)` = Campaign 内所有 instance；
/// `Some(Group(g))` = 某身份组（通过 `CharacterDefinition.group` 匹配）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BroadcastTarget {
    /// 广播给 Campaign 内所有角色
    All,
    /// 广播给某身份组（匹配 CharacterDefinition.group）
    Group(String),
}

// ─── 后处理 Agent 产出的知识更新（D40）──────────────────────────────────────

/// 后处理 Agent 抽取的知识更新（尚未分配 id，写入时生成）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterKnowledgeUpdate {
    pub character_id: Id,
    pub knowledge_text: String,
    pub source: KnowledgeSource,
    /// ToldByOther 时填
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_character_id: Option<Id>,
    /// 是否标记为 pin（重大揭示）
    #[serde(default)]
    pub pinned: bool,
    /// 广播目标（方向 1）：None=单角色; Some(All)=全体; Some(Group(g))=身份组。
    /// `#[serde(default)]` 保证既有数据/测试不传此字段时反序列化为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<BroadcastTarget>,
    /// 传播策略。默认 open，只有成文明确提示"秘密/禁止外传"时由 postprocess 标 private。
    #[serde(default, skip_serializing_if = "PropagationPolicy::is_open")]
    pub propagation: PropagationPolicy,
}

impl CharacterKnowledgeUpdate {
    /// 转成持久化的 entry（分配 id + 补 campaign_id + turn）
    pub fn into_entry(self, campaign_id: Id, turn: u32) -> CharacterKnowledgeEntry {
        CharacterKnowledgeEntry {
            id: Id::new(),
            campaign_id,
            character_id: self.character_id,
            knowledge_text: self.knowledge_text,
            source: self.source,
            source_character_id: self.source_character_id,
            source_knowledge_id: None,
            turn_number: turn,
            event_id: None,
            pinned: self.pinned,
            propagation: self.propagation,
        }
    }
}

// ─── 注入子 Agent 上下文时的渲染（确定性查表，零 LLM）────────────────────

type KnowledgeNameResolver<'a> = dyn Fn(&Id) -> Option<String> + 'a;

/// 把某角色的知识渲染成文本，拼进子 Agent 上下文
///
/// `pinned_only = true` 时只渲染 pinned 知识（进 system 稳定层）；
/// `pinned_only = false` 时渲染非 pinned 的近期知识（进末尾 user）。
///
/// `name_resolver`：可选的 source_character_id → 名字解析闭包。
/// ToldByOther 渲染时带上告知者名字（"X 告诉我：……"）。
/// 传 `None` 时退化为旧行为（只显示"（被告知）"）。
pub fn render_knowledge_for_injection(
    entries: &[CharacterKnowledgeEntry],
    pinned_only: bool,
    max_count: usize,
    name_resolver: Option<&KnowledgeNameResolver<'_>>,
) -> String {
    let filtered: Vec<_> = entries
        .iter()
        .filter(|e| e.pinned == pinned_only)
        .take(max_count)
        .collect();

    if filtered.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let title = if pinned_only {
        "你始终记得的事"
    } else {
        "你近期知道的事"
    };
    out.push_str(&format!("【{title}】\n"));
    for e in &filtered {
        let source_tag = match &e.source {
            KnowledgeSource::Witnessed => "（亲眼所见）".to_string(),
            KnowledgeSource::ToldByOther => {
                // 方向 3：ToldByOther 渲染带告知者名字
                if let (Some(resolver), Some(source_id)) = (name_resolver, &e.source_character_id) {
                    if let Some(source_name) = resolver(source_id) {
                        format!("（被告知，来源：{source_name}）")
                    } else {
                        "（被告知）".to_string()
                    }
                } else {
                    "（被告知）".to_string()
                }
            }
            KnowledgeSource::Inferred => "（推断）".to_string(),
            KnowledgeSource::Backstory => "（背景）".to_string(),
        };
        let propagation_tag = match &e.propagation {
            PropagationPolicy::Open => String::new(),
            PropagationPolicy::Private => "（秘密，禁止外传）".to_string(),
            PropagationPolicy::GroupRestricted(group) => format!("（限制传播：仅{group}）"),
        };
        out.push_str(&format!(
            "- {}{source_tag}{propagation_tag}\n",
            e.knowledge_text
        ));
    }
    out
}

// ─── 测试 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backstory_entry_auto_pinned() {
        let entry = CharacterKnowledgeEntry::backstory(Id::new(), Id::new(), "我是外科医生");
        assert_eq!(entry.source, KnowledgeSource::Backstory);
        assert_eq!(entry.turn_number, 0);
        assert!(entry.pinned); // backstory 默认 pin
    }

    #[test]
    fn test_witnessed_not_pinned() {
        let entry = CharacterKnowledgeEntry::witnessed(Id::new(), Id::new(), "发生了爆炸", 5);
        assert_eq!(entry.source, KnowledgeSource::Witnessed);
        assert!(!entry.pinned);
        assert_eq!(entry.turn_number, 5);
    }

    #[test]
    fn test_told_by_records_source() {
        let teller = Id::new();
        let entry = CharacterKnowledgeEntry::told_by(
            Id::new(),
            Id::new(),
            "地下室有尸体",
            teller.clone(),
            7,
        );
        assert_eq!(entry.source, KnowledgeSource::ToldByOther);
        assert_eq!(entry.source_character_id, Some(teller));
    }

    #[test]
    fn test_render_pinned_only() {
        let campaign = Id::new();
        let char_id = Id::new();
        let entries = vec![
            CharacterKnowledgeEntry::backstory(campaign.clone(), char_id.clone(), "我是外科医生"),
            CharacterKnowledgeEntry::witnessed(campaign.clone(), char_id.clone(), "发生了爆炸", 3),
        ];
        let pinned_text = render_knowledge_for_injection(&entries, true, 10, None);
        let recent_text = render_knowledge_for_injection(&entries, false, 10, None);

        assert!(pinned_text.contains("我是外科医生"));
        assert!(pinned_text.contains("你始终记得的事"));
        assert!(!pinned_text.contains("发生了爆炸"));

        assert!(recent_text.contains("发生了爆炸"));
        assert!(recent_text.contains("你近期知道的事"));
        assert!(!recent_text.contains("我是外科医生"));
    }

    #[test]
    fn test_render_empty_returns_empty() {
        let entries: Vec<CharacterKnowledgeEntry> = vec![];
        let out = render_knowledge_for_injection(&entries, true, 10, None);
        assert!(out.is_empty());
    }

    #[test]
    fn test_update_into_entry() {
        let char_id = Id::new();
        let update = CharacterKnowledgeUpdate {
            character_id: char_id.clone(),
            knowledge_text: "得知真相".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(Id::new()),
            pinned: true, // 重大揭示，pin
            broadcast: None,
            propagation: PropagationPolicy::Open,
        };
        let entry = update.into_entry(Id::new(), 8);
        assert_eq!(entry.character_id, char_id);
        assert_eq!(entry.turn_number, 8);
        assert!(entry.pinned);
        assert!(entry.source_character_id.is_some());
        assert!(entry.source_knowledge_id.is_none());
    }

    #[test]
    fn test_set_pinned() {
        let mut entry = CharacterKnowledgeEntry::witnessed(Id::new(), Id::new(), "重大揭示", 3);
        assert!(!entry.pinned);
        entry.set_pinned(true); // 升级为重大信息
        assert!(entry.pinned);
    }

    // ─── W6 方向 3：render_knowledge_for_injection 带告知者名字 ─────────────

    #[test]
    fn test_render_told_by_other_with_source_name() {
        let campaign = Id::new();
        let char_id = Id::new();
        let source_id = Id::new();
        let entries = vec![CharacterKnowledgeEntry {
            id: Id::new(),
            campaign_id: campaign,
            character_id: char_id,
            knowledge_text: "地下室有尸体".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(source_id.clone()),
            source_knowledge_id: None,
            turn_number: 3,
            event_id: None,
            pinned: false,
            propagation: PropagationPolicy::Open,
        }];

        // 有 resolver 时，显示告知者名字
        let resolver = |id: &Id| -> Option<String> {
            if *id == source_id {
                Some("林医生".to_string())
            } else {
                None
            }
        };
        let text = render_knowledge_for_injection(&entries, false, 10, Some(&resolver));
        assert!(text.contains("（被告知，来源：林医生）"));
        assert!(text.contains("地下室有尸体"));
    }

    #[test]
    fn test_render_told_by_other_without_resolver() {
        let campaign = Id::new();
        let char_id = Id::new();
        let entries = vec![CharacterKnowledgeEntry {
            id: Id::new(),
            campaign_id: campaign,
            character_id: char_id,
            knowledge_text: "地下室有尸体".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(Id::new()),
            source_knowledge_id: None,
            turn_number: 3,
            event_id: None,
            pinned: false,
            propagation: PropagationPolicy::Open,
        }];

        // 无 resolver 时，退化为旧行为
        let text = render_knowledge_for_injection(&entries, false, 10, None);
        assert!(text.contains("（被告知）"));
        assert!(!text.contains("来源："));
    }

    #[test]
    fn test_render_private_policy_marks_secret() {
        let campaign = Id::new();
        let char_id = Id::new();
        let mut entry =
            CharacterKnowledgeEntry::witnessed(campaign, char_id, "保险柜密码是 0427", 4);
        entry.propagation = PropagationPolicy::Private;

        let text = render_knowledge_for_injection(&[entry], false, 10, None);
        assert!(text.contains("保险柜密码是 0427"));
        assert!(text.contains("禁止外传"));
    }

    // ─── W6 方向 1：BroadcastTarget serde 兼容 ─────────────────────────────

    #[test]
    fn test_broadcast_target_serde_roundtrip() {
        let all = BroadcastTarget::All;
        let json = serde_json::to_string(&all).unwrap();
        assert_eq!(json, "\"all\"");
        let back: BroadcastTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(back, BroadcastTarget::All);

        // Group 变体是 tagged enum：{"group": "守卫"}
        let group = BroadcastTarget::Group("守卫".to_string());
        let json = serde_json::to_string(&group).unwrap();
        assert!(json.contains("守卫"));
        assert!(json.contains("group"));
        let back: BroadcastTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(back, BroadcastTarget::Group("守卫".to_string()));
    }

    #[test]
    fn test_knowledge_update_serde_default_broadcast() {
        // 既有 JSON 不含 broadcast 字段 → 反序列化为 None（向后兼容）
        let json = r#"{
            "character_id": "char-1",
            "knowledge_text": "test",
            "source": "witnessed",
            "source_character_id": null,
            "pinned": false
        }"#;
        let update: CharacterKnowledgeUpdate = serde_json::from_str(json).unwrap();
        assert!(update.broadcast.is_none());
        assert_eq!(update.propagation, PropagationPolicy::Open);
    }

    #[test]
    fn test_knowledge_update_serde_with_broadcast() {
        // 含 broadcast 字段 → 正确反序列化（枚举 tagged 格式）
        let json = r#"{
            "character_id": "char-1",
            "knowledge_text": "公告内容",
            "source": "witnessed",
            "source_character_id": null,
            "pinned": false,
            "broadcast": "all"
        }"#;
        let update: CharacterKnowledgeUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(update.broadcast, Some(BroadcastTarget::All));

        // Group 变体用 tagged 格式：{"group": "守卫"}
        let json_group = r#"{
            "character_id": "char-1",
            "knowledge_text": "组内公告",
            "source": "witnessed",
            "source_character_id": null,
            "pinned": false,
            "broadcast": {"group": "守卫"}
        }"#;
        let update: CharacterKnowledgeUpdate = serde_json::from_str(json_group).unwrap();
        assert_eq!(
            update.broadcast,
            Some(BroadcastTarget::Group("守卫".to_string()))
        );
    }

    #[test]
    fn test_knowledge_update_serde_with_private_propagation() {
        let json = r#"{
            "character_id": "char-1",
            "knowledge_text": "保险柜密码是 0427",
            "source": "witnessed",
            "pinned": false,
            "propagation": "private"
        }"#;
        let update: CharacterKnowledgeUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(update.propagation, PropagationPolicy::Private);

        let entry = update.into_entry(Id::from_str("camp-1"), 3);
        assert_eq!(entry.propagation, PropagationPolicy::Private);
    }
}
