use serde::{Deserialize, Serialize};

use crate::Source;
use crate::character::StWorldInfoEntry;

/// 世界书（可独立存在，也可嵌入角色卡）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldInfoBook {
    pub entries: Vec<WorldInfoEntry>,
    pub source: Source,
    /// ST character_book / lorebook 顶层未知字段（name/description/scan_depth/extensions 等）。
    /// 导入时从 StWorldInfoBook.extra 保留，导出时写回，避免 book-level metadata 静默丢失。
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
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
    /// ST entry-level fields unknown to StoryForge, preserved for round-trip.
    #[serde(default)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
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
        match (self.constant, self.selective) {
            (true, true) => LoreRoute::Both,
            (true, false) => LoreRoute::Constant,
            // ST 语义：selective=false 只表示无副键过滤；非常驻条目一律按绿灯主键触发
            (false, _) => LoreRoute::Selective,
        }
    }

    /// Enable or disable an entry while preserving a meaningful injection
    /// route. An entry that was explicitly routed as Disabled can only be
    /// restored if its ST blue/green flags describe a usable default route.
    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        if enabled && matches!(self.route, LoreRoute::Disabled) {
            let restored = self.default_route();
            if matches!(restored, LoreRoute::Disabled) {
                return Err("world-info entry has no injectable route to restore".into());
            }
            self.route = restored;
        }
        self.disabled = !enabled;
        Ok(())
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
    ///
    /// 禁用条目（v2 `disable:true` / v3 `enabled:false`）**保留**并标记 `disabled=true`，
    /// 不再丢弃：MVU 卡把 `[InitVar]` 初始变量、DLC 事件等放在禁用条目里当数据用，
    /// 运行时开关（card-shell worldbook bridge）也需要禁用条目在册才能重新启用。
    /// 注入路径（constant_entries / triggered_selective_entries / matches_query）均已检查
    /// `disabled`，禁用条目不会进入提示词。
    pub fn from_st(st: crate::character::StWorldInfoBook) -> Self {
        let entries = st.entries.into_iter().map(WorldInfoEntry::from_st).collect();

        Self {
            entries,
            metadata: st.extra,
            source: Source::ImportedFromST,
        }
    }

    /// 导出为 ST 世界书结构（reverse of `from_st`）
    pub fn to_st_book(&self) -> crate::character::StWorldInfoBook {
        crate::character::StWorldInfoBook {
            entries: self.entries.iter().map(|e| e.to_st_entry()).collect(),
            extra: self.metadata.clone(),
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

        // v2 用 `disable:true`；v3 导出用 `enabled:false`（落在 extra 里，wire 原样保留）
        let v3_enabled = st
            .extra
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let disabled = st.disable.unwrap_or(false) || !v3_enabled;

        let route = if constant && selective {
            LoreRoute::Both
        } else if constant {
            LoreRoute::Constant
        } else {
            // ST 语义：selective=false 只表示"无副键过滤"，绿灯条目仍按主键触发。
            // 旧映射把 (false,false) 判成 Disabled，会让大量普通关键词条目静默死档。
            LoreRoute::Selective
        };

        // v3 导出把 order 写成 insertion_order、depth 放进 extensions
        let order = st.order.or_else(|| {
            st.extra
                .get("insertion_order")
                .and_then(serde_json::Value::as_i64)
                .map(|v| v as i32)
        });
        let depth = st.depth.or_else(|| {
            st.extensions
                .get("depth")
                .and_then(serde_json::Value::as_i64)
                .map(|v| v as i32)
        });

        Self {
            st_id: st.id,
            keys: st.resolved_keys(),
            secondary_keys: st.resolved_secondary_keys(),
            content: st.content.unwrap_or_default(),
            constant,
            selective,
            selective_logic: SelectiveLogic::from(st.selective_logic.unwrap_or(0)),
            disabled,
            position,
            depth: depth.unwrap_or(2),
            order: order.unwrap_or(100),
            route,
            extensions: st.extensions,
            extra: st.extra,
        }
    }

    /// 导出为 ST 条目（reverse of `from_st`）
    pub fn to_st_entry(&self) -> StWorldInfoEntry {
        use serde_json::Value;
        StWorldInfoEntry {
            id: self.st_id,
            keys: self.keys.clone(),
            key_alias: None,
            secondary_keys: if self.secondary_keys.is_empty() {
                None
            } else {
                Some(self.secondary_keys.clone())
            },
            keysecondary_alias: None,
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
            extra: self.extra.clone(),
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
            extra: Default::default(),
        }
    }

    fn book(entries: Vec<WorldInfoEntry>) -> WorldInfoBook {
        WorldInfoBook {
            entries,
            source: crate::Source::Native,
            metadata: Default::default(),
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

    #[test]
    fn from_st_preserves_both_route_for_constant_and_selective_entries() {
        let lore = WorldInfoBook::from_st(crate::character::StWorldInfoBook {
            entries: vec![crate::character::StWorldInfoEntry {
                id: Some(7),
                keys: vec!["harbor".into()],
                key_alias: None,
                secondary_keys: None,
                keysecondary_alias: None,
                content: Some("both route lore".into()),
                constant: true,
                selective: true,
                selective_logic: Some(0),
                position: None,
                disable: None,
                order: Some(42),
                depth: Some(3),
                extensions: serde_json::json!({ "source": "st" }),
                extra: Default::default(),
            }],
            extra: Default::default(),
        });

        assert_eq!(lore.entries[0].route, LoreRoute::Both);
        assert_eq!(
            contents(lore.constant_entries()),
            vec!["both route lore"],
            "Both entries should stay in the director system context"
        );
        assert_eq!(
            contents(lore.triggered_selective_entries("reach the harbor")),
            vec!["both route lore"],
            "Both entries should also remain keyword-triggerable"
        );
    }

    fn st_entry_from_json(v: serde_json::Value) -> crate::character::StWorldInfoEntry {
        serde_json::from_value(v).expect("st entry json should parse")
    }

    #[test]
    fn from_st_keeps_v3_disabled_entries_as_disabled() {
        // 卿卿/命定之诗形态：v3 导出用 enabled:false（顶层无 disable 字段）
        let lore = WorldInfoBook::from_st(crate::character::StWorldInfoBook {
            entries: vec![
                st_entry_from_json(serde_json::json!({
                    "id": 1,
                    "keys": [],
                    "content": "[initvar] 初始变量 YAML",
                    "constant": false,
                    "selective": true,
                    "enabled": false,
                    "comment": "[initvar]变量初始化勿开"
                })),
                st_entry_from_json(serde_json::json!({
                    "id": 2,
                    "keys": ["旧版"],
                    "content": "旧版战斗系统",
                    "constant": true,
                    "selective": false,
                    "enabled": false
                })),
                st_entry_from_json(serde_json::json!({
                    "id": 3,
                    "keys": ["世界观"],
                    "content": "active constant",
                    "constant": true,
                    "selective": false,
                    "enabled": true
                })),
                st_entry_from_json(serde_json::json!({
                    "id": 4,
                    "keys": ["v2旧字段"],
                    "content": "v2 disable flag",
                    "constant": true,
                    "selective": false,
                    "disable": true
                })),
            ],
            extra: Default::default(),
        });

        // 全部保留（不再丢弃），禁用位正确
        assert_eq!(lore.entries.len(), 4);
        assert!(lore.entries[0].disabled, "v3 enabled:false 应标记 disabled");
        assert!(lore.entries[1].disabled);
        assert!(!lore.entries[2].disabled);
        assert!(lore.entries[3].disabled, "v2 disable:true 仍应生效");
        // 禁用的 constant 条目不得进入常驻注入
        assert_eq!(contents(lore.constant_entries()), vec!["active constant"]);
        // 导出往返：disable 位写回
        let st = lore.to_st_book();
        assert_eq!(st.entries[1].disable, Some(true));
        assert_eq!(st.entries[2].disable, Some(false));
    }

    #[test]
    fn from_st_maps_plain_keyword_entries_to_selective_not_dead() {
        // ST 语义：constant=false && selective=false 的普通关键词条目仍按主键触发
        let lore = WorldInfoBook::from_st(crate::character::StWorldInfoBook {
            entries: vec![st_entry_from_json(serde_json::json!({
                "id": 9,
                "keys": ["灯塔"],
                "content": "plain keyword lore",
                "constant": false,
                "selective": false
            }))],
            extra: Default::default(),
        });

        assert_eq!(lore.entries[0].route, LoreRoute::Selective);
        assert_eq!(
            contents(lore.triggered_selective_entries("走向灯塔")),
            vec!["plain keyword lore"]
        );
    }

    #[test]
    fn from_st_reads_v3_insertion_order_and_extensions_depth() {
        let lore = WorldInfoBook::from_st(crate::character::StWorldInfoBook {
            entries: vec![st_entry_from_json(serde_json::json!({
                "id": 5,
                "keys": ["江离在场激活"],
                "content": "persona lore",
                "constant": false,
                "selective": true,
                "insertion_order": 7,
                "position": "before_char",
                "extensions": { "depth": 3 }
            }))],
            extra: Default::default(),
        });

        let e = &lore.entries[0];
        assert_eq!(e.order, 7, "v3 insertion_order 应作为 order 读入");
        assert_eq!(e.depth, 3, "v3 extensions.depth 应作为 depth 读入");
        assert_eq!(e.position, 0);
    }
}
