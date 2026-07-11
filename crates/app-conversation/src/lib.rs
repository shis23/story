/// 对话管理（对应设计 §3.7）
///
/// 对话树持久化 + 编辑/删除/swipe/整体重 roll/部分重 roll。
/// 存储位置：data/conversations/<id>.json
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use storyforge_domain::Id;
use storyforge_domain::conversation::{
    Conversation, MessageVariant, Provenance, Role, SubagentSnapshot, VariantStatus,
};
use storyforge_domain::llm::ChatMessage;

// ─── 错误类型 ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("对话不存在: {0}")]
    NotFound(String),

    #[error("节点不存在: {0}")]
    NodeNotFound(String),

    #[error("变体索引越界: {0}")]
    VariantIndexOutOfBounds(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("序列化错误: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("部分重 roll 约束违反: {0}")]
    PartialRollViolation(String),

    #[error("变体已被丢弃，无法采纳")]
    VariantDiscarded,
}

// ─── 对话存储（对应设计 §11.3 data/conversations/）─────────────────────────

/// 对话持久化存储
pub struct ConversationStore {
    dir: PathBuf,
    /// 内存缓存（key = conversation_id）
    cache: Mutex<Vec<Conversation>>,
    /// 是否已从磁盘加载（AtomicBool 实现 &self 可修改）
    loaded: AtomicBool,
}

impl ConversationStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            cache: Mutex::new(Vec::new()),
            loaded: AtomicBool::new(false),
        }
    }

    /// 获取缓存锁，恢复被毒化的 mutex 而非 panic
    fn lock_cache(&self) -> MutexGuard<'_, Vec<Conversation>> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 确保从磁盘加载所有对话
    fn ensure_loaded(&self) {
        if self.loaded.load(Ordering::Acquire) {
            return;
        }

        let mut cache = self.lock_cache();
        // 双重检查（另一个线程可能已经加载了）
        if self.loaded.load(Ordering::Acquire) {
            return;
        }

        std::fs::create_dir_all(&self.dir).ok();

        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false)
                    && let Ok(data) = std::fs::read_to_string(&path)
                    && let Ok(conv) = serde_json::from_str::<Conversation>(&data)
                {
                    cache.push(conv);
                }
            }
        }

        self.loaded.store(true, Ordering::Release);
    }

    /// 清空缓存并标记为未加载，下次访问时从磁盘重新加载。
    ///
    /// 用于测试或外部进程修改了 `data/conversations/` 后强制刷新内存缓存。
    /// 历史 bug：loaded 一旦置 true 永不复位，外部修改磁盘后本进程缓存永远看不到。
    pub fn invalidate(&self) {
        let mut cache = self.lock_cache();
        cache.clear();
        self.loaded.store(false, Ordering::Release);
    }

    /// 持久化对话到磁盘（原子写：.tmp → rename）
    fn persist(&self, conv: &Conversation) -> Result<(), ConversationError> {
        let path = self.dir.join(format!("{}.json", conv.id.as_str()));
        storyforge_infra_util::atomic_write_json(&path, conv)?;
        Ok(())
    }

    /// 原子地修改对话：持有缓存锁完成查找→修改→持久化，消除竞态条件
    fn with_conversation_mut<F, R>(&self, id: &Id, f: F) -> Result<R, ConversationError>
    where
        F: FnOnce(&mut Conversation) -> Result<R, ConversationError>,
    {
        self.ensure_loaded();
        let mut cache = self.lock_cache();
        let conv = cache
            .iter_mut()
            .find(|c| &c.id == id)
            .ok_or_else(|| ConversationError::NotFound(id.to_string()))?;
        let result = f(conv)?;
        self.persist(conv)?;
        Ok(result)
    }

    /// 创建新对话（可绑定 Campaign）
    pub fn create(&self, character_id: Option<String>, campaign_id: Option<Id>) -> Conversation {
        self.ensure_loaded();

        let conv = Conversation::new(character_id, campaign_id);
        if let Err(e) = self.persist(&conv) {
            tracing::error!("持久化新对话失败: {e}");
        }

        let mut cache = self.lock_cache();
        cache.push(conv.clone());
        conv
    }

    /// 获取对话列表（摘要，card_name 由 Tauri 层联查填充）
    pub fn list(&self) -> Vec<ConversationSummary> {
        self.ensure_loaded();

        let cache = self.lock_cache();
        cache
            .iter()
            .map(|c| ConversationSummary {
                id: c.id.clone(),
                character_id: c.character_id.clone(),
                campaign_id: c.campaign_id.clone(),
                card_name: None, // Tauri 层联查角色卡名后填充
                message_count: c.nodes.len(),
                created_at: c.created_at,
                updated_at: c.updated_at,
            })
            .collect()
    }

    /// 按 Campaign ID 查对话（一 Campaign 一对话）
    pub fn find_by_campaign(&self, campaign_id: &Id) -> Option<Conversation> {
        self.ensure_loaded();
        let cache = self.lock_cache();
        cache
            .iter()
            .find(|c| c.campaign_id.as_ref() == Some(campaign_id))
            .cloned()
    }

    /// 获取对话详情
    pub fn fork_at(
        &self,
        source_conv_id: &Id,
        campaign_id: Id,
        fork_node_id: &Id,
    ) -> Result<Conversation, ConversationError> {
        self.ensure_loaded();

        let forked = {
            let cache = self.lock_cache();
            let source = cache
                .iter()
                .find(|c| &c.id == source_conv_id)
                .ok_or_else(|| ConversationError::NotFound(source_conv_id.to_string()))?;
            let fork_idx = source
                .nodes
                .iter()
                .position(|node| &node.id == fork_node_id)
                .ok_or_else(|| ConversationError::NodeNotFound(fork_node_id.to_string()))?;

            let mut forked = Conversation::new(source.character_id.clone(), Some(campaign_id));
            forked.nodes = source.nodes[..=fork_idx].to_vec();
            // 继承源对话水位，但钳制到 fork 前缀可归档消息数，避免把已归档前缀再归档一遍
            let forked_archivable = forked
                .nodes
                .iter()
                .filter(|n| {
                    n.active()
                        .map(|v| {
                            v.status != storyforge_domain::conversation::VariantStatus::Discarded
                        })
                        .unwrap_or(false)
                })
                .count();
            forked.archived_upto = source.archived_upto.min(forked_archivable);
            forked.updated_at = Utc::now();
            forked
        };

        self.persist(&forked)?;
        let mut cache = self.lock_cache();
        cache.push(forked.clone());
        Ok(forked)
    }

    pub fn get(&self, id: &Id) -> Option<Conversation> {
        self.ensure_loaded();

        let cache = self.lock_cache();
        cache.iter().find(|c| &c.id == id).cloned()
    }

    /// 删除对话
    pub fn delete(&self, id: &Id) -> Result<(), ConversationError> {
        self.ensure_loaded();

        let path = self.dir.join(format!("{}.json", id.as_str()));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }

        let mut cache = self.lock_cache();
        cache.retain(|c| &c.id != id);
        Ok(())
    }

    // ─── 对话操作（全部通过 with_conversation_mut 保证原子性）──────────────

    /// 追加一条 Final 状态消息（开场白等系统消息，任意角色）
    pub fn append_final_message(
        &self,
        conv_id: &Id,
        role: Role,
        content: String,
    ) -> Result<Id, ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            let node_id = conv.append_message(role, content);
            conv.updated_at = Utc::now();
            Ok(node_id)
        })
    }

    /// 追加用户消息
    pub fn append_user_message(
        &self,
        conv_id: &Id,
        content: String,
    ) -> Result<Id, ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            let node_id = conv.append_message(Role::User, content);
            conv.updated_at = Utc::now();
            Ok(node_id)
        })
    }

    /// 追加 AI 成文（Draft 状态，等待采纳）
    pub fn append_ai_draft(
        &self,
        conv_id: &Id,
        content: String,
        provenance: Option<Provenance>,
    ) -> Result<Id, ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            let node_id = conv.append_ai_draft(content, provenance);
            conv.updated_at = Utc::now();
            Ok(node_id)
        })
    }

    /// 采纳当前 AI 变体（Draft → Final）
    pub fn accept_variant(&self, conv_id: &Id, node_id: &Id) -> Result<(), ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            let node = conv
                .find_node_mut(node_id)
                .ok_or_else(|| ConversationError::NodeNotFound(node_id.to_string()))?;

            if let Some(variant) = node.active_mut() {
                if variant.status == VariantStatus::Discarded {
                    return Err(ConversationError::VariantDiscarded);
                }
                variant.status = VariantStatus::Final;
            }

            conv.updated_at = Utc::now();
            Ok(())
        })
    }

    /// 添加新变体（swipe/分支）
    pub fn add_variant(
        &self,
        conv_id: &Id,
        node_id: &Id,
        content: String,
        provenance: Option<Provenance>,
    ) -> Result<usize, ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            let node = conv
                .find_node_mut(node_id)
                .ok_or_else(|| ConversationError::NodeNotFound(node_id.to_string()))?;

            let variant = MessageVariant {
                id: Id::new(),
                role: Role::Assistant,
                content,
                created_at: Utc::now(),
                status: VariantStatus::Draft,
                provenance,
            };

            node.add_variant(variant);
            let new_index = node.active_variant;

            conv.updated_at = Utc::now();
            Ok(new_index)
        })
    }

    /// 重 roll 最后一条时：旧 active → Discarded，push 新 variant → active（原地替换语义）
    ///
    /// 与 `add_variant` 区别：add_variant 永远新增、保留旧 active 为 Final/Draft；
    /// 本方法把旧 active 降级为 Discarded（软删除，可 switch 切回查看），再 push 新 variant。
    /// 等价于「自动删旧的、留新的」，用于重 roll 对话**最后一条** AI 消息（避免无谓累积分支）。
    /// 行为是 `soft_delete + add_variant` 的原子组合。
    pub fn replace_active_variant(
        &self,
        conv_id: &Id,
        node_id: &Id,
        content: String,
        provenance: Option<Provenance>,
    ) -> Result<usize, ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            let node = conv
                .find_node_mut(node_id)
                .ok_or_else(|| ConversationError::NodeNotFound(node_id.to_string()))?;

            // 旧 active 降级为 Discarded（幂等：已 Discarded 再设不影响）
            if let Some(old) = node.variants.get_mut(node.active_variant) {
                old.status = VariantStatus::Discarded;
            }

            // push 新 variant（Draft），add_variant 内部会把 active_variant 指到末尾
            let variant = MessageVariant {
                id: Id::new(),
                role: Role::Assistant,
                content,
                created_at: Utc::now(),
                status: VariantStatus::Draft,
                provenance,
            };
            node.add_variant(variant);
            let new_index = node.active_variant;

            conv.updated_at = Utc::now();
            Ok(new_index)
        })
    }

    /// 判定 node_id 是否为对话最后一条 Assistant 消息（用于重 roll 分支策略）
    ///
    /// 后端单一事实源：按当前 `nodes.last()` 实时判定，避免前端传 isLast 标志的脏数据。
    /// 重 roll 后若用户又发新消息使原 node 不再最后，下次重 roll 它自动回退到开分支。
    pub fn is_last_assistant_node(
        &self,
        conv_id: &Id,
        node_id: &Id,
    ) -> Result<bool, ConversationError> {
        let conv = self
            .get(conv_id)
            .ok_or_else(|| ConversationError::NotFound(conv_id.to_string()))?;
        match conv.nodes.last() {
            Some(last) => Ok(last.id == *node_id
                && last
                    .active()
                    .map(|v| v.role == Role::Assistant)
                    .unwrap_or(false)),
            None => Ok(false),
        }
    }

    /// 切换变体（左右滑）
    pub fn switch_variant(
        &self,
        conv_id: &Id,
        node_id: &Id,
        index: usize,
    ) -> Result<(), ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            let node = conv
                .find_node_mut(node_id)
                .ok_or_else(|| ConversationError::NodeNotFound(node_id.to_string()))?;

            node.switch_variant(index)
                .map_err(ConversationError::VariantIndexOutOfBounds)?;

            conv.updated_at = Utc::now();
            Ok(())
        })
    }

    /// 编辑当前变体内容
    pub fn edit_variant(
        &self,
        conv_id: &Id,
        node_id: &Id,
        new_content: String,
    ) -> Result<(), ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            let node = conv
                .find_node_mut(node_id)
                .ok_or_else(|| ConversationError::NodeNotFound(node_id.to_string()))?;

            node.edit_active(new_content)
                .map_err(ConversationError::NodeNotFound)?;

            conv.updated_at = Utc::now();
            Ok(())
        })
    }

    /// 软删除当前变体（→ Discarded）
    pub fn soft_delete_variant(&self, conv_id: &Id, node_id: &Id) -> Result<(), ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            let node = conv
                .find_node_mut(node_id)
                .ok_or_else(|| ConversationError::NodeNotFound(node_id.to_string()))?;

            node.soft_delete_active()
                .map_err(ConversationError::NodeNotFound)?;

            conv.updated_at = Utc::now();
            Ok(())
        })
    }

    /// 删除指定 node 及其之后所有 node（截断对话）
    ///
    /// 语义：删除某条 AI 成文 = 撤销从这条开始的写作（含其后的所有消息）。
    /// 如果 node_id 不存在则报错；node_id 是首个被删的（保留它之前的所有消息）。
    pub fn truncate_from(&self, conv_id: &Id, node_id: &Id) -> Result<(), ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            let pos = conv
                .nodes
                .iter()
                .position(|n| &n.id == node_id)
                .ok_or_else(|| ConversationError::NodeNotFound(node_id.to_string()))?;
            conv.nodes.truncate(pos);
            // 截断后可归档条数变少：水位钳制到当前 archivable 长度，避免指向已删除前缀之外
            let archivable = conv
                .nodes
                .iter()
                .filter(|n| {
                    n.active()
                        .map(|v| {
                            v.status != storyforge_domain::conversation::VariantStatus::Discarded
                        })
                        .unwrap_or(false)
                })
                .count();
            if conv.archived_upto > archivable {
                conv.archived_upto = archivable;
            }
            conv.updated_at = Utc::now();
            Ok(())
        })
    }

    /// 推进归档水位（archived_upto）。只允许单调不减。
    ///
    /// `new_upto` 是 archivable 消息列表上的前缀长度（不是 node 下标）。
    pub fn advance_archived_upto(
        &self,
        conv_id: &Id,
        new_upto: usize,
    ) -> Result<usize, ConversationError> {
        self.with_conversation_mut(conv_id, |conv| {
            if new_upto > conv.archived_upto {
                conv.archived_upto = new_upto;
                conv.updated_at = Utc::now();
            }
            Ok(conv.archived_upto)
        })
    }

    /// 读取归档水位。
    pub fn archived_upto(&self, conv_id: &Id) -> usize {
        self.get(conv_id).map(|c| c.archived_upto).unwrap_or(0)
    }

    /// 获取最近 N 条消息（用于上下文窗口）
    pub fn recent_messages(&self, conv_id: &Id, n: usize) -> Vec<String> {
        self.get(conv_id)
            .map(|c| c.recent_messages(n))
            .unwrap_or_default()
    }

    /// 获取最近 N 条消息（带角色标签，用于注入 Agent 上下文）
    /// `before_node_id`：如果指定，只返回该节点之前的消息（重 roll 时排除目标消息）
    pub fn recent_messages_with_role(
        &self,
        conv_id: &Id,
        n: usize,
        before_node_id: Option<&Id>,
    ) -> Vec<String> {
        self.get(conv_id)
            .map(|c| c.recent_messages_with_role(n, before_node_id))
            .unwrap_or_default()
    }

    /// 获取最近 N 条消息作为 ChatMessage 列表（§22 cache 友好布局用）
    ///
    /// 返回真正的 `[user, assistant, ...]` 消息（role 映射，content 无前缀），
    /// 让历史作为独立消息段进 LLM，保证 system+history 前缀稳定、cache 命中。
    /// `before_node_id`：regenerate 重 roll 时排除目标节点及之后。
    pub fn recent_messages_as_chat(
        &self,
        conv_id: &Id,
        n: usize,
        before_node_id: Option<&Id>,
    ) -> Vec<ChatMessage> {
        self.get(conv_id)
            .map(|c| c.recent_messages_as_chat(n, before_node_id))
            .unwrap_or_default()
    }

    // ─── 部分重 roll（对应设计 §3.7.3）─────────────────────────────────────

    /// 验证部分重 roll 的合法性
    ///
    /// 约束（设计 §3.7.3）：
    /// - 可以只重跑某子 Agent ✅
    /// - 可以只重跑编剧 ✅
    /// - 可以只重跑导演 ✅
    /// - 不能只重跑导演却保留旧子产出（Plan 变了旧子产出不匹配）❌
    pub fn validate_partial_roll(
        &self,
        conv_id: &Id,
        node_id: &Id,
        targets: &[PartialRollTarget],
    ) -> Result<(), ConversationError> {
        let conv = self
            .get(conv_id)
            .ok_or_else(|| ConversationError::NotFound(conv_id.to_string()))?;

        let node = conv
            .find_node(node_id)
            .ok_or_else(|| ConversationError::NodeNotFound(node_id.to_string()))?;

        let variant = node
            .active()
            .ok_or_else(|| ConversationError::NodeNotFound("无变体".into()))?;

        let provenance = variant.provenance.as_ref().ok_or_else(|| {
            ConversationError::PartialRollViolation("该变体无溯源信息，无法部分重 roll".into())
        })?;

        // 检查约束：不能只重导演却保留旧子产出
        let rerun_director = targets
            .iter()
            .any(|t| matches!(t, PartialRollTarget::Director));
        let keep_subagents = !targets
            .iter()
            .any(|t| matches!(t, PartialRollTarget::Subagent(_)));

        if rerun_director && keep_subagents && !provenance.subagent_results.is_empty() {
            return Err(ConversationError::PartialRollViolation(
                "不能只重跑导演却保留旧子产出——Plan 变了旧子产出不匹配".into(),
            ));
        }

        Ok(())
    }
}

