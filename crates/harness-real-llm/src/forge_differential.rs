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
    /// 正则全量表（name → 配置对象）。大体积 replaceString 外置成
    /// `正则/*.txt`（带 replace_file），小正则内联——目录文件数不是正则总数，
    /// 这里才是权威计数。
    #[serde(default)]
    pub regex_scripts: Option<serde_json::Map<String, serde_json::Value>>,
    /// 开场白全量（first_mes + alternates），权威计数。
    #[serde(default)]
    pub first_messages: Option<Vec<serde_json::Value>>,
    /// Zod schema 引导脚本被 forge 特化提取（schema.ts + state.zod），
    /// 不写入 脚本/ 目录——TH 计数口径需要 +1。
    #[serde(default)]
    pub zod: Option<serde_json::Value>,
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
// V2-3：section 清单对照（开场白 / 正则 / tavern_helper 脚本）
// ═══════════════════════════════════════════════════════════════════════════

/// 非世界书面的清单对照。对照口径（与世界书差分同教义——我们保留全量，
/// forge 做归并，比对按语义口径）：
/// - 开场白：ours 按**非空**计数（实测卿卿原卡带一条空 alternate，
///   我们保留、forge 丢弃）
/// - 正则 / TH 脚本：卡内常带同名历史版本，forge 按名归并——硬断言比
///   **去重名集**大小，原始数与收缩量入报告
/// - 名字为宽松对照（forge 文件名经 sanitize，不可逆——缺口只报告不判失败）
#[derive(Debug, Serialize)]
pub struct SectionDiffReport {
    pub card: String,
    /// ours 非空开场白数（first_mes 非空计 1 + 非空 alternates）
    pub ours_greetings: usize,
    /// ours 空开场白数（保留但不参与对照）
    pub ours_empty_greetings: usize,
    pub forge_greetings: usize,
    /// ours 原始正则条数（含同名历史版本）
    pub ours_regex_raw: usize,
    /// ours 去重名集大小（对照口径）
    pub ours_regex_unique: usize,
    pub forge_regex: usize,
    pub ours_th_raw: usize,
    pub ours_th_unique: usize,
    pub forge_th_scripts: usize,
    /// ours 有名、forge（归一化后）对不上的样例（上限 10，报告用）
    pub regex_name_gaps: Vec<String>,
    pub th_name_gaps: Vec<String>,
}

impl SectionDiffReport {
    /// 硬不变量：非空开场白数一致 + 正则/TH 去重名集大小一致
    pub fn counts_match(&self) -> bool {
        self.ours_greetings == self.forge_greetings
            && self.ours_regex_unique == self.forge_regex
            && self.ours_th_unique == self.forge_th_scripts
    }
}

/// 宽松归一化：只留字母/数字（forge 文件名 sanitize 会替换标点/空白）
fn lenient_name(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

fn dir_file_stems(dir: &Path) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut stems: Vec<String> = read
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .collect();
    stems.sort();
    stems
}

fn name_gaps(ours_names: &[String], forge_stems: &[String]) -> Vec<String> {
    let forge_set: std::collections::BTreeSet<String> =
        forge_stems.iter().map(|s| lenient_name(s)).collect();
    ours_names
        .iter()
        .filter(|n| !forge_set.contains(&lenient_name(n)))
        .take(10)
        .cloned()
        .collect()
}

