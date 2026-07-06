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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectiveLogic {
    #[default]
    And,
    Or,
    Not,
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

    pub fn matches_query(&self, query: &str) -> bool {
        if self.disabled {
            return false;
        }

        let query_lower = query.to_lowercase();
        let primary_matches = any_key_matches(&query_lower, &self.keys);
        let secondary_matches = any_key_matches(&query_lower, &self.secondary_keys);
        let has_secondary = self.secondary_keys.iter().any(|k| !k.trim().is_empty());

        if !has_secondary {
            return primary_matches;
        }

        match self.selective_logic {
            SelectiveLogic::And => primary_matches && secondary_matches,
            SelectiveLogic::Or => primary_matches || secondary_matches,
            SelectiveLogic::Not => primary_matches && !secondary_matches,
        }
    }

    fn is_constant_route(&self) -> bool {
        !self.disabled && matches!(self.route, LoreRoute::Constant | LoreRoute::Both)
    }

    fn is_selective_route(&self) -> bool {
        !self.disabled && matches!(self.route, LoreRoute::Selective | LoreRoute::Both)
    }
}

fn any_key_matches(query_lower: &str, keys: &[String]) -> bool {
    keys.iter()
        .map(|k| k.trim())
        .filter(|k| !k.is_empty())
        .any(|k| query_lower.contains(&k.to_lowercase()))
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
            .filter(|e| e.is_constant_route())
            .collect()
    }

    /// 获取所有绿灯（向量检索）条目
    pub fn selective_entries(&self) -> Vec<&WorldInfoEntry> {
        self.entries
            .iter()
            .filter(|e| e.is_selective_route())
            .collect()
    }

    pub fn triggered_selective_entries(&self, query: &str) -> Vec<&WorldInfoEntry> {
        self.entries
            .iter()
            .filter(|e| e.is_selective_route() && e.matches_query(query))
            .collect()
    }

    /// 按关键词匹配条目（非向量的简单匹配，M1 用）
    pub fn search_by_keywords(&self, query: &str) -> Vec<&WorldInfoEntry> {
        self.triggered_selective_entries(query)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        content: &str,
        route: LoreRoute,
        keys: &[&str],
        secondary_keys: &[&str],
        selective_logic: SelectiveLogic,
        disabled: bool,
    ) -> WorldInfoEntry {
        WorldInfoEntry {
            st_id: None,
            keys: keys.iter().map(|s| (*s).to_string()).collect(),
            secondary_keys: secondary_keys.iter().map(|s| (*s).to_string()).collect(),
            content: content.to_string(),
            constant: matches!(route, LoreRoute::Constant | LoreRoute::Both),
            selective: matches!(route, LoreRoute::Selective | LoreRoute::Both),
            selective_logic,
            disabled,
            position: 0,
            depth: 2,
            order: 100,
            route,
            extensions: serde_json::json!({}),
        }
    }

    fn book(entries: Vec<WorldInfoEntry>) -> WorldInfoBook {
        WorldInfoBook {
            entries,
            source: crate::Source::Native,
        }
    }

    fn contents(entries: Vec<&WorldInfoEntry>) -> Vec<&str> {
        entries.into_iter().map(|e| e.content.as_str()).collect()
    }

    #[test]
    fn triggered_selective_entries_honor_secondary_logic() {
        let lore = book(vec![
            entry(
                "and lore",
                LoreRoute::Selective,
                &["vault"],
                &["moon"],
                SelectiveLogic::And,
                false,
            ),
            entry(
                "or lore",
                LoreRoute::Selective,
                &["river"],
                &["ferry"],
                SelectiveLogic::Or,
                false,
            ),
            entry(
                "not lore",
                LoreRoute::Selective,
                &["crown"],
                &["decoy"],
                SelectiveLogic::Not,
                false,
            ),
        ]);

        assert_eq!(
            contents(lore.triggered_selective_entries("the vault opens under the moon")),
            vec!["and lore"]
        );
        assert!(
            contents(lore.triggered_selective_entries("the vault opens")).is_empty(),
            "AND requires a primary key and a secondary key"
        );
        assert_eq!(
            contents(lore.triggered_selective_entries("the ferry waits")),
            vec!["or lore"],
            "OR may trigger from a secondary key"
        );
        assert_eq!(
            contents(lore.triggered_selective_entries("the crown is hidden")),
            vec!["not lore"]
        );
        assert!(
            contents(lore.triggered_selective_entries("the crown has a decoy")).is_empty(),
            "NOT blocks when a secondary key is present"
        );
    }

    #[test]
    fn triggered_selective_entries_skip_disabled_and_constant_only_routes() {
        let lore = book(vec![
            entry(
                "constant lore",
                LoreRoute::Constant,
                &["castle"],
                &[],
                SelectiveLogic::And,
                false,
            ),
            entry(
                "selective lore",
                LoreRoute::Selective,
                &["forest"],
                &[],
                SelectiveLogic::And,
                false,
            ),
            entry(
                "both lore",
                LoreRoute::Both,
                &["harbor"],
                &[],
                SelectiveLogic::And,
                false,
            ),
            entry(
                "disabled route lore",
                LoreRoute::Disabled,
                &["dungeon"],
                &[],
                SelectiveLogic::And,
                false,
            ),
            entry(
                "disabled flag lore",
                LoreRoute::Selective,
                &["crypt"],
                &[],
                SelectiveLogic::And,
                true,
            ),
            entry(
                "empty key lore",
                LoreRoute::Selective,
                &[""],
                &[],
                SelectiveLogic::And,
                false,
            ),
        ]);

        assert_eq!(
            contents(
                lore.triggered_selective_entries("castle forest harbor dungeon crypt anything")
            ),
            vec!["selective lore", "both lore"]
        );
    }

    #[test]
    fn search_by_keywords_uses_selective_trigger_rules() {
        let lore = book(vec![
            entry(
                "constant lore",
                LoreRoute::Constant,
                &["castle"],
                &[],
                SelectiveLogic::And,
                false,
            ),
            entry(
                "selective lore",
                LoreRoute::Selective,
                &["forest"],
                &[],
                SelectiveLogic::And,
                false,
            ),
            entry(
                "blocked lore",
                LoreRoute::Selective,
                &["crown"],
                &["decoy"],
                SelectiveLogic::Not,
                false,
            ),
        ]);

        assert_eq!(
            contents(lore.search_by_keywords("castle forest crown decoy")),
            vec!["selective lore"]
        );
    }
}
