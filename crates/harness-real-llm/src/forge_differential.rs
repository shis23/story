//! forge 差分预言机：用社区工具 ai4rpg/tavern-cards 的 forge CLI unpack 结果
//! 作为外部对照，字段级验证 storyforge 导入层的世界书语义。
//!
//! 用法（一次性准备，测试消费产物）：
//! ```text
//! node <tavern-cards>/scripts/tavern-cards-forge.mjs unpack destiny \
//!     --file test-card.png --output <out>/destiny --fresh
//! node <tavern-cards>/scripts/tavern-cards-forge.mjs unpack qingqing \
//!     --file "卿卿 (33).png" --output <out>/qingqing --fresh
//! STORYFORGE_FORGE_OUT=<out> cargo test -p harness-real-llm --test forge_differential
//! ```
//!
//! 对照维度（v1，结构字段级）：
//! - 条目总数 / 启用数（ours `!disabled` ↔ forge `enabled`）
//! - 按条目名匹配（ours `extra["comment"]` ↔ forge entryManifest 键）
//! - 路由（ours `LoreRoute` ↔ forge `strategy.type`）
//! - 排序（ours `order` ↔ forge `position.order`）
//! - 主键集合（ours `keys` ↔ forge `strategy.keys`）
//!
//! 内容正文对照（文件级 hash）留 v2：forge 会把正文改写成 yaml/txt 文件，
//! 需要归一化后才可比。

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use storyforge_domain::world_info::{LoreRoute, WorldInfoBook};

