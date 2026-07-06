use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::agent::{Performance, Plan};
use crate::llm::{ChatMessage, ChatRole};

/// 反序列化时将 active_variant 钳制到有效范围，避免越界。
/// serde 不支持跨字段校验，故手动 impl Deserialize。
impl<'de> Deserialize<'de> for MessageNode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            id: Id,
            parent_id: Option<Id>,
            variants: Vec<MessageVariant>,
            active_variant: usize,
        }
        let raw = Raw::deserialize(deserializer)?;
        let clamped = if raw.variants.is_empty() {
            0
        } else {
            raw.active_variant.min(raw.variants.len() - 1)
        };
        if clamped != raw.active_variant {
            eprintln!(
                "WARN: MessageNode {} active_variant {} out of bounds (variants len={}), clamped to {}",
                raw.id,
                raw.active_variant,
                raw.variants.len(),
                clamped
            );
        }
        Ok(MessageNode {
            id: raw.id,
            parent_id: raw.parent_id,
            variants: raw.variants,
            active_variant: clamped,
        })
    }
}

// ─── 对话树（对应设计 §3.7 MessageNode 树结构）────────────────────────────

/// 角色
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
}

/// 消息变体状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VariantStatus {
    /// 编辑中/未定稿
    Draft,
    /// 已采纳（才会触发记忆归档，M2）
    Final,
    /// 被重 roll 或手动丢弃（软删除，可恢复）
    Discarded,
}

/// 单个变体（一个版本的消息内容，对应设计 §3.7.1 MessageVariant）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageVariant {
    pub id: Id,
    pub role: Role,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub status: VariantStatus,
    /// 溯源信息（若是 Agent 产出，保留上下文用于部分重 roll）
    pub provenance: Option<Provenance>,
}

/// 溯源信息（对应设计 §3.7.1 Provenance）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// 来自哪次写作流水线会话
    pub session_id: Id,
    /// 当时的 Plan 快照
    pub plan: Option<Plan>,
    /// 当时各子 Agent 产出快照
    pub subagent_results: Vec<SubagentSnapshot>,
    /// 当时用的提示词预设 ID
    pub profile_id: Option<Id>,
    /// 随机种子（重 roll 时可换）
    pub seed: u64,
    /// 上次重 roll 时附加的 hint（若有），便于二次重 roll 时 LLM 看到迭代历史
    #[serde(default)]
    pub last_hint: Option<String>,
}

/// 子 Agent 产出快照（存进 Provenance，用于部分重 roll）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSnapshot {
    pub character_id: String,
    pub full_text: String,
    /// Campaign 模式下绑定的 instance id（阶段 5 新增）。None = 旧路径或未匹配。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_instance_id: Option<String>,
    /// 显示名（阶段 5 新增）。Campaign 模式下为 instance.name，旧路径为 character_id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// fallback 原因（阶段 5 新增）。如 "instance not found, fell back to context_package"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
}

impl From<&Performance> for SubagentSnapshot {
    fn from(p: &Performance) -> Self {
        Self {
            character_id: p.character_id.clone(),
            full_text: p.full_text.clone(),
            character_instance_id: None,
            display_name: None,
            fallback_reason: None,
        }
    }
}

/// 消息节点（一个位置可有多个版本，对应设计 §3.7.1 MessageNode）
/// Deserialize 为手动实现，反序列化时钳制 active_variant 到有效范围。
#[derive(Debug, Clone, Serialize)]
pub struct MessageNode {
    pub id: Id,
    /// 父消息（首条为 None）
    pub parent_id: Option<Id>,
    /// 同一位置的多个版本（分支/swipe）
    pub variants: Vec<MessageVariant>,
    /// 当前选中的版本索引
    pub active_variant: usize,
}

impl MessageNode {
    /// 获取当前激活的变体
    pub fn active(&self) -> Option<&MessageVariant> {
        self.variants.get(self.active_variant)
    }

    /// 获取当前激活的变体（可变）
    pub fn active_mut(&mut self) -> Option<&mut MessageVariant> {
        self.variants.get_mut(self.active_variant)
    }

    /// 获取当前激活变体的内容（空字符串兜底）
    pub fn active_content(&self) -> &str {
        self.active().map(|v| v.content.as_str()).unwrap_or("")
    }

