//! Meta Agent（对应设计 §9，AGENT_INTERFACES §8.3 / §9）
//!
//! 配置调试助手，独立于写作流水线。核心能力：
//! - 诊断工具集（inspect_world_info / inspect_character）
//! - Patch 提议/采纳系统
//! - MVU 五合一分析（[`mvu_import`]，对应设计 §19.4）
//! - ST 预设 LLM 分类（[`mvu_import::classify_st_preset_with_llm`]）
//! - 多轮对话框架（[`meta_conversation`]，对应设计 §9.1）

pub mod meta_conversation;
pub mod mvu_import;
pub mod prompts;

use serde::{Deserialize, Serialize};

// 重新导出常用类型（向后兼容现有 tauri-app 引用）
pub use meta_conversation::{
    MetaConversation, MetaMessage, MetaSession, MetaTurn, ToolResultDisplay, chat as meta_chat,
};
pub use mvu_import::{
    AgentSuggestion, PromptClassification, StPresetClassification, analyze_mvu_card,
    classify_st_preset_with_llm, score_card as score_card_complexity,
};

// ─── 诊断报告 ──────────────────────────────────────────────────────────────

/// 世界书诊断报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldInfoReport {
    pub total_entries: usize,
    pub constant_count: usize,
    pub selective_count: usize,
    pub disabled_count: usize,
    pub conflicts: Vec<WorldInfoConflict>,
    pub orphan_entries: Vec<usize>, // 从未被触发的绿灯条目
}

/// 世界书条目冲突
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldInfoConflict {
    pub entry_a_id: i32,
    pub entry_b_id: i32,
    pub reason: String,
}

/// 预设诊断报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetReport {
    pub name: String,
    pub prompt_count: usize,
    pub enabled_count: usize,
    pub regex_count: usize,
    pub issues: Vec<String>,
}

/// 角色卡诊断报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardReport {
    pub name: String,
    pub has_description: bool,
    pub has_personality: bool,
    pub has_scenario: bool,
    pub has_first_mes: bool,
    pub has_world_info: bool,
    pub world_info_entry_count: usize,
    pub issues: Vec<String>,
}

// ─── Patch 系统 ────────────────────────────────────────────────────────────

/// Patch 操作类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatchAction {
    /// 创建新条目
    Create {
        target: String,
        data: serde_json::Value,
    },
    /// 更新现有条目
    Update {
        target: String,
        field: String,
        value: serde_json::Value,
    },
    /// 删除条目
    Delete { target: String },
}

/// Patch 提议（Meta Agent 生成，用户采纳才执行）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    pub id: String,
    pub description: String,
    pub actions: Vec<PatchAction>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub applied: bool,
}

// ─── Meta Agent 诊断工具 ───────────────────────────────────────────────────

/// 诊断世界书：检查冲突和孤立条目
pub fn inspect_world_info(book: &storyforge_domain::world_info::WorldInfoBook) -> WorldInfoReport {
    let mut conflicts = Vec::new();

    // 检查关键词重叠冲突
    for (i, a) in book.entries.iter().enumerate() {
        for (j, b) in book.entries.iter().enumerate() {
            if i >= j {
                continue;
            }
            // 同为蓝灯且关键词有交集
            if a.constant && b.constant {
                let overlap: Vec<_> = a
                    .keys
                    .iter()
                    .filter(|k| b.keys.iter().any(|bk| bk.eq_ignore_ascii_case(k)))
                    .collect();
                if !overlap.is_empty() {
                    conflicts.push(WorldInfoConflict {
                        entry_a_id: a.st_id.unwrap_or(i as i32),
                        entry_b_id: b.st_id.unwrap_or(j as i32),
                        reason: format!("关键词重叠: {:?}", overlap),
                    });
                }
            }
        }
    }

    let orphan_entries = book
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.constant && e.keys.is_empty())
        .map(|(i, _)| i)
        .collect();

    WorldInfoReport {
        total_entries: book.entries.len(),
        constant_count: book.constant_entries().len(),
        selective_count: book.selective_entries().len(),
        disabled_count: book.entries.iter().filter(|e| e.disabled).count(),
        conflicts,
        orphan_entries,
    }
}