/// section 清单差分：开场白 / 正则 / tavern_helper 脚本。
/// 正则与开场白优先用 state.json 权威计数（目录文件数做回退——
/// forge 只把大体积正文外置成文件）。
pub fn diff_sections(
    card_label: &str,
    character: &storyforge_domain::character::Character,
    forge_dir: &Path,
    forge: &ForgeState,
) -> SectionDiffReport {
    let non_empty_alternates = character
        .alternate_greetings
        .iter()
        .filter(|g| !g.trim().is_empty())
        .count();
    let ours_greetings = usize::from(!character.first_mes.trim().is_empty()) + non_empty_alternates;
    let ours_empty_greetings = character.alternate_greetings.len() - non_empty_alternates;

    let ours_regex_names: Vec<String> = character
        .extensions
        .get("regex_scripts")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.get("scriptName").and_then(|v| v.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let ours_th_names: Vec<String> = character
        .extensions
        .get("tavern_helper")
        .and_then(|v| v.get("scripts"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.get("name").and_then(|v| v.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let forge_greeting_files = dir_file_stems(&forge_dir.join("开场白"));
    let forge_script_files = dir_file_stems(&forge_dir.join("脚本"));

    // 正则：state.regex_scripts 为权威（键即原始名，无 sanitize），目录回退
    let (forge_regex, forge_regex_names): (usize, Vec<String>) = match &forge.regex_scripts {
        Some(map) => (map.len(), map.keys().cloned().collect()),
        None => {
            let files = dir_file_stems(&forge_dir.join("正则"));
            (files.len(), files)
        }
    };
    let forge_greetings = forge
        .first_messages
        .as_ref()
        .map(Vec::len)
        .unwrap_or(forge_greeting_files.len());

    let unique = |names: &[String]| -> Vec<String> {
        let set: std::collections::BTreeSet<&str> =
            names.iter().map(|n| n.trim()).collect();
        set.into_iter().map(String::from).collect()
    };
    let ours_regex_unique_names = unique(&ours_regex_names);
    let ours_th_unique_names = unique(&ours_th_names);

    // TH 对照名单 = 脚本/ 文件 stem + zod 特化脚本名（如有）
    let mut forge_th_names = forge_script_files.clone();
    let zod_script = forge
        .zod
        .as_ref()
        .and_then(|z| z.get("scriptName"))
        .and_then(|v| v.as_str());
    if let Some(name) = zod_script {
        forge_th_names.push(name.to_string());
    }

    SectionDiffReport {
        card: card_label.to_string(),
        ours_greetings,
        ours_empty_greetings,
        forge_greetings,
        ours_regex_raw: ours_regex_names.len(),
        ours_regex_unique: ours_regex_unique_names.len(),
        forge_regex,
        ours_th_raw: ours_th_names.len(),
        ours_th_unique: ours_th_unique_names.len(),
        forge_th_scripts: forge_th_names.len(),
        regex_name_gaps: name_gaps(&ours_regex_unique_names, &forge_regex_names),
        th_name_gaps: name_gaps(&ours_th_unique_names, &forge_th_names),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// V2-2：MVU schema 对照（InitVar YAML 作 ground truth，翻译产物打分）
// ═══════════════════════════════════════════════════════════════════════════

/// 把 InitVar YAML 展平成叶路径集合（作者定义的变量树 ground truth）。
///
/// - mapping 递归拼路径；标量 / 序列 / 空 mapping 记为叶
/// - `$` 开头的键是 MVU 元数据（$meta 等），跳过
pub fn flatten_initvar_yaml(yaml: &str) -> Result<std::collections::BTreeSet<String>, String> {
    let value: serde_yaml::Value =
        serde_yaml::from_str(yaml).map_err(|e| format!("InitVar YAML 解析失败: {e}"))?;
    let mut leaves = std::collections::BTreeSet::new();
    flatten_yaml_value(&value, String::new(), &mut leaves);
    Ok(leaves)
}

fn flatten_yaml_value(
    value: &serde_yaml::Value,
    path: String,
    leaves: &mut std::collections::BTreeSet<String>,
) {
    match value {
        serde_yaml::Value::Mapping(map) if !map.is_empty() => {
            for (k, v) in map {
                let Some(key) = k.as_str() else { continue };
                if key.starts_with('$') {
                    continue;
                }
                let child = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                flatten_yaml_value(v, child, leaves);
            }
        }
        _ => {
            if !path.is_empty() {
                leaves.insert(path);
            }
        }
    }
}

/// 翻译 schema 与作者树的对齐报告（脱敏：只含键路径与计数）。
#[derive(Debug, Serialize)]
pub struct SchemaAlignmentReport {
    pub card: String,
    pub model: String,
    /// 作者树叶路径数（ground truth 规模）
    pub author_leaves: usize,
    /// 参与对照的翻译键数
    pub translated_keys: usize,
    /// 翻译键是否只是样本（旧证据封顶 40；true 时覆盖率只是下界）
    pub keys_are_sample: bool,
    /// 有据键数（在作者树中存在前缀关系）
    pub grounded: usize,
    /// 幻觉键样例（作者树中无任何前缀关系；上限 20）
    pub hallucinated: Vec<String>,
    /// 幻觉率 %（hallucinated / translated）
    pub hallucination_pct: f64,
    /// 被覆盖的作者叶数
    pub covered_leaves: usize,
    /// 作者树覆盖率 %（样本时为下界）
    pub coverage_pct: f64,
}

/// 归一化翻译键：
/// - 斜杠记法 → 点记法（实测 pro 输出 `/世界/时间`，flash 输出点记法——
///   模型间键记法不稳定，评测必须归一）
/// - 去掉 `stat_data.` 前缀（翻译产物惯例带、作者树不带）
fn normalize_translated_key(key: &str) -> String {
    let k = key.trim().trim_start_matches('/').replace('/', ".");
    k.strip_prefix("stat_data.").unwrap_or(&k).to_string()
}

/// 路径段匹配：`{角色名}` / `<xxx>` 形式的段是模板占位符，通配任意一段
/// （实测 flash 输出参数化 schema——一条模板代表所有同构角色子树）。
fn seg_match(key_seg: &str, leaf_seg: &str) -> bool {
    let is_placeholder = (key_seg.starts_with('{') && key_seg.ends_with('}'))
        || (key_seg.starts_with('<') && key_seg.ends_with('>'));
    is_placeholder || key_seg == leaf_seg
}

/// 段级前缀关系：逐段比对（含占位符通配），一方是另一方的段前缀即相关
fn path_related(key: &str, leaf: &str) -> bool {
    let key_segs: Vec<&str> = key.split('.').collect();
    let leaf_segs: Vec<&str> = leaf.split('.').collect();
    let n = key_segs.len().min(leaf_segs.len());
    key_segs[..n]
        .iter()
        .zip(&leaf_segs[..n])
        .all(|(k, l)| seg_match(k, l))
}

/// 计算翻译 schema 与作者树的覆盖率 / 幻觉率
pub fn schema_alignment(
    card: &str,
    model: &str,
    translated_keys: &[String],
    author_leaves: &std::collections::BTreeSet<String>,
    keys_are_sample: bool,
) -> SchemaAlignmentReport {
    let normalized: Vec<String> = translated_keys
        .iter()
        .map(|k| normalize_translated_key(k))
        .collect();

    let mut grounded = 0usize;
    let mut hallucinated = Vec::new();
    for key in &normalized {
        if author_leaves.iter().any(|leaf| path_related(key, leaf)) {
            grounded += 1;
        } else if hallucinated.len() < 20 {
            hallucinated.push(key.clone());
        }
    }

    let covered_leaves = author_leaves
        .iter()
        .filter(|leaf| normalized.iter().any(|key| path_related(key, leaf)))
        .count();

    let pct = |num: usize, den: usize| {
        if den == 0 { 0.0 } else { (num as f64) * 100.0 / (den as f64) }
    };
    let hallucinated_count = normalized.len() - grounded;

    SchemaAlignmentReport {
        card: card.to_string(),
        model: model.to_string(),
        author_leaves: author_leaves.len(),
        translated_keys: normalized.len(),
        keys_are_sample,
        grounded,
        hallucinated,
        hallucination_pct: pct(hallucinated_count, normalized.len()),
        covered_leaves,
        coverage_pct: pct(covered_leaves, author_leaves.len()),
    }
}

/// 在 forge unpack 的世界书目录里找 InitVar YAML（名字大小写不定）
pub fn find_initvar_yaml(forge_dir: &Path) -> Option<std::path::PathBuf> {
    let dir = forge_dir.join("世界书");
    let read = std::fs::read_dir(&dir).ok()?;
    read.flatten()
        .map(|e| e.path())
        .find(|p| {
            p.extension().is_some_and(|ext| ext == "yaml")
                && p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().contains("initvar"))
                    .unwrap_or(false)
        })
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
    fn test_flatten_initvar_yaml_leaves_and_meta_skip() {
        let yaml = r#"
世界信息:
  年号年份: 昭阳元年
  大事件:
    北狄入侵: false
  $meta:
    internal: true
主角:
  物品栏: {}
女性角色:
  江离:
    好感度: 90
"#;
        let leaves = flatten_initvar_yaml(yaml).unwrap();
        assert!(leaves.contains("世界信息.年号年份"));
        assert!(leaves.contains("世界信息.大事件.北狄入侵"));
        assert!(leaves.contains("主角.物品栏"), "空 mapping 应记为叶");
        assert!(leaves.contains("女性角色.江离.好感度"));
        assert!(
            !leaves.iter().any(|l| l.contains("$meta")),
            "$ 开头的元数据键应跳过"
        );
    }

    #[test]
    fn test_schema_alignment_scores_coverage_and_hallucination() {
        let mut author = std::collections::BTreeSet::new();
        author.insert("世界信息.年号年份".to_string());
        author.insert("女性角色.江离.好感度".to_string());
        author.insert("女性角色.江离.情绪".to_string());
        author.insert("主角.物品栏".to_string());

        let translated = vec![
            "stat_data.女性角色.江离.好感度".to_string(), // 精确命中
            "stat_data.女性角色.江离".to_string(),        // 对象级前缀 → 覆盖江离两叶
            "stat_data.凭空捏造.魔力值".to_string(),      // 幻觉
        ];
        let report = schema_alignment("样例", "m", &translated, &author, false);
        assert_eq!(report.grounded, 2);
        assert_eq!(report.hallucinated, vec!["凭空捏造.魔力值".to_string()]);
        assert!((report.hallucination_pct - 33.33).abs() < 0.4);
        // 覆盖：江离.好感度（精确+前缀）、江离.情绪（前缀）→ 2/4
        assert_eq!(report.covered_leaves, 2);
        assert!((report.coverage_pct - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_schema_alignment_accepts_slash_and_template_notations() {
        let mut author = std::collections::BTreeSet::new();
        author.insert("主角.属性.力量".to_string());
        author.insert("女性角色.江离.好感度".to_string());
        author.insert("女性角色.沈若萱.好感度".to_string());

        // 斜杠记法（实测 pro）+ 模板记法（实测 flash）
        let translated = vec![
            "/主角/属性/力量".to_string(),
            "女性角色.{角色名}.好感度".to_string(),
        ];
        let report = schema_alignment("样例", "m", &translated, &author, false);
        assert_eq!(report.grounded, 2, "{report:?}");
        assert!(report.hallucinated.is_empty(), "{report:?}");
        // 模板键应覆盖两个角色实例的好感度叶
        assert_eq!(report.covered_leaves, 3, "{report:?}");
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