    /// 切换版本
    pub fn switch_variant(&mut self, index: usize) -> Result<(), String> {
        if index >= self.variants.len() {
            return Err(format!(
                "变体索引 {} 越界（共 {} 个变体）",
                index,
                self.variants.len()
            ));
        }
        self.active_variant = index;
        Ok(())
    }

    /// 添加新变体（swipe/分支）
    pub fn add_variant(&mut self, variant: MessageVariant) {
        self.variants.push(variant);
        self.active_variant = self.variants.len() - 1;
    }

    /// 软删除当前变体（标记为 Discarded）
    ///
    /// 删除后自动切换到最近的非 Discarded 变体。
    /// 如果所有变体都被 Discarded，仍然返回 Ok（删除本身已成功）。
    pub fn soft_delete_active(&mut self) -> Result<(), String> {
        {
            let v = self.active_mut().ok_or("当前节点无变体")?;
            v.status = VariantStatus::Discarded;
        }
        // 尝试切换到最近的非 Discarded 变体，找不到也没关系
        let _ = self.switch_to_nearest_active();
        Ok(())
    }

    /// 切换到最近的非 Discarded 变体（从当前索引向两侧搜索）
    fn switch_to_nearest_active(&mut self) -> Result<(), String> {
        let len = self.variants.len();
        if len == 0 {
            return Err("节点无变体".into());
        }
        let current = self.active_variant.min(len - 1);
        // 按距离从近到远搜索：先当前，再 +1/-1，+2/-2 ...
        for offset in 0..len {
            let candidates = if offset == 0 {
                vec![current]
            } else {
                let mut c = Vec::with_capacity(2);
                if current + offset < len {
                    c.push(current + offset);
                }
                if offset <= current {
                    c.push(current - offset);
                }
                c
            };
            for &idx in &candidates {
                if self.variants[idx].status != VariantStatus::Discarded {
                    self.active_variant = idx;
                    return Ok(());
                }
            }
        }
        Err("所有变体已被丢弃".into())
    }

    /// 编辑当前变体内容
    pub fn edit_active(&mut self, new_content: String) -> Result<(), String> {
        let v = self.active_mut().ok_or("当前节点无变体")?;
        v.content = new_content;
        Ok(())
    }
}

// ─── 对话（一棵对话树）────────────────────────────────────────────────────

