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
            turn_number: 0,
            event_id: None,
            pinned: true, // backstory 默认 pin
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
            turn_number: turn,
            event_id: None,
            pinned: false,
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
            turn_number: turn,
            event_id: None,
            pinned: false,
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
            turn_number: turn,
            event_id: None,
            pinned: false,
        }
    }

    /// pin / unpin（标记为重大信息或取消）
    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }
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
            turn_number: turn,
            event_id: None,
            pinned: self.pinned,
        }
    }
}

// ─── 注入子 Agent 上下文时的渲染（确定性查表，零 LLM）────────────────────

/// 把某角色的知识渲染成文本，拼进子 Agent 上下文
///
/// `pinned_only = true` 时只渲染 pinned 知识（进 system 稳定层）；
/// `pinned_only = false` 时渲染非 pinned 的近期知识（进末尾 user）。
pub fn render_knowledge_for_injection(
    entries: &[CharacterKnowledgeEntry],
    pinned_only: bool,
    max_count: usize,
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
        let source_tag = match e.source {
            KnowledgeSource::Witnessed => "（亲眼所见）",
            KnowledgeSource::ToldByOther => "（被告知）",
            KnowledgeSource::Inferred => "（推断）",
            KnowledgeSource::Backstory => "（背景）",
        };
        out.push_str(&format!("- {}{source_tag}\n", e.knowledge_text));
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
        let pinned_text = render_knowledge_for_injection(&entries, true, 10);
        let recent_text = render_knowledge_for_injection(&entries, false, 10);

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
        let out = render_knowledge_for_injection(&entries, true, 10);
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
        };
        let entry = update.into_entry(Id::new(), 8);
        assert_eq!(entry.character_id, char_id);
        assert_eq!(entry.turn_number, 8);
        assert!(entry.pinned);
        assert!(entry.source_character_id.is_some());
    }

    #[test]
    fn test_set_pinned() {
        let mut entry = CharacterKnowledgeEntry::witnessed(Id::new(), Id::new(), "重大揭示", 3);
        assert!(!entry.pinned);
        entry.set_pinned(true); // 升级为重大信息
        assert!(entry.pinned);
    }
}