// ═══════════════════════════════════════════════════════════════════════════
// forge 侧 state.json 形状（只反序列化对照所需字段）
// ═══════════════════════════════════════════════════════════════════════════

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct ForgeState {
    #[serde(rename = "projectName")]
    pub project_name: Option<String>,
    pub form: Option<String>,
    #[serde(default)]
    pub mvu: bool,
    /// group → name → entry（group 常见为 "unknown"，可能有多组）
    #[serde(rename = "entryManifest", default)]
    pub entry_manifest: BTreeMap<String, BTreeMap<String, ForgeEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct ForgeEntry {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub strategy: Option<ForgeStrategy>,
    pub position: Option<ForgePosition>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub display_index: Option<i64>,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ForgeStrategy {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ForgePosition {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub order: Option<i64>,
}

impl ForgeState {
    /// 展平所有组，按 **trim 后名字** 分组（forge manifest 键保留原始
    /// 首尾空白——实测有 `"大乾_地图事件输出\n"`、`" 霍青棠"`——必须归一化
    /// 才能与 ours 的 trimmed comment 对上）。
    pub fn flat_entries(&self) -> BTreeMap<&str, Vec<&ForgeEntry>> {
        let mut flat: BTreeMap<&str, Vec<&ForgeEntry>> = BTreeMap::new();
        for group in self.entry_manifest.values() {
            for (name, entry) in group {
                flat.entry(name.trim()).or_default().push(entry);
            }
        }
        flat
    }
}

/// 读取 unpack 产物目录里的 tavern-cards-state.json
pub fn load_forge_state(dir: &Path) -> Result<ForgeState, String> {
    let path = dir.join("tavern-cards-state.json");
    let bytes =
        std::fs::read(&path).map_err(|e| format!("读 forge state 失败 {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("解析 forge state 失败: {e}"))
}

// ═══════════════════════════════════════════════════════════════════════════
// 差分报告
// ═══════════════════════════════════════════════════════════════════════════

/// 脱敏差分报告：只含条目名/计数/枚举值，无正文。
#[derive(Debug, Serialize)]
pub struct ForgeDiffReport {
    pub card: String,
    pub ours_total: usize,
    pub forge_total: usize,
    pub ours_enabled: usize,
    pub forge_enabled: usize,
    /// 一对一按名配对并做了字段级比较的条目数
    pub matched_by_name: usize,
    /// ours 没有 comment、无法按名匹配的条目数
    pub ours_unnamed: usize,
    /// forge 有、ours 按名找不到（上限 20 条示例）
    pub missing_in_ours: Vec<String>,
    /// ours 有名字、forge manifest 找不到（上限 20 条示例）
    pub missing_in_forge: Vec<String>,
    /// 重名组两侧数量不一致：`name (ours=N, forge=M)`。
    /// ST 卡允许重名条目，forge manifest 键唯一会吞并——重名组只比数量，
    /// 不做字段级配对（无法确定版本对应关系，字段比较全是假阳性）。
    pub name_count_mismatches: Vec<String>,
    /// 启停不一致（仅一对一组）：`name (ours=enabled/disabled, forge=...)`
    pub enabled_mismatches: Vec<String>,
    /// 路由不一致（仅一对一组）：`name (ours=Route, forge=type)`
    pub route_mismatches: Vec<String>,
    /// 接受的语义映射：forge `vectorized` ↔ ours `Selective`。
    /// ST vectorized = 仅向量触发；我们的 Selective 本就是向量检索池，
    /// 且 keys 为空时关键词路径不触发——行为等价。原始 `vectorized`
    /// 字段保留在 entry.extra，导出回 ST 可还原。
    pub vectorized_as_selective: usize,
    /// 接受的语义映射：forge `constant` ↔ ours `Both`。
    /// ST (constant=true, selective=true) 条目 ST 语义按蓝灯常驻；我们映射
    /// Both 后 `is_constant_route` 包含 Both，常驻注入等价，额外进向量池
    /// 是超集不是缺失。
    pub both_as_constant: usize,
    /// 排序不一致（仅一对一组）：`name (ours=N, forge=M)`
    pub order_mismatches: Vec<String>,
    /// 主键集合不一致（仅一对一组）：`name (ours=[..], forge=[..])`
    pub keys_mismatches: Vec<String>,
}

impl ForgeDiffReport {
    /// 硬不变量：两侧名字集合互覆盖（trim 归一化后无缺失）、
    /// 一对一组零启停分歧。总数允许差 = 重名收缩（记录在
    /// name_count_mismatches，非缺失）。
    pub fn hard_invariants_hold(&self) -> bool {
        self.missing_in_ours.is_empty()
            && self.missing_in_forge.is_empty()
            && self.enabled_mismatches.is_empty()
    }
}

fn route_str(route: &LoreRoute) -> &'static str {
    match route {
        LoreRoute::Constant => "constant",
        LoreRoute::Selective => "selective",
        LoreRoute::Both => "both",
        LoreRoute::Disabled => "disabled",
    }
}

fn cap_push(list: &mut Vec<String>, item: String) {
    if list.len() < 20 {
        list.push(item);
    }
}

/// 世界书字段级差分（v1：结构字段，无正文）。
///
/// 匹配策略：两侧按 trim 后名字分组。一对一组做字段级比较；
/// 数量不等的组只记 `name_count_mismatches`（ST 允许重名条目，
/// forge manifest 键唯一会吞并，组内配对无据可依）。
pub fn diff_worldbook(
    card_label: &str,
    book: Option<&WorldInfoBook>,
    forge: &ForgeState,
) -> ForgeDiffReport {
    let forge_groups = forge.flat_entries();
    let forge_total: usize = forge_groups.values().map(Vec::len).sum();
    let forge_enabled = forge_groups
        .values()
        .flat_map(|v| v.iter())
        .filter(|e| e.enabled)
        .count();

    let empty: Vec<storyforge_domain::world_info::WorldInfoEntry> = vec![];
    let ours = book.map(|b| b.entries.as_slice()).unwrap_or(&empty);

    let mut report = ForgeDiffReport {
        card: card_label.to_string(),
        ours_total: ours.len(),
        forge_total,
        ours_enabled: ours.iter().filter(|e| !e.disabled).count(),
        forge_enabled,
        matched_by_name: 0,
        ours_unnamed: 0,
        missing_in_ours: vec![],
        missing_in_forge: vec![],
        name_count_mismatches: vec![],
        enabled_mismatches: vec![],
        route_mismatches: vec![],
        vectorized_as_selective: 0,
        both_as_constant: 0,
        order_mismatches: vec![],
        keys_mismatches: vec![],
    };

    // ours 按 trim 后名字分组；无 comment 的条目用 forge 同款合成名
    // `entry_<st_id>` 回退（forge 对无名条目就是这么起 manifest 键的）
    let mut synthetic_names: Vec<String> = Vec::new();
    for entry in ours {
        let has_comment = entry
            .extra
            .get("comment")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
        if !has_comment && let Some(st_id) = entry.st_id {
            synthetic_names.push(format!("entry_{st_id}"));
        }
    }
    let mut synthetic_iter = synthetic_names.iter();
    let mut ours_groups: BTreeMap<&str, Vec<&storyforge_domain::world_info::WorldInfoEntry>> =
        BTreeMap::new();
    for entry in ours {
        let name = entry
            .extra
            .get("comment")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        match name {
            Some(name) => ours_groups.entry(name).or_default().push(entry),
            None => {
                if entry.st_id.is_some() {
                    let synth = synthetic_iter.next().expect("synthetic name pool");
                    ours_groups.entry(synth.as_str()).or_default().push(entry);
                } else {
                    report.ours_unnamed += 1;
                }
            }
        }
    }

    for (name, ours_group) in &ours_groups {
        let Some(forge_group) = forge_groups.get(name) else {
            cap_push(&mut report.missing_in_forge, (*name).to_string());
            continue;
        };
        if ours_group.len() != forge_group.len() {
            cap_push(
                &mut report.name_count_mismatches,
                format!("{name} (ours={}, forge={})", ours_group.len(), forge_group.len()),
            );
            continue;
        }
        if ours_group.len() != 1 {
            // 两侧同为多条：数量一致即可，组内配对无据可依
            continue;
        }
        let (entry, forge_entry) = (ours_group[0], forge_group[0]);
        report.matched_by_name += 1;

        let ours_enabled = !entry.disabled;
        if ours_enabled != forge_entry.enabled {
            cap_push(
                &mut report.enabled_mismatches,
                format!(
                    "{name} (ours={}, forge={})",
                    if ours_enabled { "enabled" } else { "disabled" },
                    if forge_entry.enabled { "enabled" } else { "disabled" }
                ),
            );
        }

        if let Some(strategy) = &forge_entry.strategy {
            let ours_route = route_str(&entry.route);
            match (ours_route, strategy.kind.as_str()) {
                // 接受映射（见字段文档）
                ("selective", "vectorized") => report.vectorized_as_selective += 1,
                ("both", "constant") => report.both_as_constant += 1,
                (a, b) if a != b => cap_push(
                    &mut report.route_mismatches,
                    format!("{name} (ours={a}, forge={b})"),
                ),
                _ => {}
            }
            // 主键集合：顺序无关比较
            let mut ours_keys: Vec<&str> = entry.keys.iter().map(String::as_str).collect();
            let mut forge_keys: Vec<&str> = strategy.keys.iter().map(String::as_str).collect();
            ours_keys.sort_unstable();
            forge_keys.sort_unstable();
            if ours_keys != forge_keys {
                cap_push(
                    &mut report.keys_mismatches,
                    format!("{name} (ours={ours_keys:?}, forge={forge_keys:?})"),
                );
            }
        }

        if let Some(position) = &forge_entry.position
            && let Some(forge_order) = position.order
            && i64::from(entry.order) != forge_order
        {
            cap_push(
                &mut report.order_mismatches,
                format!("{name} (ours={}, forge={forge_order})", entry.order),
            );
        }
    }

    for name in forge_groups.keys() {
        if !ours_groups.contains_key(name) {
            cap_push(&mut report.missing_in_ours, (*name).to_string());
        }
    }

    report
}

// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn forge_state_from_json(json: serde_json::Value) -> ForgeState {
        serde_json::from_value(json).unwrap()
    }

    fn book_with(entries: Vec<storyforge_domain::world_info::WorldInfoEntry>) -> WorldInfoBook {
        WorldInfoBook {
            entries,
            source: storyforge_domain::Source::ImportedFromST,
            metadata: Default::default(),
        }
    }

    fn entry(
        comment: &str,
        disabled: bool,
        route: LoreRoute,
        order: i32,
        keys: Vec<&str>,
    ) -> storyforge_domain::world_info::WorldInfoEntry {
        let mut extra = std::collections::BTreeMap::new();
        extra.insert("comment".to_string(), serde_json::json!(comment));
        storyforge_domain::world_info::WorldInfoEntry {
            st_id: None,
            keys: keys.into_iter().map(String::from).collect(),
            secondary_keys: vec![],
            content: "内容".into(),
            constant: matches!(route, LoreRoute::Constant),
            selective: matches!(route, LoreRoute::Selective),
            selective_logic: Default::default(),
            disabled,
            position: 0,
            depth: 4,
            order,
            route,
            extensions: serde_json::json!({}),
            extra,
        }
    }

    fn sample_forge() -> ForgeState {
        forge_state_from_json(serde_json::json!({
            "projectName": "样例",
            "form": "charactercard",
            "mvu": true,
            "entryManifest": {
                "unknown": {
                    "常驻设定": {
                        "enabled": true,
                        "strategy": {"type": "constant"},
                        "position": {"type": "before_character_definition", "order": 100}
                    },
                    "在场角色": {
                        "enabled": true,
                        "strategy": {"type": "selective", "keys": ["江离在场激活"]},
                        "position": {"type": "before_character_definition", "order": 560}
                    },
                    "禁用DLC": {
                        "enabled": false,
                        "strategy": {"type": "selective"},
                        "position": {"type": "before_character_definition", "order": 300}
                    }
                }
            }
        }))
    }

    #[test]
    fn test_diff_clean_match_holds_invariants() {
        let book = book_with(vec![
            entry("常驻设定", false, LoreRoute::Constant, 100, vec![]),
            entry("在场角色", false, LoreRoute::Selective, 560, vec!["江离在场激活"]),
            entry("禁用DLC", true, LoreRoute::Selective, 300, vec![]),
        ]);
        let report = diff_worldbook("样例", Some(&book), &sample_forge());
        assert!(report.hard_invariants_hold(), "{report:?}");
        assert_eq!(report.matched_by_name, 3);
        assert!(report.route_mismatches.is_empty(), "{report:?}");
        assert!(report.order_mismatches.is_empty(), "{report:?}");
        assert!(report.keys_mismatches.is_empty(), "{report:?}");
        assert!(report.missing_in_ours.is_empty() && report.missing_in_forge.is_empty());
    }

    #[test]
    fn test_diff_catches_enabled_route_order_divergence() {
        // 模拟旧导入 bug：禁用条目被丢弃 + 死路由 + order 丢失
        let book = book_with(vec![
            entry("常驻设定", false, LoreRoute::Constant, 100, vec![]),
            entry("在场角色", false, LoreRoute::Disabled, 0, vec![]),
        ]);
        let report = diff_worldbook("样例", Some(&book), &sample_forge());
        assert!(!report.hard_invariants_hold());
        assert_eq!(report.ours_total, 2);
        assert_eq!(report.forge_total, 3);
        assert_eq!(report.missing_in_ours, vec!["禁用DLC".to_string()]);
        assert_eq!(report.route_mismatches.len(), 1);
        assert_eq!(report.order_mismatches.len(), 1);
        assert_eq!(report.keys_mismatches.len(), 1);
    }

    #[test]
    fn test_forge_state_flatten_groups_by_trimmed_name() {
        // 实测 forge manifest 键保留首尾空白（" 霍青棠"、"大乾_地图事件输出\n"），
        // 展平必须按 trim 后名字归组
        let forge = forge_state_from_json(serde_json::json!({
            "entryManifest": {
                "a": {"同名": {"enabled": true}, " 空白名\n": {"enabled": true}},
                "b": {"同名 ": {"enabled": false}}
            }
        }));
        let flat = forge.flat_entries();
        assert_eq!(flat.get("同名").map(Vec::len), Some(2));
        assert_eq!(flat.get("空白名").map(Vec::len), Some(1));
        assert!(!flat.contains_key(" 空白名\n"));
    }

    #[test]
    fn test_diff_duplicate_name_groups_compare_counts_not_fields() {
        // ours 两条重名（一启一禁）vs forge 吞并后一条：记数量分歧，不出假启停分歧
        let book = book_with(vec![
            entry("战斗系统", false, LoreRoute::Constant, 100, vec![]),
            entry("战斗系统", true, LoreRoute::Selective, 100, vec![]),
        ]);
        let forge = forge_state_from_json(serde_json::json!({
            "entryManifest": {
                "unknown": {
                    "战斗系统": {
                        "enabled": true,
                        "strategy": {"type": "constant"},
                        "position": {"order": 100}
                    }
                }
            }
        }));
        let report = diff_worldbook("样例", Some(&book), &forge);
        assert_eq!(report.name_count_mismatches, vec!["战斗系统 (ours=2, forge=1)"]);
        assert!(report.enabled_mismatches.is_empty(), "{report:?}");
        assert!(report.route_mismatches.is_empty(), "{report:?}");
        assert!(report.hard_invariants_hold(), "{report:?}");
    }

    #[test]
    fn test_diff_accepts_both_as_constant_mapping() {
        let book = book_with(vec![entry("常驻", false, LoreRoute::Both, 100, vec![])]);
        let forge = forge_state_from_json(serde_json::json!({
            "entryManifest": {
                "unknown": {
                    "常驻": {
                        "enabled": true,
                        "strategy": {"type": "constant"},
                        "position": {"order": 100}
                    }
                }
            }
        }));
        let report = diff_worldbook("样例", Some(&book), &forge);
        assert_eq!(report.both_as_constant, 1);
        assert!(report.route_mismatches.is_empty(), "{report:?}");
    }
}