/// 对话（对应设计 §3.7.1 Conversation）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Id,
    /// 关联的角色卡 ID
    pub character_id: Option<String>,
    /// 关联的 Campaign ID（一 Campaign 一对话模型：每个对话归属一个 Campaign）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<Id>,
    /// 消息列表（按时间顺序，但可通过 parent_id 支持分支）
    pub nodes: Vec<MessageNode>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    pub fn new(character_id: Option<String>, campaign_id: Option<Id>) -> Self {
        let now = Utc::now();
        Self {
            id: Id::new(),
            character_id,
            campaign_id,
            nodes: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// 追加一条消息（自动链接 parent）
    pub fn append_message(&mut self, role: Role, content: String) -> Id {
        let parent_id = self.nodes.last().map(|n| n.id.clone());
        let node_id = Id::new();
        let variant = MessageVariant {
            id: Id::new(),
            role,
            content,
            created_at: Utc::now(),
            status: VariantStatus::Final,
            provenance: None,
        };
        let node = MessageNode {
            id: node_id.clone(),
            parent_id,
            variants: vec![variant],
            active_variant: 0,
        };
        self.nodes.push(node);
        self.updated_at = Utc::now();
        node_id
    }

    /// 追加一条 AI 消息（Draft 状态，等待用户采纳）
    pub fn append_ai_draft(&mut self, content: String, provenance: Option<Provenance>) -> Id {
        let parent_id = self.nodes.last().map(|n| n.id.clone());
        let node_id = Id::new();
        let variant = MessageVariant {
            id: Id::new(),
            role: Role::Assistant,
            content,
            created_at: Utc::now(),
            status: VariantStatus::Draft,
            provenance,
        };
        let node = MessageNode {
            id: node_id.clone(),
            parent_id,
            variants: vec![variant],
            active_variant: 0,
        };
        self.nodes.push(node);
        self.updated_at = Utc::now();
        node_id
    }

    /// 获取最近 N 条消息的文本（用于上下文窗口）
    pub fn recent_messages(&self, n: usize) -> Vec<String> {
        self.nodes
            .iter()
            .rev()
            .take(n)
            .rev()
            .filter_map(|node| {
                let v = node.active()?;
                if v.status == VariantStatus::Discarded || v.content.is_empty() {
                    None
                } else {
                    Some(v.content.clone())
                }
            })
            .collect()
    }

    /// 获取最近 N 条消息（带角色标签，用于注入 Agent 上下文）
    /// 格式："用户: {content}" 或 "AI: {content}"
    /// `before_node_id`：如果指定，只返回该节点之前的消息（不含该节点及其后的）
    pub fn recent_messages_with_role(&self, n: usize, before_node_id: Option<&Id>) -> Vec<String> {
        self.iter_recent_active(n, before_node_id)
            .map(|v| {
                let role_label = match v.role {
                    Role::User => "用户",
                    Role::Assistant => "AI",
                };
                format!("{}: {}", role_label, v.content)
            })
            .collect()
    }

    /// 获取最近 N 条消息作为真正的 ChatMessage 列表（§22 cache 友好布局用）
    ///
    /// 与 `recent_messages_with_role` 的区别：返回 `Vec<ChatMessage>` 而非 `Vec<String>`，
    /// 每条消息的 role 映射为 `ChatRole::User/Assistant`，content 不加「用户:」前缀。
    /// 这样历史能作为独立消息段进 LLM（而非塞进 user tail 文本），保证 system+history 前缀稳定，cache 命中。
    ///
    /// `before_node_id`：如果指定，只返回该节点之前的消息（regenerate 重 roll 时排除目标节点及之后）
    pub fn recent_messages_as_chat(
        &self,
        n: usize,
        before_node_id: Option<&Id>,
    ) -> Vec<ChatMessage> {
        self.iter_recent_active(n, before_node_id)
            .map(|v| {
                let role = match v.role {
                    Role::User => ChatRole::User,
                    Role::Assistant => ChatRole::Assistant,
                };
                ChatMessage {
                    role,
                    content: v.content.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                }
            })
            .collect()
    }

    /// 共享迭代器：返回最近 N 条「活跃且非空非 Discarded」的变体（按时间正序）
    ///
    /// `recent_messages_with_role` 和 `recent_messages_as_chat` 都复用此逻辑，
    /// 保证过滤规则（Discarded/空跳过）和截断规则（before_node_id）一致。
    fn iter_recent_active(
        &self,
        n: usize,
        before_node_id: Option<&Id>,
    ) -> impl Iterator<Item = &MessageVariant> {
        // 确定截止位置：before_node_id 指定时只取该节点之前（不含）
        let end_idx = if let Some(bid) = before_node_id {
            self.nodes
                .iter()
                .position(|node| &node.id == bid)
                .unwrap_or(self.nodes.len())
        } else {
            self.nodes.len()
        };
        // rev → take(n) → rev：取最近 N 条但保持正序
        self.nodes[..end_idx]
            .iter()
            .rev()
            .take(n)
            .rev()
            .filter_map(|node| node.active())
            .filter(|v| v.status != VariantStatus::Discarded && !v.content.is_empty())
    }

    /// 查找节点
    pub fn find_node(&self, id: &Id) -> Option<&MessageNode> {
        self.nodes.iter().find(|n| &n.id == id)
    }

    /// 查找节点（可变）
    pub fn find_node_mut(&mut self, id: &Id) -> Option<&mut MessageNode> {
        self.nodes.iter_mut().find(|n| &n.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(role: Role, content: &str, status: VariantStatus) -> MessageVariant {
        MessageVariant {
            id: Id::new(),
            role,
            content: content.into(),
            created_at: Utc::now(),
            status,
            provenance: None,
        }
    }

    fn node(id: &str, v: MessageVariant) -> MessageNode {
        MessageNode {
            id: Id::from_str(id),
            parent_id: None,
            variants: vec![v],
            active_variant: 0,
        }
    }

    fn conv(nodes: Vec<MessageNode>) -> Conversation {
        Conversation {
            id: Id::from_str("c1"),
            character_id: Some("char1".into()),
            campaign_id: None,
            nodes,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_message_node_deserialize_clamps_out_of_bounds_active_variant() {
        let json = serde_json::json!({
            "id": "n1",
            "parent_id": null,
            "variants": [
                {
                    "id": "v1",
                    "role": "Assistant",
                    "content": "旧稿",
                    "created_at": "2026-01-01T00:00:00Z",
                    "status": "Draft",
                    "provenance": null
                },
                {
                    "id": "v2",
                    "role": "Assistant",
                    "content": "最终稿",
                    "created_at": "2026-01-01T00:01:00Z",
                    "status": "Final",
                    "provenance": null
                }
            ],
            "active_variant": 99
        });

        let node: MessageNode = serde_json::from_value(json).unwrap();

        assert_eq!(node.active_variant, 1);
        assert_eq!(node.active_content(), "最终稿");
    }

    #[test]
    fn test_recent_messages_as_chat_maps_roles() {
        let c = conv(vec![
            node(
                "n1",
                variant(Role::User, "写雨中告别", VariantStatus::Final),
            ),
            node(
                "n2",
                variant(Role::Assistant, "雨滴敲在屋檐…", VariantStatus::Final),
            ),
            node("n3", variant(Role::User, "继续", VariantStatus::Final)),
        ]);
        let msgs = c.recent_messages_as_chat(10, None);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, ChatRole::User);
        assert_eq!(msgs[0].content, "写雨中告别");
        assert_eq!(msgs[1].role, ChatRole::Assistant);
        assert_eq!(msgs[2].role, ChatRole::User);
    }

    #[test]
    fn test_recent_messages_as_chat_filters_discarded_and_empty() {
        let c = conv(vec![
            node("n1", variant(Role::User, "意图", VariantStatus::Final)),
            node("n2", variant(Role::Assistant, "", VariantStatus::Final)), // 空，跳过
            node(
                "n3",
                variant(Role::Assistant, "废弃稿", VariantStatus::Discarded),
            ), // 软删，跳过
            node("n4", variant(Role::Assistant, "成文", VariantStatus::Final)),
        ]);
        let msgs = c.recent_messages_as_chat(10, None);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "意图");
        assert_eq!(msgs[1].content, "成文");
    }

    #[test]
    fn test_recent_messages_as_chat_truncates_n() {
        let c = conv(vec![
            node("n1", variant(Role::User, "u1", VariantStatus::Final)),
            node("n2", variant(Role::Assistant, "a1", VariantStatus::Final)),
            node("n3", variant(Role::User, "u2", VariantStatus::Final)),
            node("n4", variant(Role::Assistant, "a2", VariantStatus::Final)),
        ]);
        // 只取最近 2 条，且保持正序
        let msgs = c.recent_messages_as_chat(2, None);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "u2");
        assert_eq!(msgs[1].content, "a2");
    }

    #[test]
    fn test_recent_messages_as_chat_before_node_excludes_target_and_after() {
        let c = conv(vec![
            node("n1", variant(Role::User, "u1", VariantStatus::Final)),
            node("n2", variant(Role::Assistant, "a1", VariantStatus::Final)),
            node("n3", variant(Role::User, "u2", VariantStatus::Final)), // 重 roll 目标
            node("n4", variant(Role::Assistant, "a2", VariantStatus::Final)),
        ]);
        // before_node_id = n3：只返回 n1, n2（不含 n3 及之后）
        let msgs = c.recent_messages_as_chat(10, Some(&Id::from_str("n3")));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "u1");
        assert_eq!(msgs[1].content, "a1");
    }

    #[test]
    fn test_recent_messages_as_chat_with_role_agree_on_filter() {
        // 两个方法底层共享 iter_recent_active，过滤规则必须一致
        let c = conv(vec![
            node("n1", variant(Role::User, "意图", VariantStatus::Final)),
            node(
                "n2",
                variant(Role::Assistant, "废弃", VariantStatus::Discarded),
            ),
            node("n3", variant(Role::Assistant, "成文", VariantStatus::Final)),
        ]);
        let as_chat = c.recent_messages_as_chat(10, None);
        let with_role = c.recent_messages_with_role(10, None);
        assert_eq!(as_chat.len(), with_role.len());
        assert_eq!(as_chat[0].content, "意图");
        assert!(with_role[0].contains("意图"));
    }
}