/// 诊断角色卡
pub fn inspect_character(card: &storyforge_domain::character::Character) -> CardReport {
    let mut issues = Vec::new();

    if card.description.is_empty() {
        issues.push("角色描述为空".into());
    }
    if card.personality.is_empty() {
        issues.push("性格描述为空".into());
    }
    if card.first_mes.is_empty() {
        issues.push("开场白为空".into());
    }
    if card.first_mes == "【首页】" {
        issues.push("开场白为占位符「首页」，可能需要脚本运行时".into());
    }

    CardReport {
        name: card.name.clone(),
        has_description: !card.description.is_empty(),
        has_personality: !card.personality.is_empty(),
        has_scenario: !card.scenario.is_empty(),
        has_first_mes: !card.first_mes.is_empty(),
        has_world_info: card.embedded_world_info.is_some(),
        world_info_entry_count: card
            .embedded_world_info
            .as_ref()
            .map(|b| b.entries.len())
            .unwrap_or(0),
        issues,
    }
}

// ─── Patch 管理 ────────────────────────────────────────────────────────────

/// Patch 存储（内存 + 持久化）
pub struct PatchStore {
    patches: std::sync::RwLock<Vec<Patch>>,
}

impl PatchStore {
    pub fn new() -> Self {
        Self {
            patches: std::sync::RwLock::new(Vec::new()),
        }
    }

    /// 提议新 Patch（不执行，等用户采纳）
    pub fn propose(&self, description: String, actions: Vec<PatchAction>) -> Patch {
        let patch = Patch {
            id: uuid::Uuid::new_v4().to_string(),
            description,
            actions,
            created_at: chrono::Utc::now(),
            applied: false,
        };
        let mut patches = self.patches.write().unwrap_or_else(|p| p.into_inner());
        patches.push(patch.clone());
        patch
    }

    /// 获取待采纳的 Patch 列表
    pub fn pending(&self) -> Vec<Patch> {
        self.patches
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|p| !p.applied)
            .cloned()
            .collect()
    }

    /// 标记 Patch 为已采纳
    pub fn accept(&self, id: &str) -> Result<(), MetaError> {
        let mut patches = self.patches.write().unwrap_or_else(|p| p.into_inner());
        let patch = patches
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| MetaError::PatchNotFound(id.into()))?;
        patch.applied = true;
        Ok(())
    }

    /// 忽略 Patch
    pub fn dismiss(&self, id: &str) -> Result<(), MetaError> {
        let mut patches = self.patches.write().unwrap_or_else(|p| p.into_inner());
        patches.retain(|p| p.id != id);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MetaError {
    #[error("Patch 不存在: {0}")]
    PatchNotFound(String),

    #[error("Patch 执行失败: {0}")]
    ExecutionFailed(String),

    #[error("目标格式无效: {0}")]
    InvalidTarget(String),
}

/// Patch 执行上下志（可修改的数据源）
pub struct PatchContext<'a> {
    /// 角色名 → 世界书条目（可修改路由/内容）
    pub world_info_entries: Option<&'a mut Vec<serde_json::Value>>,
    /// 角色字段（可修改 personality/scenario 等）
    pub character_fields: Option<&'a mut serde_json::Value>,
}

/// 解析目标字符串，返回 (类型, 索引/字段名)
///
/// 格式：
/// - "world_info[0]" → ("world_info", Some(0))
/// - "character.personality" → ("character", Some("personality"))
/// - "preset.prompts[2]" → ("preset", Some(2))
fn parse_target(target: &str) -> Result<(&str, Option<TargetRef>), MetaError> {
    if let Some(bracket) = target.find('[') {
        let kind = &target[..bracket];
        let idx_str = target[bracket + 1..].trim_end_matches(']');
        let idx: usize = idx_str
            .parse()
            .map_err(|_| MetaError::InvalidTarget(target.into()))?;
        Ok((kind, Some(TargetRef::Index(idx))))
    } else if let Some(dot) = target.find('.') {
        let kind = &target[..dot];
        let field = &target[dot + 1..];
        Ok((kind, Some(TargetRef::Field(field.to_string()))))
    } else {
        Ok((target, None))
    }
}

enum TargetRef {
    Index(usize),
    Field(String),
}