/// 对话摘要（列表用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: Id,
    pub character_id: Option<String>,
    /// 关联的 Campaign ID（一 Campaign 一对话）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<Id>,
    /// 关联角色卡名（前端列表显示用，由 Tauri 层联查填充）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_name: Option<String>,
    pub message_count: usize,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// 部分重 roll 目标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PartialRollTarget {
    /// 只重跑导演
    Director,
    /// 只重跑某个子 Agent（按角色 ID）
    Subagent(String),
    /// 只重跑编剧
    Editor,
}

// ─── 便捷构造 Provenance ──────────────────────────────────────────────────

/// 从写作会话结果构造 Provenance（供 pipeline 使用）
///
/// `last_hint` 为本次重 roll 附加的提示词（首次写作传 None）。
pub fn build_provenance(
    session_id: Id,
    plan: Option<storyforge_domain::agent::Plan>,
    subagent_results: &[storyforge_domain::agent::Performance],
    profile_id: Option<Id>,
    seed: u64,
    last_hint: Option<String>,
) -> Provenance {
    Provenance {
        session_id,
        plan,
        subagent_results: subagent_results
            .iter()
            .map(SubagentSnapshot::from)
            .collect(),
        profile_id,
        seed,
        last_hint,
    }
}

/// 从写作会话结果构造 Provenance（Campaign 模式，阶段 5 新增）
///
/// 与 `build_provenance` 相同，但额外从 `CampaignRuntimeContext` 中提取
/// 每个子 Agent 的 `character_instance_id`、`display_name` 和 `fallback_reason`。
///
/// 匹配逻辑：按 `Performance.character_id` 优先匹配 instance.id，再匹配 instance.name。
pub fn build_provenance_with_campaign(
    session_id: Id,
    plan: Option<storyforge_domain::agent::Plan>,
    subagent_results: &[storyforge_domain::agent::Performance],
    profile_id: Option<Id>,
    seed: u64,
    last_hint: Option<String>,
    campaign_runtime: Option<&storyforge_domain::campaign_runtime::CampaignRuntimeContext>,
) -> Provenance {
    let subagent_snapshots: Vec<SubagentSnapshot> = subagent_results
        .iter()
        .map(|perf| {
            let mut snap = SubagentSnapshot::from(perf);
            if let Some(cr) = campaign_runtime {
                if let Some(inst) = cr.find_instance_by_id_or_name(&perf.character_id) {
                    snap.character_instance_id = Some(inst.id.to_string());
                    snap.display_name = Some(inst.name.clone());
                    // 如果 character_id 不是 instance.id，说明走了 name fallback
                    if perf.character_id != inst.id.as_str() {
                        snap.fallback_reason = Some(format!(
                            "matched by name '{}' to instance id '{}'",
                            perf.character_id, inst.id
                        ));
                    }
                } else {
                    snap.fallback_reason = Some(format!(
                        "instance not found for '{}', fell back to context_package",
                        perf.character_id
                    ));
                }
            }
            snap
        })
        .collect();

    Provenance {
        session_id,
        plan,
        subagent_results: subagent_snapshots,
        profile_id,
        seed,
        last_hint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> ConversationStore {
        let dir =
            std::env::temp_dir().join(format!("storyforge_test_conv_{}", uuid::Uuid::new_v4()));
        ConversationStore::new(dir)
    }

    #[test]
    fn test_create_and_list() {
        let store = temp_store();
        let conv = store.create(Some("Seraphina".into()), None);

        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].character_id, Some("Seraphina".into()));

        // 清理
        let _ = store.delete(&conv.id);
    }

    #[test]
    fn test_append_messages() {
        let store = temp_store();
        let conv = store.create(None, None);

        store
            .append_user_message(&conv.id, "写一场戏".into())
            .unwrap();
        store
            .append_ai_draft(&conv.id, "成文内容...".into(), None)
            .unwrap();

        let updated = store.get(&conv.id).unwrap();
        assert_eq!(updated.nodes.len(), 2);
        assert_eq!(updated.nodes[0].active_content(), "写一场戏");
        assert_eq!(updated.nodes[1].active_content(), "成文内容...");
        assert_eq!(
            updated.nodes[1].active().unwrap().status,
            VariantStatus::Draft
        );

        let _ = store.delete(&conv.id);
    }

    #[test]
    fn test_accept_variant() {
        let store = temp_store();
        let conv = store.create(None, None);
        let node_id = store
            .append_ai_draft(&conv.id, "草稿".into(), None)
            .unwrap();

        store.accept_variant(&conv.id, &node_id).unwrap();

        let updated = store.get(&conv.id).unwrap();
        let node = updated.find_node(&node_id).unwrap();
        assert_eq!(node.active().unwrap().status, VariantStatus::Final);

        let _ = store.delete(&conv.id);
    }

    #[test]
    fn test_cannot_accept_discarded_variant() {
        let store = temp_store();
        let conv = store.create(None, None);
        let node_id = store
            .append_ai_draft(&conv.id, "草稿".into(), None)
            .unwrap();

        // 先软删除变体
        store.soft_delete_variant(&conv.id, &node_id).unwrap();

        // 尝试采纳已丢弃的变体应失败
        let result = store.accept_variant(&conv.id, &node_id);
        assert!(result.is_err());

        let _ = store.delete(&conv.id);
    }

    #[test]
    fn test_swipe_and_switch() {
        let store = temp_store();
        let conv = store.create(None, None);
        let node_id = store
            .append_ai_draft(&conv.id, "版本1".into(), None)
            .unwrap();

        // 添加新变体
        let new_idx = store
            .add_variant(&conv.id, &node_id, "版本2".into(), None)
            .unwrap();
        assert_eq!(new_idx, 1);

        let updated = store.get(&conv.id).unwrap();
        let node = updated.find_node(&node_id).unwrap();
        assert_eq!(node.variants.len(), 2);
        assert_eq!(node.active_content(), "版本2");

        // 切换回版本1
        store.switch_variant(&conv.id, &node_id, 0).unwrap();
        let updated = store.get(&conv.id).unwrap();
        let node = updated.find_node(&node_id).unwrap();
        assert_eq!(node.active_content(), "版本1");

        let _ = store.delete(&conv.id);
    }

    #[test]
    fn test_fork_at_copies_prefix_to_new_campaign_conversation() {
        let store = temp_store();
        let source_campaign_id = Id::from_str("source-campaign");
        let fork_campaign_id = Id::from_str("fork-campaign");
        let conv = store.create(Some("card-1".into()), Some(source_campaign_id));
        let first = store
            .append_user_message(&conv.id, "first user".into())
            .unwrap();
        let branch_point = store
            .append_ai_draft(&conv.id, "branch point".into(), None)
            .unwrap();
        let _later = store
            .append_user_message(&conv.id, "later user".into())
            .unwrap();

        let forked = store
            .fork_at(&conv.id, fork_campaign_id.clone(), &branch_point)
            .unwrap();

        assert_ne!(forked.id, conv.id);
        assert_eq!(forked.character_id, Some("card-1".into()));
        assert_eq!(forked.campaign_id, Some(fork_campaign_id));
        assert_eq!(forked.nodes.len(), 2);
        assert_eq!(forked.nodes[0].id, first);
        assert_eq!(forked.nodes[1].id, branch_point);
        assert_eq!(forked.nodes[1].active_content(), "branch point");
        assert_eq!(forked.archived_upto, 0);

        let persisted = store.get(&forked.id).unwrap();
        assert_eq!(persisted.nodes.len(), 2);
        assert!(
            store
                .fork_at(&conv.id, Id::from_str("bad-campaign"), &Id::new())
                .is_err()
        );

        let _ = store.delete(&conv.id);
        let _ = store.delete(&forked.id);
    }

    #[test]
    fn test_fork_at_inherits_archived_upto_clamped_to_prefix() {
        let store = temp_store();
        let source_campaign_id = Id::from_str("source-wm");
        let fork_campaign_id = Id::from_str("fork-wm");
        let conv = store.create(Some("card-1".into()), Some(source_campaign_id));
        let _u1 = store.append_user_message(&conv.id, "u1".into()).unwrap();
        let branch = store.append_ai_draft(&conv.id, "ai1".into(), None).unwrap();
        let _u2 = store.append_user_message(&conv.id, "u2".into()).unwrap();
        // 源对话水位 5，但 fork 前缀只有 2 条可归档 → 钳到 2
        store.advance_archived_upto(&conv.id, 5).unwrap();
        let forked = store.fork_at(&conv.id, fork_campaign_id, &branch).unwrap();
        assert_eq!(forked.nodes.len(), 2);
        assert_eq!(forked.archived_upto, 2);
        let _ = store.delete(&conv.id);
        let _ = store.delete(&forked.id);
    }

    #[test]
    fn test_truncate_from_clamps_archived_upto() {
        let store = temp_store();
        let conv = store.create(None, None);
        let n1 = store.append_user_message(&conv.id, "u1".into()).unwrap();
        let n2 = store.append_ai_draft(&conv.id, "ai1".into(), None).unwrap();
        let _n3 = store.append_user_message(&conv.id, "u2".into()).unwrap();
        store.advance_archived_upto(&conv.id, 3).unwrap();
        assert_eq!(store.archived_upto(&conv.id), 3);
        // 截断到 n2（保留 n1；删除 n2 及之后）→ 仅剩 1 条可归档
        store.truncate_from(&conv.id, &n2).unwrap();
        let updated = store.get(&conv.id).unwrap();
        assert_eq!(updated.nodes.len(), 1);
        assert_eq!(updated.nodes[0].id, n1);
        assert_eq!(updated.archived_upto, 1);
        let _ = store.delete(&conv.id);
    }

    #[test]
    fn test_edit_and_soft_delete() {
        let store = temp_store();
        let conv = store.create(None, None);
        let node_id = store
            .append_ai_draft(&conv.id, "原始内容".into(), None)
            .unwrap();

        // 编辑
        store
            .edit_variant(&conv.id, &node_id, "修改后内容".into())
            .unwrap();
        let updated = store.get(&conv.id).unwrap();
        assert_eq!(
            updated.find_node(&node_id).unwrap().active_content(),
            "修改后内容"
        );

        // 软删除
        store.soft_delete_variant(&conv.id, &node_id).unwrap();
        let updated = store.get(&conv.id).unwrap();
        assert_eq!(
            updated
                .find_node(&node_id)
                .unwrap()
                .active()
                .unwrap()
                .status,
            VariantStatus::Discarded
        );

        let _ = store.delete(&conv.id);
    }

    /// truncate_from：删除指定 node 及其后所有，保留之前的
    #[test]
    fn test_truncate_from_remains_prior_nodes() {
        let store = temp_store();
        let conv = store.create(None, None);
        // user1 → ai1 → ai2 → ai3
        let _u1 = store.append_user_message(&conv.id, "意图1".into()).unwrap();
        let ai1 = store
            .append_ai_draft(&conv.id, "成文1".into(), None)
            .unwrap();
        let _ai2 = store
            .append_ai_draft(&conv.id, "成文2".into(), None)
            .unwrap();
        let _ai3 = store
            .append_ai_draft(&conv.id, "成文3".into(), None)
            .unwrap();

        // 从 ai1 起截断（删 ai1/ai2/ai3，保留 u1）
        store.truncate_from(&conv.id, &ai1).unwrap();

        let updated = store.get(&conv.id).unwrap();
        assert_eq!(updated.nodes.len(), 1, "应只保留 user1");
        assert_eq!(updated.nodes[0].active_content(), "意图1");

        // 截断不存在的 node 应报错
        let bad = store.truncate_from(&conv.id, &Id::new());
        assert!(bad.is_err());

        let _ = store.delete(&conv.id);
    }

    #[test]
    fn test_partial_roll_validation() {
        let store = temp_store();
        let conv = store.create(None, None);
        let node_id = store
            .append_ai_draft(&conv.id, "成文".into(), None)
            .unwrap();

        // 无 Provenance → 应报错
        let result = store.validate_partial_roll(&conv.id, &node_id, &[PartialRollTarget::Editor]);
        assert!(result.is_err());

        // 有 Provenance 但尝试只重导演却保留旧子产出 → 应报错
        let provenance = Provenance {
            session_id: Id::new(),
            plan: None,
            subagent_results: vec![SubagentSnapshot {
                character_id: "A".into(),
                full_text: "旧表演".into(),
                character_instance_id: None,
                display_name: None,
                fallback_reason: None,
            }],
            profile_id: None,
            seed: 42,
            last_hint: None,
        };
        // 先加一个带 Provenance 的变体
        store
            .add_variant(&conv.id, &node_id, "带溯源的版本".into(), Some(provenance))
            .unwrap();

        let result = store.validate_partial_roll(
            &conv.id,
            &node_id,
            &[PartialRollTarget::Director], // 只重导演，保留旧子产出
        );
        assert!(result.is_err());

        // 只重编剧 → 应成功
        let result = store.validate_partial_roll(&conv.id, &node_id, &[PartialRollTarget::Editor]);
        assert!(result.is_ok());

        let _ = store.delete(&conv.id);
    }

    /// 重 roll 最后一条：replace_active_variant 把旧 active 降级 Discarded + push 新 active
    #[test]
    fn test_replace_active_variant_demotes_old_and_promotes_new() {
        let store = temp_store();
        let conv = store.create(None, None);
        // 首写一条 AI 草稿（带 provenance，便于后续可重 roll）
        let node_id = store
            .append_ai_draft(&conv.id, "初版成文".into(), Some(dummy_provenance()))
            .unwrap();

        // 原地替换（模拟重 roll 最后一条）
        let new_index = store
            .replace_active_variant(
                &conv.id,
                &node_id,
                "重 roll 版".into(),
                Some(dummy_provenance()),
            )
            .unwrap();

        let updated = store.get(&conv.id).unwrap();
        let node = updated.nodes.iter().find(|n| n.id == node_id).unwrap();
        // 两个 variant，active 指向新版（index 1）
        assert_eq!(node.variants.len(), 2);
        assert_eq!(node.active_variant, new_index);
        assert_eq!(node.active_variant, 1);
        // 旧版（index 0）被降级为 Discarded
        assert_eq!(node.variants[0].status, VariantStatus::Discarded);
        assert_eq!(node.variants[0].content, "初版成文");
        // 新版（index 1）是 Draft 且为当前内容
        assert_eq!(node.variants[1].status, VariantStatus::Draft);
        assert_eq!(node.variants[1].content, "重 roll 版");
        assert_eq!(node.active_content(), "重 roll 版");

        // 旧版仍可 switch 切回查看（软删除，未真删）
        store.switch_variant(&conv.id, &node_id, 0).unwrap();
        let updated = store.get(&conv.id).unwrap();
        let node = updated.nodes.iter().find(|n| n.id == node_id).unwrap();
        assert_eq!(node.active_variant, 0);
        assert_eq!(node.active_content(), "初版成文");

        let _ = store.delete(&conv.id);
    }

    /// is_last_assistant_node 判定：最后是 AI / 最后是 user / 空对话
    #[test]
    fn test_is_last_assistant_node_three_scenarios() {
        let store = temp_store();
        let conv = store.create(None, None);
        // 空对话
        assert!(!store.is_last_assistant_node(&conv.id, &conv.id).unwrap());

        // AI 消息 → 是最后一条 Assistant
        let ai_node = store
            .append_ai_draft(&conv.id, "AI 成文".into(), Some(dummy_provenance()))
            .unwrap();
        assert!(store.is_last_assistant_node(&conv.id, &ai_node).unwrap());

        // 再追加 user 消息 → ai_node 不再是最后一条 Assistant
        let _user_node = store.append_user_message(&conv.id, "继续".into()).unwrap();
        assert!(!store.is_last_assistant_node(&conv.id, &ai_node).unwrap());

        let _ = store.delete(&conv.id);
    }

    /// 辅助：构造一个最小 Provenance（仅供本模块测试）
    fn dummy_provenance() -> Provenance {
        Provenance {
            session_id: Id::new(),
            plan: None,
            subagent_results: vec![],
            profile_id: None,
            seed: 0,
            last_hint: None,
        }
    }

    #[test]
    fn test_advance_archived_upto_is_monotonic() {
        let store = temp_store();
        let conv = store.create(None, None);
        assert_eq!(store.archived_upto(&conv.id), 0);
        assert_eq!(store.advance_archived_upto(&conv.id, 5).unwrap(), 5);
        // 不允许回退
        assert_eq!(store.advance_archived_upto(&conv.id, 3).unwrap(), 5);
        assert_eq!(store.advance_archived_upto(&conv.id, 8).unwrap(), 8);
        let reloaded = store.get(&conv.id).unwrap();
        assert_eq!(reloaded.archived_upto, 8);
        let _ = store.delete(&conv.id);
    }
}
