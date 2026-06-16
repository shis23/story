use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::agent::{Performance, Plan};

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
}

impl From<&Performance> for SubagentSnapshot {
    fn from(p: &Performance) -> Self {
        Self {
            character_id: p.character_id.clone(),
            full_text: p.full_text.clone(),
        }
    }
}

/// 消息节点（一个位置可有多个版本，对应设计 §3.7.1 MessageNode）
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        self.active()
            .map(|v| v.content.as_str())
            .unwrap_or("")
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
            let v = self
                .active_mut()
                .ok_or("当前节点无变体")?;
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
                if current + offset < len { c.push(current + offset); }
                if offset <= current { c.push(current - offset); }
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
        let v = self
            .active_mut()
            .ok_or("当前节点无变体")?;
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
    /// 消息列表（按时间顺序，但可通过 parent_id 支持分支）
    pub nodes: Vec<MessageNode>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    pub fn new(character_id: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Id::new(),
            character_id,
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
        // 确定截止位置
        let end_idx = if let Some(bid) = before_node_id {
            self.nodes.iter().position(|n| &n.id == bid).unwrap_or(self.nodes.len())
        } else {
            self.nodes.len()
        };
        self.nodes[..end_idx]
            .iter()
            .rev()
            .take(n)
            .rev()
            .filter_map(|node| {
                let v = node.active()?;
                if v.status == VariantStatus::Discarded || v.content.is_empty() {
                    return None;
                }
                let role_label = match v.role {
                    Role::User => "用户",
                    Role::Assistant => "AI",
                };
                Some(format!("{}: {}", role_label, v.content))
            })
            .collect()
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