/// 执行单个 PatchAction
fn execute_action(action: &PatchAction, ctx: &mut PatchContext) -> Result<(), MetaError> {
    match action {
        PatchAction::Create { target, data } => {
            let (kind, target_ref) = parse_target(target)?;
            match kind {
                "world_info" => {
                    if let Some(ref mut entries) = ctx.world_info_entries {
                        entries.push(data.clone());
                    }
                }
                _ => return Err(MetaError::ExecutionFailed(format!("不支持创建 {kind}"))),
            }
        }
        PatchAction::Update {
            target,
            field,
            value,
        } => {
            let (kind, target_ref) = parse_target(target)?;
            match kind {
                "world_info" => {
                    if let Some(TargetRef::Index(idx)) = target_ref {
                        if let Some(ref mut entries) = ctx.world_info_entries {
                            if let Some(entry) = entries.get_mut(idx) {
                                if let Some(obj) = entry.as_object_mut() {
                                    obj.insert(field.clone(), value.clone());
                                }
                            }
                        }
                    }
                }
                "character" => {
                    if let Some(ref mut char_json) = ctx.character_fields {
                        if let Some(obj) = char_json.as_object_mut() {
                            obj.insert(field.clone(), value.clone());
                        }
                    }
                }
                _ => return Err(MetaError::ExecutionFailed(format!("不支持更新 {kind}"))),
            }
        }
        PatchAction::Delete { target } => {
            let (kind, target_ref) = parse_target(target)?;
            match kind {
                "world_info" => {
                    if let Some(TargetRef::Index(idx)) = target_ref {
                        if let Some(ref mut entries) = ctx.world_info_entries {
                            if idx < entries.len() {
                                entries.remove(idx);
                            }
                        }
                    }
                }
                _ => return Err(MetaError::ExecutionFailed(format!("不支持删除 {kind}"))),
            }
        }
    }
    Ok(())
}

/// 执行 Patch 的所有 actions
pub fn execute_patch(patch: &Patch, ctx: &mut PatchContext) -> Result<(), MetaError> {
    for action in &patch.actions {
        execute_action(action, ctx)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};

    fn make_entry(id: i32, keys: Vec<&str>, constant: bool) -> WorldInfoEntry {
        WorldInfoEntry {
            st_id: Some(id),
            keys: keys.into_iter().map(String::from).collect(),
            secondary_keys: vec![],
            content: format!("内容 {id}"),
            constant,
            selective: !constant,
            selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
            disabled: false,
            position: 0,
            depth: 2,
            order: 100,
            route: if constant {
                LoreRoute::Constant
            } else {
                LoreRoute::Selective
            },
            extensions: serde_json::json!({}),
        }
    }

    #[test]
    fn test_inspect_world_info_conflicts() {
        let book = WorldInfoBook {
            entries: vec![
                make_entry(1, vec!["龙", "冒险"], true),
                make_entry(2, vec!["龙", "宝藏"], true),
                make_entry(3, vec!["城市"], false),
            ],
            source: storyforge_domain::Source::Native,
        };

        let report = inspect_world_info(&book);
        assert_eq!(report.total_entries, 3);
        assert_eq!(report.conflicts.len(), 1); // 条目 1 和 2 有"龙"冲突
    }

    #[test]
    fn test_inspect_world_info_no_conflict() {
        let book = WorldInfoBook {
            entries: vec![
                make_entry(1, vec!["龙"], true),
                make_entry(2, vec!["城市"], true),
            ],
            source: storyforge_domain::Source::Native,
        };

        let report = inspect_world_info(&book);
        assert_eq!(report.conflicts.len(), 0);
    }

    #[test]
    fn test_patch_store_propose_and_accept() {
        let store = PatchStore::new();
        let patch = store.propose("测试 Patch".into(), vec![]);
        assert!(!patch.applied);

        let pending = store.pending();
        assert_eq!(pending.len(), 1);

        store.accept(&patch.id).unwrap();
        let pending = store.pending();
        assert_eq!(pending.len(), 0);
    }

    #[test]
    fn test_patch_store_dismiss() {
        let store = PatchStore::new();
        let patch = store.propose("要忽略的 Patch".into(), vec![]);
        store.dismiss(&patch.id).unwrap();
        assert_eq!(store.pending().len(), 0);
    }
}
