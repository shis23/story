use serde::{Deserialize, Serialize};

use crate::Source;
use crate::character::StWorldInfoEntry;

/// 世界书（可独立存在，也可嵌入角色卡）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldInfoBook {
    pub entries: Vec<WorldInfoEntry>,
    pub source: Source,
}

/// 世界书条目（内部表示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldInfoEntry {
    /// 条目 ID（ST 原始 ID，便于导出回 ST）
    pub st_id: Option<i32>,
    pub keys: Vec<String>,
    pub secondary_keys: Vec<String>,
    pub content: String,
    /// ST 原始蓝灯标记
    pub constant: bool,
    /// ST 原始绿灯标记
    pub selective: bool,
    /// 选择逻辑（0=AND, 1=OR, 2=NOT）
    pub selective_logic: SelectiveLogic,
    /// 是否禁用
    pub disabled: bool,
    /// 注入位置（保留 ST 原值，但我们不用于注入）
    pub position: i32,
    /// 注入深度
    pub depth: i32,
    /// 排序
    pub order: i32,
    /// 用户可调的路由（D13，默认按蓝绿灯映射）
    pub route: LoreRoute,
    /// ST extensions（保留原始 JSON）
    pub extensions: serde_json::Value,
}

/// 世界书条目路由（D13：用户可调，默认按灯色映射）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoreRoute {
    /// 蓝灯：进导演常驻上下文
    Constant,
    /// 绿灯：进向量检索池（默认）
    Selective,
    /// 两者都走
    Both,
    /// 不使用
    Disabled,
}

/// ST 选择逻辑
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectiveLogic {
    And,
    Or,
    Not,
}

impl Default for SelectiveLogic {
    fn default() -> Self {
        Self::And
    }
}

impl From<i32> for SelectiveLogic {
    fn from(v: i32) -> Self {
        match v {
            1 => Self::Or,
            2 => Self::Not,
            _ => Self::And,
        }
    }
}

impl WorldInfoEntry {
    /// 根据蓝绿灯自动计算默认路由
    pub fn default_route(&self) -> LoreRoute {
        if self.constant {
            LoreRoute::Constant
        } else if self.selective {
            LoreRoute::Selective
        } else {
            LoreRoute::Disabled
        }
    }
}

impl WorldInfoBook {
    /// 从 ST 世界书结构转换为领域模型
    pub fn from_st(st: crate::character::StWorldInfoBook) -> Self {
        let entries = st
            .entries
            .into_iter()
            .filter(|e| !e.disable.unwrap_or(false)) // 过滤禁用条目
            .map(WorldInfoEntry::from_st)
            .collect();

        Self {
            entries,
            source: Source::ImportedFromST,
        }
    }

    /// 导出为 ST 世界书结构（reverse of `from_st`）
    pub fn to_st_book(&self) -> crate::character::StWorldInfoBook {
        crate::character::StWorldInfoBook {
            entries: self.entries.iter().map(|e| e.to_st_entry()).collect(),
        }
    }

    /// 获取所有蓝灯（常驻）条目
    pub fn constant_entries(&self) -> Vec<&WorldInfoEntry> {
        self.entries
            .iter()
            .filter(|e| e.route == LoreRoute::Constant || e.route == LoreRoute::Both)
            .collect()
    }

    /// 获取所有绿灯（向量检索）条目
    pub fn selective_entries(&self) -> Vec<&WorldInfoEntry> {
        self.entries
            .iter()
            .filter(|e| e.route == LoreRoute::Selective || e.route == LoreRoute::Both)
            .collect()
    }

    /// 按关键词匹配条目（非向量的简单匹配，M1 用）
    pub fn search_by_keywords(&self, query: &str) -> Vec<&WorldInfoEntry> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.keys
                    .iter()
                    .any(|k| query_lower.contains(&k.to_lowercase()))
            })
            .collect()
    }
}

impl WorldInfoEntry {
    /// 从 ST 条目转换
    fn from_st(st: StWorldInfoEntry) -> Self {
        let constant = st.constant;
        let selective = st.selective;
        let position = st.position_as_i32();

        let route = if constant {
            LoreRoute::Constant
        } else if selective {
            LoreRoute::Selective
        } else {
            LoreRoute::Disabled
        };

        Self {
            st_id: st.id,
            keys: st.keys,
            secondary_keys: st.secondary_keys.unwrap_or_default(),
            content: st.content.unwrap_or_default(),
            constant,
            selective,
            selective_logic: SelectiveLogic::from(st.selective_logic.unwrap_or(0)),
            disabled: false,
            position,
            depth: st.depth.unwrap_or(2),
            order: st.order.unwrap_or(100),
            route,
            extensions: st.extensions,
        }
    }

    /// 导出为 ST 条目（reverse of `from_st`）
    pub fn to_st_entry(&self) -> StWorldInfoEntry {
        use serde_json::Value;
        StWorldInfoEntry {
            id: self.st_id,
            keys: self.keys.clone(),
            secondary_keys: if self.secondary_keys.is_empty() {
                None
            } else {
                Some(self.secondary_keys.clone())
            },
            content: Some(self.content.clone()),
            constant: self.constant,
            selective: self.selective,
            selective_logic: Some(match self.selective_logic {
                SelectiveLogic::And => 0,
                SelectiveLogic::Or => 1,
                SelectiveLogic::Not => 2,
            }),
            position: Some(Value::Number(self.position.into())),
            disable: Some(self.disabled),
            order: Some(self.order),
            depth: Some(self.depth),
            extensions: self.extensions.clone(),
        }
    }
}
