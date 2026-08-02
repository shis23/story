//! Gate 5 后端等价套件：同一组输入分别跑在 JSON 与 SQLite 上，比较
//! 操作结果（成功/失败/错误类别）与规范化领域快照。
//!
//! `sqlite_runtime::activate` 是进程全局 OnceLock，JSON facade 拒绝与活动
//! SQLite runtime 共存——因此本二进制在**同一进程内先完整跑 JSON 阶段**（无需
//! activation），随后激活 SQLite 并跑同一 op 序列，最后统一比较（与既有
//! sqlite_* 集成测试的单 authority 约束一致）。
//!
//! 比较规则（PLAN §10.3）：
//! - 领域快照：卡片/角色库/Campaign/实例/知识/任务/总结/Turn/会话/世界书/MVU +
//!   compress jobs + outbox + 重启恢复状态，经 id 规范（已知 id → 唯一
//!   ⟨label:N⟩）与时间戳规范（→ ⟨ts⟩）后逐项相等；快照读取**任何失败都
//!   直接失败测试**（缺失文件/表不得静默成空数组，四.1）。
//! - 操作结果：op 名 + 成功/失败类别 + 规范化消息。
//! - 重启恢复（三.2）：真实子进程 + 生产启动恢复入口（resolve_backend →
//!   StorageFacade → AppState → recover_turns → recover_compress_jobs），
//!   父进程断言子进程报告的各后端自身恢复契约 + 二次恢复幂等。
//!
//! 测试列表（本二进制单 authority，SQLite 只由主测试激活）：
//! - `backend_parity_equivalent_domain_snapshots`：主等价矩阵。
//! - `parity_detects_mutant_swap_of_accepted_attempt` / `..._variant`：四.3
//!   突变测试——交换 fixture 数据后奇偶校验必须失败（归一化不得掩盖差异）。
//! - `restart_child_entry`：真实重启子进程入口（环境变量门控）。

use std::path::Path;
use std::sync::Arc;

use storyforge_app_agent::PostProcessOutcome;
use storyforge_domain::Id;
use storyforge_domain::agent::PostProcessResult;
use storyforge_domain::character_knowledge::{
    CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
};
use storyforge_domain::story_task::{NewTaskSpec, TaskUpdate};
use storyforge_domain::turn::{AttemptStatus, TurnRecord, TurnStatus};
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_infra_sqlite::cutover::{
    CutoverOutcome, CutoverPlan, CutoverRequest, recover_or_verify,
};
use storyforge_lib::production_postprocess as pp;
use storyforge_lib::sqlite_runtime;
use storyforge_lib::storage_backend::{BackendCapability, StorageFacade};
use storyforge_lib::{AppState, TurnWorkflow};

fn write_json(path: &Path, value: serde_json::Value) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

/// 起始 fixture：1 张双角色卡 + 1 个角色库条目（无 Campaign——由 op 创建）。
///
/// char-store-1 的 extensions 携带 ST scoped regex 脚本（三.4 resolver 的
/// 跨后端等价判别数据：stored/source/card/name 四键都能解析出同一脚本）。
fn write_fixture(dir: &Path) {
    write_json(
        &dir.join("cards.json"),
        serde_json::json!([{
            "card": {
                "id": "card-1",
                "name": "等价卡",
                "source_character_id": "char-src-1",
                "character_definitions": [
                    {
                        "id": "def-alice", "card_id": "card-1", "name": "Alice",
                        "role_type": "protagonist",
                        "persona_prompt": "勇敢", "behavior_rules": "不撒谎",
                        "base_backstory": [], "group": "主角团",
                        "variable_schema": [
                            {"key": "hp", "label": "生命", "value_type": "int",
                             "default": 10, "description": null, "group": "角色"}
                        ]
                    },
                    {
                        "id": "def-bob", "card_id": "card-1", "name": "Bob",
                        "role_type": "supporting",
                        "persona_prompt": "沉稳", "behavior_rules": "守约",
                        "base_backstory": [], "group": "主角团",
                        "variable_schema": []
                    }
                ],
                "campaign_variable_schema": [
                    {"key": "gold", "label": "金币", "value_type": "int",
                     "default": 0, "description": null, "group": "全局"}
                ],
                "raw_card_json": {},
                "extraction_status": "extracted",
                "extraction_message": null
            },
            "imported_at": "2026-07-01T00:00:00Z"
        }]),
    );
    write_json(
        &dir.join("characters.json"),
        serde_json::json!([{
            "id": "char-store-1",
            "info": {
                "source_character_id": "char-src-1",
                "name": "Alice", "description": "主角", "personality": "勇敢",
                "scenario": "地下城", "first_mes": "你好，我是 Alice。",
                "mes_example": "", "post_history_instructions": "",
                "alternate_greetings": [], "system_prompt": "扮演 Alice",
                "tags": ["主角"], "creator": "test", "character_version": "1.0",
                "spec_version": "2.0",
                "extensions": {
                    "regex_scripts": [{
                        "scriptName": "酒窖别名",
                        "findRegex": "酒窖|地窖",
                        "replaceString": "密窖",
                        "placement": [2]
                    }]
                },
                "embedded_world_info": null, "renderable_assets": null,
                "raw_card_json": {}, "has_world_info": false,
                "has_renderable_assets": false, "world_info_count": 0,
                "world_info_entries": []
            },
            "imported_at": "2026-07-01 10:00:00"
        },
        {
            "id": "char-store-2",
            "info": {
                "source_character_id": "char-src-2",
                "name": "Carol", "description": "配角", "personality": "机智",
                "scenario": "集市", "first_mes": "想买点什么？",
                "mes_example": "", "post_history_instructions": "",
                "alternate_greetings": [], "system_prompt": "扮演 Carol",
                "tags": [], "creator": "test", "character_version": "1.0",
                "spec_version": "2.0", "extensions": {},
                "embedded_world_info": null, "renderable_assets": null,
                "raw_card_json": {}, "has_world_info": false,
                "has_renderable_assets": false, "world_info_count": 0,
                "world_info_entries": []
            },
            "imported_at": "2026-07-02 10:00:00"
        }]),
    );
    for name in [
        "campaigns.json",
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "turns.json",
        "mvu_translations.json",
        "compress_jobs.json",
    ] {
        write_json(&dir.join(name), serde_json::json!([]));
    }
}

/// 测试用 tauri::State 包装（既有 sqlite_command_lifecycle 同款 transmute）。
fn tauri_state_for_test(state: &Arc<AppState>) -> tauri::State<'_, Arc<AppState>> {
    unsafe { std::mem::transmute::<&Arc<AppState>, tauri::State<'_, Arc<AppState>>>(state) }
}

// ─── 规范化 ────────────────────────────────────────────────────────────

/// 每次运行（JSON/SQLite 各自）记录到的实体 id，用于快照与消息规范化。
///
/// 四.3：**每个 id 必须拥有唯一标签**（uuid → ⟨label:N⟩）——旧实现把同一
/// 路径下所有随机 UUID 映射成同一个标签（如所有 attempt → 同一个
/// `⟨path:...attempt_id⟩`），会掩盖 accepted_attempt_id / variant 等字段的
/// 真实差异。手动标签经 `register_labeled` 按基数分配 `{base}:{N}`；
/// 路径登记经全局序号分配 `path:{child}#{N}`。
#[derive(Debug, Default, Clone)]
struct IdRegistry {
    ids: Vec<(String, String)>, // (label, actual id)
    path_seq: usize,
}

impl IdRegistry {
    /// 手动登记：同一 id 重复登记保留首个标签；不同 id 用 `register_labeled`
    /// 保证标签唯一（四.3）。
    fn register(&mut self, label: &str, id: &str) {
        if !self.ids.iter().any(|(_, actual)| actual == id) {
            self.ids.push((label.to_string(), id.to_string()));
        }
    }

    /// 按基数分配唯一标签（四.3）：`{base}:{N}`，N 为每个新 id 递增。
    fn register_labeled(&mut self, base: &str, id: &str) {
        if self.contains_actual(id) {
            return;
        }
        let n = self
            .ids
            .iter()
            .filter(|(label, _)| label.starts_with(&format!("{base}:")))
            .count()
            + 1;
        self.ids.push((format!("{base}:{n}"), id.to_string()));
    }

    fn contains_actual(&self, id: &str) -> bool {
        self.ids.iter().any(|(_, actual)| actual == id)
    }
    /// 把结构内尚未登记的随机 UUID id 按字段路径登记（如 `tasks[].id`，数组无
    /// 下标——instances/tasks 等是集合语义，双后端顺序不同（见 list_* 实现），
    /// 标签必须与顺序无关；内容差异仍显式可见）。
    ///
    /// 四.3：每个**不同**的 UUID 分配不同标签 `path:{child}#{seq}`——相同 id
    /// 出现多处（同一 Attempt 的 id 同时出现在 attempt_id 与 accepted_attempt_id）
    /// 仍映射到同一个标签（按 actual id 去重）。
    fn register_uuid_ids_by_path(&mut self, value: &mut serde_json::Value, path: &str) {
        fn is_uuid(s: &str) -> bool {
            s.len() == 36
                && s.bytes().enumerate().all(|(i, b)| match i {
                    8 | 13 | 18 | 23 => b == b'-',
                    _ => b.is_ascii_hexdigit(),
                })
        }
        fn is_id_key(k: &str) -> bool {
            k == "id" || k.ends_with("_id")
        }
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map.iter_mut() {
                    let child = format!("{path}.{k}");
                    if is_id_key(k)
                        && let serde_json::Value::String(s) = v
                        && is_uuid(s)
                        && !self.contains_actual(s)
                    {
                        self.path_seq += 1;
                        self.ids
                            .push((format!("path:{child}#{}", self.path_seq), s.clone()));
                    }
                    self.register_uuid_ids_by_path(v, &child);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items.iter_mut() {
                    self.register_uuid_ids_by_path(item, &format!("{path}[]"));
                }
            }
            _ => {}
        }
    }
    /// 对象数组按规范化序列化内容排序（集合语义，双后端顺序无关；比较前必须
    /// 排序，否则 SQLite 与 JSON 的 list_* 顺序差异会误报不等价）。
    fn sort_object_arrays(&self, value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Array(items) => {
                for item in items.iter_mut() {
                    self.sort_object_arrays(item);
                }
                items.sort_by(|a, b| {
                    let ka = serde_json::to_string(a).unwrap_or_default();
                    let kb = serde_json::to_string(b).unwrap_or_default();
                    ka.cmp(&kb)
                });
            }
            serde_json::Value::Object(map) => {
                for (_, v) in map.iter_mut() {
                    self.sort_object_arrays(v);
                }
            }
            _ => {}
        }
    }
    /// 已知 id → ⟨label⟩；未知 id 视为领域值，必须两边一致。
    /// 错误消息会把 id 嵌入长文本（如 "找不到任务 <uuid>"），故做子串替换；
    /// 先替换较长的 id（UUID 等长、fixture id 含非 hex 字符，无嵌套歧义）。
    fn canonicalize(&self, value: &mut serde_json::Value) {
        let mut ordered: Vec<&(String, String)> = self.ids.iter().collect();
        ordered.sort_by_key(|(_, actual)| std::cmp::Reverse(actual.len()));
        match value {
            serde_json::Value::String(s) => {
                for (label, actual) in ordered {
                    if s.contains(actual.as_str()) {
                        *s = s.replace(actual.as_str(), &format!("⟨{label}⟩"));
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    self.canonicalize(item);
                }
            }
            serde_json::Value::Object(map) => {
                for (_, v) in map.iter_mut() {
                    self.canonicalize(v);
                }
            }
            _ => {}
        }
    }
}

/// 时间戳等非领域字段 → ⟨ts⟩（明确规范化，不整体忽略对象）。
const TS_KEYS: &[&str] = &[
    "created_at",
    "updated_at",
    "imported_at",
    "analyzed_at",
    "committed_at",
    "started_at",
    "finished_at",
    "exported_at",
];

fn canonicalize_timestamps(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                canonicalize_timestamps(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if TS_KEYS.contains(&k.as_str()) {
                    *v = serde_json::json!("⟨ts⟩");
                } else {
                    canonicalize_timestamps(v);
                }
            }
        }
        _ => {}
    }
}

/// 集合数组按**后端无关语义键**预排序（title/name/knowledge_text/turn…），
/// 使 id 路径登记的遍历顺序跨后端确定——四.3 唯一标签（⟨label:N⟩）的前提：
/// 双后端 list_* 顺序不同时，按语义键排序后注册顺序一致，同一实体得到同一
/// 标签。无语义键的对象回退到"掩码序列化"（UUID/时间戳 → 占位符，避免随机
/// id 决定顺序）。
fn stable_sort_collections(value: &mut serde_json::Value) {
    const SEMANTIC_KEYS: &[&str] = &[
        "title",
        "name",
        "knowledge_text",
        "character_name",
        "code",
        "content",
        "turn",
    ];
    fn masked_key(v: &serde_json::Value) -> String {
        let mut c = v.clone();
        mask_volatile(&mut c);
        serde_json::to_string(&c).unwrap_or_default()
    }
    fn mask_volatile(v: &mut serde_json::Value) {
        fn is_uuid(s: &str) -> bool {
            s.len() == 36
                && s.bytes().enumerate().all(|(i, b)| match i {
                    8 | 13 | 18 | 23 => b == b'-',
                    _ => b.is_ascii_hexdigit(),
                })
        }
        match v {
            serde_json::Value::String(s) if is_uuid(s) => {
                *s = "⟨uuid⟩".to_string();
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    mask_volatile(item);
                }
            }
            serde_json::Value::Object(map) => {
                for (k, val) in map.iter_mut() {
                    if TS_KEYS.contains(&k.as_str()) {
                        *val = serde_json::json!("⟨ts⟩");
                    } else {
                        mask_volatile(val);
                    }
                }
            }
            _ => {}
        }
    }
    match value {
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                stable_sort_collections(item);
            }
            if items.iter().all(|i| i.is_object()) {
                let semantic = |v: &serde_json::Value| -> Option<String> {
                    SEMANTIC_KEYS.iter().find_map(|key| {
                        v.get(*key).and_then(|x| match x {
                            serde_json::Value::String(s) => Some(s.clone()),
                            serde_json::Value::Number(n) => Some(n.to_string()),
                            _ => None,
                        })
                    })
                };
                items.sort_by(|a, b| {
                    let (sa, sb) = (semantic(a), semantic(b));
                    match (sa, sb) {
                        (Some(ka), Some(kb)) => (ka, masked_key(a)).cmp(&(kb, masked_key(b))),
                        _ => masked_key(a).cmp(&masked_key(b)),
                    }
                });
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                stable_sort_collections(v);
            }
        }
        _ => {}
    }
}

/// 恢复诊断文案（各后端私有）→ ⟨recovery_reason⟩：JSON 恢复写
/// "启动恢复：崩溃时处于 X 态"，SQLite 写 "sqlite ... recovery: incomplete
/// turn failed..."。这是后端私有诊断文本（非领域语义），双后端各按自身文案
/// 终态化同一 Turn；归一化后仍显式可见"存在恢复原因"。
fn normalize_recovery_reasons(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_recovery_reasons(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if k == "failure_reason" {
                    if let Some(s) = v.as_str() {
                        let is_recovery_text = s.starts_with("启动恢复：")
                            || s.contains("recovery: incomplete turn failed")
                            || (s.contains("sqlite") && s.contains("recover"));
                        if is_recovery_text {
                            *v = serde_json::json!("⟨recovery_reason⟩");
                        }
                    }
                } else {
                    normalize_recovery_reasons(v);
                }
            }
        }
        _ => {}
    }
}

/// 操作记录：名称 + 类别（ok/validation/not_found/storage/internal）+ 规范化消息。
#[derive(Debug, Clone, PartialEq, Eq)]
struct OpRecord {
    name: &'static str,
    kind: &'static str,
    message: String,
}

fn classify(err: &storyforge_lib::error::TauriCommandError) -> &'static str {
    use storyforge_lib::error::TauriCommandError;
    match err {
        TauriCommandError::Validation { .. } => "validation",
        TauriCommandError::NotFound { .. } => "not_found",
        TauriCommandError::Storage { .. } => "storage",
        TauriCommandError::Llm { .. } => "llm",
        TauriCommandError::Internal { .. } => "internal",
        TauriCommandError::Pipeline { .. } => "pipeline",
        TauriCommandError::Cancelled => "cancelled",
    }
}

/// 一次后端运行的完整产出。
#[derive(Debug, Clone)]
struct PhaseOutput {
    results: Vec<OpRecord>,
    snapshot: serde_json::Value,
    /// 删除前的领域快照（含 Turn/Attempt/知识/任务/总结等删除级联会清掉的数据）。
    snapshot_before_delete: serde_json::Value,
    /// 关键 Pipeline 事件序列（postprocess Done/Skipped/Failed），经真实 helper
    /// `postprocess_pipeline_event`（runtime_support）派生——双后端输入相同 →
    /// 事件序列必须逐项相等（三.3：测试调用真实 helper，禁止手拼字符串）。
    pipeline_events: Vec<String>,
}

/// 四.3 突变：在**单侧**运行注入的数据损坏（另一侧保持真值），奇偶校验必须
/// 失败——证明 IdRegistry 归一化不会掩盖 accepted_attempt_id / variant 差异。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TamperKind {
    /// accept 后把 accepted_attempt_id 换成另一 attempt 的 id。
    SwapAcceptedAttempt,
    /// accept 后交换被接受 attempt 与首个 attempt 的 variant_id。
    SwapVariant,
}

// ─── 重启恢复（三.2）───────────────────────────────────────────────────

/// 子进程报告（紧凑单行 JSON，父进程解析）。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RestartReport {
    backend: String,
    /// 恢复后 Turn 状态（id, status, attempts[{id,status}]）。
    turns: Vec<ReportTurn>,
    /// 二次恢复后的 Turn 列表（幂等断言：与 turns 逐项相等）。
    turns_after_second_recovery: Vec<ReportTurn>,
    /// 仍处于非终态（Generating/DraftReady/Committing/…）的 Turn 数。
    turns_non_terminal: usize,
    /// 恢复中被终态化（Failed）的 Turn 数。
    turns_failed: usize,
    compress_jobs: Vec<ReportJob>,
    compress_reset_first: usize,
    compress_reset_second: usize,
    /// 待 accept 的 pending outbox 单位数（各后端原始口径）。
    outbox_pending: usize,
    /// 活跃 Campaign（各后端原始口径：JSON 从 active_campaign.json 恢复，
    /// SQLite 为进程内指针——重启后为 null）。
    active_campaign: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReportTurn {
    id: String,
    status: String,
    attempts: Vec<ReportAttempt>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReportAttempt {
    id: String,
    status: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReportJob {
    id: String,
    status: String,
    attempts: u32,
}

// ─── 驱动 ──────────────────────────────────────────────────────────────

struct ParityDriver {
    state: Arc<AppState>,
    sqlite: bool,
    results: Vec<OpRecord>,
    pipeline_events: Vec<String>,
    ids: IdRegistry,
    campaign_id: Option<Id>,
    conv_id: Option<Id>,
    instance_ids: Vec<Id>,
    task_ids: Vec<Id>,
    turn_ids: Vec<Id>,
    extracted_card_id: Option<Id>,
    /// delete_campaign 成功后置位：快照对 Turn 缺失放行（级联删除是合法状态，
    /// 等价性仍由另一侧的存在/缺失对比保证——四.1 的"缺失即错误"只针对
    /// 存储读取失败与异常缺失）。
    deleted_campaign: bool,
}

impl ParityDriver {
    fn new(state: Arc<AppState>, sqlite: bool) -> Self {
        Self {
            state,
            sqlite,
            results: Vec::new(),
            pipeline_events: Vec::new(),
            ids: IdRegistry::default(),
            campaign_id: None,
            conv_id: None,
            instance_ids: Vec::new(),
            task_ids: Vec::new(),
            turn_ids: Vec::new(),
            extracted_card_id: None,
            deleted_campaign: false,
        }
    }

    fn push(&mut self, name: &'static str, kind: &'static str, message: String) {
        self.results.push(OpRecord {
            name,
            kind,
            message,
        });
    }

    /// 三.3：经真实 helper（`backend_workflows::postprocess_pipeline_event`，
    /// runtime_support 唯一事件派生点）记录 Pipeline 事件——不手拼字符串。
    fn record_postprocess_events(&mut self, result: &pp::ProductionPostprocessResult) {
        if let Some(event) = storyforge_lib::backend_workflows::postprocess_pipeline_event(
            Some(&Ok(result.clone())),
            "",
            false,
        ) {
            self.pipeline_events.push(format!("{event:?}"));
        }
    }

    fn refresh_ids(&mut self) {
        if let Some(cid) = &self.campaign_id {
            if let Ok(Some(rec)) = self.state.storage().get_campaign(cid)
                && let Some(lin) = rec.campaign.lineage_id
            {
                // lineage 是随机 UUID，双后端各自生成——显式登记为领域字段。
                self.ids.register("lineage", lin.as_str());
            }
            // instance/task 的 id **不**在此按 list 顺序登记（双后端 list_*
            // 顺序可能不同，顺序相关标签会分叉）——统一由快照的稳定语义键
            // 预排序 + 路径登记分配（四.3 唯一标签）。
            if let Ok(instances) = self.state.storage().list_instances(cid) {
                for inst in &instances {
                    if !self.instance_ids.contains(&inst.id) {
                        self.instance_ids.push(inst.id.clone());
                    }
                }
            }
            if let Ok(tasks) = self.state.storage().list_tasks(cid) {
                for task in &tasks {
                    if !self.task_ids.contains(&task.id) {
                        self.task_ids.push(task.id.clone());
                    }
                }
            }
        }
    }

    fn campaign_revision(&self) -> u64 {
        let cid = self.campaign_id.as_ref().expect("campaign");
        self.state
            .storage()
            .get_campaign(cid)
            .ok()
            .flatten()
            .map(|r| r.campaign.revision)
            .unwrap_or(0)
    }

    /// 全量 op 序列（JSON/SQLite 共用同一份）。
    ///
    /// `tamper`：四.3 突变注入（仅用于突变测试的**单侧**运行）。
    async fn run(&mut self, tamper: Option<TamperKind>) -> PhaseOutput {
        let st = self.state.clone();
        let storage = st.storage();

        // ── create：Campaign lifecycle ─────────────────────────────────
        match storyforge_lib::create_campaign(
            "card-1".to_string(),
            "等价局".to_string(),
            Some("你好，冒险者。".to_string()),
            tauri_state_for_test(&st),
        ) {
            Ok(dto) => {
                self.campaign_id = Some(Id::from_str(&dto.id));
                self.conv_id = dto.conversation_id.as_ref().map(Id::from_str);
                self.ids.register_labeled("campaign", &dto.id);
                if let Some(conv) = &dto.conversation_id {
                    self.ids.register_labeled("conversation", conv);
                }
                self.refresh_ids();
                self.push(
                    "create_campaign",
                    "ok",
                    format!("instances={}", dto.instance_count),
                );
            }
            Err(e) => self.push("create_campaign", classify(&e), e.to_string()),
        }
        let cid = self.campaign_id.clone().expect("campaign created");
        let conv_id = self.conv_id.clone().expect("conversation created");
        // 活跃指针（重启报告断言用：JSON 落盘 / SQLite 进程内）。
        let _ = storyforge_lib::set_active_campaign(
            cid.as_str().to_string(),
            tauri_state_for_test(&st),
        );

        // definition 重复加入 → validation（Bob 已被 create_campaign 实例化）
        match storyforge_lib::add_campaign_instance(
            cid.as_str().to_string(),
            Some("def-bob".to_string()),
            None,
            None,
            None,
            tauri_state_for_test(&st),
        ) {
            Ok(_) => self.push("add_instance_dup", "ok", "unexpected".into()),
            Err(e) => self.push("add_instance_dup", classify(&e), e.to_string()),
        }
        // 临时角色
        match storyforge_lib::add_campaign_instance(
            cid.as_str().to_string(),
            None,
            Some("临时客".to_string()),
            Some("路过的旅人".to_string()),
            None,
            tauri_state_for_test(&st),
        ) {
            Ok(dto) => {
                self.ids.register("instance", &dto.id);
                self.push("add_instance_temp", "ok", "ok".into());
            }
            Err(e) => self.push("add_instance_temp", classify(&e), e.to_string()),
        }
        // 同名 → validation
        match storyforge_lib::add_campaign_instance(
            cid.as_str().to_string(),
            None,
            Some("临时客".to_string()),
            None,
            None,
            tauri_state_for_test(&st),
        ) {
            Ok(_) => self.push("add_instance_same_name", "ok", "unexpected".into()),
            Err(e) => self.push("add_instance_same_name", classify(&e), e.to_string()),
        }
        self.refresh_ids();

        // ── variables ─────────────────────────────────────────────────
        for (name, key, value) in [
            ("set_var_gold", "gold", serde_json::json!(42)),
            ("set_var_clock", "story_clock", serde_json::json!("Day 5")),
        ] {
            match storyforge_lib::set_campaign_variable(
                cid.as_str().to_string(),
                key.to_string(),
                value,
                Some(0),
                tauri_state_for_test(&st),
            ) {
                Ok(()) => self.push(name, "ok", "ok".into()),
                Err(e) => self.push(name, classify(&e), e.to_string()),
            }
        }
        match storyforge_lib::add_campaign_variable(
            cid.as_str().to_string(),
            "mood".to_string(),
            "心情".to_string(),
            "string".to_string(),
            serde_json::json!("平静"),
            None,
            tauri_state_for_test(&st),
        ) {
            Ok(()) => self.push("add_var", "ok", "ok".into()),
            Err(e) => self.push("add_var", classify(&e), e.to_string()),
        }
        match storyforge_lib::add_campaign_variable(
            cid.as_str().to_string(),
            "bad key__x".to_string(),
            "坏键".to_string(),
            "string".to_string(),
            serde_json::json!("v"),
            None,
            tauri_state_for_test(&st),
        ) {
            Ok(()) => self.push("add_var_bad_key", "ok", "unexpected".into()),
            Err(e) => self.push("add_var_bad_key", classify(&e), e.to_string()),
        }
        match storyforge_lib::sync_campaign_variable_schema(
            cid.as_str().to_string(),
            tauri_state_for_test(&st),
        ) {
            Ok(dto) => self.push("sync_schema", "ok", format!("added={}", dto.added)),
            Err(e) => self.push("sync_schema", classify(&e), e.to_string()),
        }

        // ── tasks ─────────────────────────────────────────────────────
        let tid1 = match storyforge_lib::create_task(
            cid.as_str().to_string(),
            "找钥匙".to_string(),
            "地下城三层".to_string(),
            vec![],
            Some(1),
            tauri_state_for_test(&st),
        ) {
            Ok(id) => {
                self.ids.register("task", &id);
                self.push("create_task", "ok", "ok".into());
                Some(Id::from_str(&id))
            }
            Err(e) => {
                self.push("create_task", classify(&e), e.to_string());
                None
            }
        };
        if let Some(tid) = &tid1 {
            match storyforge_lib::complete_task(tid.as_str().to_string(), tauri_state_for_test(&st))
            {
                Ok(()) => self.push("complete_task", "ok", "ok".into()),
                Err(e) => self.push("complete_task", classify(&e), e.to_string()),
            }
        }
        let tid2 = match storyforge_lib::create_task(
            cid.as_str().to_string(),
            "护送商队".to_string(),
            "穿过峡谷".to_string(),
            vec![],
            Some(2),
            tauri_state_for_test(&st),
        ) {
            Ok(id) => {
                self.ids.register("task", &id);
                self.push("create_task_2", "ok", "ok".into());
                Some(Id::from_str(&id))
            }
            Err(e) => {
                self.push("create_task_2", classify(&e), e.to_string());
                None
            }
        };
        if let Some(tid) = &tid2 {
            match storyforge_lib::abandon_task(tid.as_str().to_string(), tauri_state_for_test(&st))
            {
                Ok(()) => self.push("abandon_task", "ok", "ok".into()),
                Err(e) => self.push("abandon_task", classify(&e), e.to_string()),
            }
        }
        let missing_task = Id::new().to_string();
        self.ids.register("missing_task", &missing_task);
        match storyforge_lib::complete_task(missing_task.clone(), tauri_state_for_test(&st)) {
            Ok(()) => self.push("complete_task_missing", "ok", "unexpected".into()),
            Err(e) => self.push("complete_task_missing", classify(&e), e.to_string()),
        }
        self.refresh_ids();

        // ── instance variables / promote ──────────────────────────────
        let alice = self
            .instance_ids
            .iter()
            .find(|id| {
                storage
                    .get_instance(&cid, id)
                    .map(|i| i.as_ref().map(|i| i.name == "Alice").unwrap_or(false))
                    .unwrap_or(false)
            })
            .cloned()
            .expect("alice instance");
        match storyforge_lib::set_character_variable(
            cid.as_str().to_string(),
            alice.as_str().to_string(),
            "hp".to_string(),
            serde_json::json!(100),
            Some(0),
            tauri_state_for_test(&st),
        ) {
            Ok(()) => self.push("set_char_var", "ok", "ok".into()),
            Err(e) => self.push("set_char_var", classify(&e), e.to_string()),
        }
        let missing_inst = Id::new().to_string();
        self.ids.register("missing_instance", &missing_inst);
        match storyforge_lib::set_character_variable(
            cid.as_str().to_string(),
            missing_inst.clone(),
            "hp".to_string(),
            serde_json::json!(1),
            Some(0),
            tauri_state_for_test(&st),
        ) {
            Ok(()) => self.push("set_char_var_missing", "ok", "unexpected".into()),
            Err(e) => self.push("set_char_var_missing", classify(&e), e.to_string()),
        }
        let temp = self
            .instance_ids
            .iter()
            .find(|id| {
                storage
                    .get_instance(&cid, id)
                    .map(|i| i.as_ref().map(|i| i.name == "临时客").unwrap_or(false))
                    .unwrap_or(false)
            })
            .cloned()
            .expect("temp instance");
        for (name, _expect_ok) in [("promote_temp", true), ("promote_temp_again", false)] {
            match storyforge_lib::promote_temporary_instance(
                cid.as_str().to_string(),
                temp.as_str().to_string(),
                tauri_state_for_test(&st),
            ) {
                Ok(()) => self.push(name, "ok", "ok".into()),
                Err(e) => self.push(name, classify(&e), e.to_string()),
            }
        }

        // ── opening ───────────────────────────────────────────────────
        match storyforge_lib::apply_campaign_opening(
            cid.as_str().to_string(),
            "改写后的开场".to_string(),
            tauri_state_for_test(&st),
        ) {
            Ok(()) => self.push("apply_opening", "ok", "ok".into()),
            Err(e) => self.push("apply_opening", classify(&e), e.to_string()),
        }

        // ── cards（extract / list / get / delete）─────────────────────
        let card_count_before = storage.list_cards().map(|c| c.len()).unwrap_or(0);
        match storyforge_lib::extract_characters(
            "char-src-2".to_string(),
            None,
            tauri_state_for_test(&st),
        )
        .await
        {
            Ok(dto) => {
                self.extracted_card_id = Some(Id::from_str(&dto.id));
                self.ids.register("card", &dto.id);
                self.push("extract_characters", "ok", format!("card={}", dto.name));
            }
            Err(e) => self.push("extract_characters", classify(&e), e.to_string()),
        }
        match storyforge_lib::list_cards(tauri_state_for_test(&st)) {
            Ok(cards) => self.push(
                "list_cards",
                "ok",
                format!("count={} (before={card_count_before})", cards.len()),
            ),
            Err(e) => self.push("list_cards", classify(&e), e.to_string()),
        }
        if let Some(card_id) = self.extracted_card_id.clone() {
            match storyforge_lib::get_card(card_id.as_str().to_string(), tauri_state_for_test(&st))
            {
                Ok(dto) => self.push(
                    "get_extracted_card",
                    "ok",
                    format!("name={} defs={}", dto.name, dto.definition_count),
                ),
                Err(e) => self.push("get_extracted_card", classify(&e), e.to_string()),
            }
            match storyforge_lib::delete_card(
                card_id.as_str().to_string(),
                tauri_state_for_test(&st),
            ) {
                Ok(()) => self.push("delete_extracted_card", "ok", "ok".into()),
                Err(e) => self.push("delete_extracted_card", classify(&e), e.to_string()),
            }
        }
        match storyforge_lib::list_cards(tauri_state_for_test(&st)) {
            Ok(cards) => self.push(
                "list_cards_after_delete",
                "ok",
                format!("count={}", cards.len()),
            ),
            Err(e) => self.push("list_cards_after_delete", classify(&e), e.to_string()),
        }

        // ── 三.4：backend-neutral scoped-regex 角色解析（stored/source/card/name）
        let tool_characters = st.snapshot_tool_ctx().characters.clone();
        let resolve_count = |key: &str| {
            storyforge_lib::backend_workflows::collect_scoped_regex_scripts_for_backend(
                Some(key),
                &tool_characters,
                Some(storage),
            )
            .map(|scripts| scripts.len())
            .unwrap_or(0)
        };
        self.push(
            "scoped_regex_stored",
            "ok",
            format!("count={}", resolve_count("char-store-1")),
        );
        self.push(
            "scoped_regex_source",
            "ok",
            format!("count={}", resolve_count("char-src-1")),
        );
        self.push(
            "scoped_regex_name",
            "ok",
            format!("count={}", resolve_count("Alice")),
        );
        self.push(
            "scoped_regex_card",
            "ok",
            format!("count={}", resolve_count("card-1")),
        );
        self.push(
            "scoped_regex_missing",
            "ok",
            format!("count={}", resolve_count("nobody")),
        );

        // ── Meta typed patch：propose + accept（跨 Turn 的 stale 拒绝）──
        let drift_key = "extra_key".to_string();
        match storyforge_lib::set_character_variable(
            cid.as_str().to_string(),
            alice.as_str().to_string(),
            drift_key.clone(),
            serde_json::json!("drift"),
            Some(0),
            tauri_state_for_test(&st),
        ) {
            Ok(()) => self.push("drift_inject", "ok", "ok".into()),
            Err(e) => self.push("drift_inject", classify(&e), e.to_string()),
        }
        let proposed = storyforge_lib::meta_propose_campaign_repairs(
            cid.as_str().to_string(),
            tauri_state_for_test(&st),
        );
        let patch_count = proposed.as_ref().map(|p| p.len()).unwrap_or(0);
        match &proposed {
            Ok(_patches) => self.push("meta_propose", "ok", format!("patches={patch_count}")),
            Err(e) => self.push("meta_propose", classify(e), e.to_string()),
        }
        let proposed_revision = self.campaign_revision();
        let first_patch_id = proposed
            .as_ref()
            .ok()
            .and_then(|patches| patches.first())
            .and_then(|p| p.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if let Some(patch_id) = &first_patch_id {
            self.ids.register("patch", patch_id);
            match storyforge_lib::meta_accept_typed_patch(
                patch_id.clone(),
                cid.as_str().to_string(),
                tauri_state_for_test(&st),
            ) {
                Ok(()) => self.push("meta_accept", "ok", "ok".into()),
                Err(e) => self.push("meta_accept", classify(&e), e.to_string()),
            }
        }

        // ── Turn 1 流水线：draft → regenerate → postprocess → Accept ──
        let base_rev = self.campaign_revision();
        let user_node = st
            .conv_store
            .append_user_message(&conv_id, "打开大门".into())
            .expect("append user message");
        let turn = TurnRecord::new(cid.clone(), conv_id.clone(), user_node, base_rev);
        let turn_id = turn.turn_id.clone();
        self.ids.register_labeled("turn", turn_id.as_str());
        self.turn_ids.push(turn_id.clone());
        storage.save_turn(&turn).expect("save turn");

        let attempt1 = Id::new();
        self.ids.register_labeled("attempt", attempt1.as_str());
        let workflow = TurnWorkflow::new(storage.clone(), st.conv_store.clone());
        // JSON 流水线先落 draft 节点（SQLite 由 preaccept UoW 落）；
        // JSON 的 provisional variant id 必须是真实节点 id。
        let provisional_variant = if self.sqlite {
            Id::new()
        } else {
            st.conv_store
                .append_ai_draft(&conv_id, "草稿甲：众人进入地下城".to_string(), None)
                .expect("JSON pipeline lands draft node")
        };
        // 仅 JSON 登记 provisional（它就是最终 variant 节点）；SQLite 的
        // provisional 是幻影 id（真实节点由 UoW 生成）——登记它会占掉标签序号，
        // 使双后端 variant 标签分叉（四.3 唯一标签）。
        if !self.sqlite {
            self.ids
                .register_labeled("variant", provisional_variant.as_str());
        }
        let draft = workflow
            .create_draft_attempt(storyforge_lib::DraftAttemptRequest {
                campaign_id: &cid,
                conversation_id: &conv_id,
                turn_id: &turn_id,
                attempt_id: &attempt1,
                draft_text: "草稿甲：众人进入地下城",
                pending_temporary_instances: vec![],
                provisional_variant_id: Some(&provisional_variant),
                provenance: None,
            })
            .expect("create draft attempt");
        self.ids
            .register_labeled("variant", draft.variant_id.as_str());
        self.push("draft", "ok", "ok".into());

        // regenerate：JSON 流水线先 replace_active_variant（SQLite 由 UoW 完成）
        let attempt2 = Id::new();
        self.ids.register_labeled("attempt", attempt2.as_str());
        if !self.sqlite {
            st.conv_store
                .replace_active_variant(
                    &conv_id,
                    &draft.variant_id,
                    "草稿乙：改写后的正文".to_string(),
                    None,
                )
                .expect("JSON pipeline replaces active variant");
        }
        let _regen = workflow
            .append_regenerate_attempt(storyforge_lib::RegenerateAttemptRequest {
                campaign_id: &cid,
                conversation_id: &conv_id,
                turn_id: &turn_id,
                previous_variant_id: &draft.variant_id,
                attempt_id: &attempt2,
                draft_text: "草稿乙：改写后的正文",
                pending_temporary_instances: vec![],
                provenance: None,
            })
            .expect("regenerate attempt");
        self.push("regenerate", "ok", "ok".into());

        // postprocess：JSON 走 PostprocessMutationService，SQLite 走 preaccept UoW
        let pp_outcome = PostProcessOutcome {
            summary: None,
            summary_attempted: false,
            post_process_attempted: true,
            post_process: Some(PostProcessResult {
                knowledge_updates: vec![CharacterKnowledgeUpdate {
                    character_id: alice.clone(),
                    knowledge_text: "宝箱藏在酒窖暗门后".to_string(),
                    source: KnowledgeSource::Witnessed,
                    source_character_id: None,
                    pinned: false,
                    broadcast: None,
                    propagation: PropagationPolicy::Open,
                }],
                variable_updates: vec![storyforge_domain::agent::VariableUpdate {
                    instance_id: Some(alice.clone()),
                    key: "hp".to_string(),
                    value: serde_json::json!(120),
                }],
                task_updates: vec![TaskUpdate {
                    task_id: None,
                    new_status: storyforge_domain::story_task::TaskStatus::Active,
                    new_task: Some(NewTaskSpec {
                        title: "调查酒窖".to_string(),
                        description: "看看暗门后面有什么".to_string(),
                        triggers: vec![],
                        related_characters: vec![],
                    }),
                }],
                parse_succeeded: true,
            }),
        };
        // postprocess：双后端走同一生产服务（共享纯函数构建 MutationBatch），
        // 仅 batch 源不同（JSON 投影 store / SQLite runtime 快照），sink 路由到
        // 各自后端。present_chars 传真实在场角色——生产语义，不是空集逃生口。
        //
        // 事件捕获（三.3）：经真实 helper 派生 PostProcessDone / Skipped /
        // Failed 事件——apply_outcome 是共享的写回逻辑，事件只由其结果驱动，
        // 双后端结果结构相同 → 事件序列逐项相等。
        let pp_result: Result<pp::ProductionPostprocessResult, pp::ProductionPostprocessError> =
            (|| {
                let sink = storyforge_lib::BackendTurnAttemptSink::production(storage.clone());
                let runtime;
                let service = if self.sqlite {
                    let campaign_rec = storage
                        .get_campaign(&cid)
                        .map_err(|e| pp::ProductionPostprocessError::Storage(e.to_string()))?
                        .ok_or_else(|| {
                            pp::ProductionPostprocessError::Storage(
                                "campaign missing for sqlite postprocess runtime".to_string(),
                            )
                        })?;
                    let instances = storage
                        .list_instances(&cid)
                        .map_err(|e| pp::ProductionPostprocessError::Storage(e.to_string()))?;
                    let knowledge = storage
                        .list_knowledge(&cid)
                        .map_err(|e| pp::ProductionPostprocessError::Storage(e.to_string()))?;
                    let tasks = storage
                        .list_tasks(&cid)
                        .map_err(|e| pp::ProductionPostprocessError::Storage(e.to_string()))?;
                    let definitions_by_id = storage
                        .list_cards()
                        .map_err(|e| pp::ProductionPostprocessError::Storage(e.to_string()))?
                        .into_iter()
                        .flat_map(|c| c.card.character_definitions)
                        .map(|def| (def.id.clone(), def))
                        .collect();
                    runtime = storyforge_domain::campaign_runtime::CampaignRuntimeContext {
                        campaign: campaign_rec.campaign,
                        instances,
                        definitions_by_id,
                        knowledge,
                        tasks,
                        turn: 0,
                    };
                    pp::ProductionPostprocessService::new_runtime(&runtime, &sink)
                } else {
                    let store = storage
                        .json_campaign_store(BackendCapability::Postprocess, "parity postprocess")
                        .map_err(|e| pp::ProductionPostprocessError::Storage(e.to_string()))?;
                    pp::ProductionPostprocessService::new_json(store, &sink)
                };
                let (_tx, cancel_rx) = tokio::sync::watch::channel(false);
                service.apply_outcome(
                    &pp::PostprocessIdentity {
                        turn_id: turn_id.clone(),
                        attempt_id: attempt2.clone(),
                        campaign_id: cid.clone(),
                        conversation_id: conv_id.clone(),
                        turn_number: 1,
                    },
                    Some(pp_outcome.clone()),
                    &[alice.as_str().to_string()],
                    &cancel_rx,
                )
            })();
        match &pp_result {
            Ok(result) => {
                assert!(result.applied, "postprocess must apply");
                self.record_postprocess_events(result);
                self.push("postprocess", "ok", "ok".into());
            }
            Err(e) => {
                // Failed 路径仍记录事件（三.3：经真实 helper 派生）。
                if let Some(event) = storyforge_lib::backend_workflows::postprocess_pipeline_event(
                    Some(&Err(e.clone())),
                    &e.to_string(),
                    false,
                ) {
                    self.pipeline_events.push(format!("{event:?}"));
                }
                self.push("postprocess", "storage", e.to_string());
            }
        }

        // Accept：revision base_rev → base_rev+1
        match workflow.accept_by_variant(&cid, &conv_id, &draft.variant_id, false) {
            Ok(outcome) => self.push(
                "accept",
                "ok",
                format!("turn_status={:?}", outcome.turn_status),
            ),
            Err(e) => self.push("accept", "storage", e.to_string()),
        }
        let turn_after = storage.get_turn(&turn_id).expect("read turn");
        assert_eq!(
            turn_after.expect("turn exists").status,
            TurnStatus::Committed,
            "accept 后 Turn 必须 Committed"
        );
        let campaign_after = storage.get_campaign(&cid).expect("read campaign");
        assert_eq!(
            campaign_after.expect("campaign exists").campaign.revision,
            base_rev + 1,
            "accept 后 revision 必须 +1"
        );

        // ── 四.2：原始持久化 pending_state_changes.status 双后端逐字节等价 ──
        // 不再归一化 batch status——Wave-2b 已让 JSON 提交后落盘 "committed"。
        // JSON 直接读磁盘 turns.json；SQLite 读权威表（get_turn 即持久化真值）。
        let raw_batch_status = if self.sqlite {
            storage
                .get_turn(&turn_id)
                .expect("read turn")
                .expect("turn exists")
                .find_attempt(&attempt2)
                .and_then(|a| a.pending_state_changes.as_ref())
                .map(|b| format!("{:?}", b.status).to_lowercase())
                .unwrap_or_else(|| "<none>".to_string())
        } else {
            let raw: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(storage.data_dir().join("turns.json"))
                    .expect("read turns.json"),
            )
            .expect("parse turns.json");
            raw.as_array()
                .and_then(|turns| {
                    turns.iter().find(|t| {
                        t.get("turn_id").and_then(|v| v.as_str()) == Some(turn_id.as_str())
                    })
                })
                .and_then(|t| t.get("attempts"))
                .and_then(|a| a.as_array())
                .and_then(|attempts| {
                    attempts.iter().find(|a| {
                        a.get("attempt_id").and_then(|v| v.as_str()) == Some(attempt2.as_str())
                    })
                })
                .and_then(|a| a.get("pending_state_changes"))
                .and_then(|b| b.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("<none>")
                .to_string()
        };
        self.push(
            "turn1_raw_batch_status",
            "ok",
            format!("status={raw_batch_status}"),
        );

        // ── 四.3 突变注入（仅单侧运行；另一侧保持真值）────────────────
        // 不 push op 记录：突变不得改变 op 序列（快照差异才是判别点）。
        if let Some(tamper) = tamper {
            match tamper {
                TamperKind::SwapAcceptedAttempt => {
                    storage
                        .update_turn_record(&turn_id, |record| {
                            let first = record.attempts.first().map(|a| a.attempt_id.clone());
                            record.accepted_attempt_id = first;
                            record.touch();
                        })
                        .expect("tamper accepted_attempt_id");
                }
                TamperKind::SwapVariant => {
                    // regenerate 复用同一 variant 节点（attempt1/attempt2 共享
                    // variant_id），"互换"是结构上的 no-op——突变改为把被接受
                    // attempt 的 variant_id 指向一个**不存在的伪造 id**（领域
                    // 上非法的值），奇偶校验必须能识别。
                    storage
                        .update_turn_record(&turn_id, |record| {
                            if let Some(accepted_idx) = record.attempts.iter().position(|a| {
                                Some(&a.attempt_id) == record.accepted_attempt_id.as_ref()
                            }) {
                                record.attempts[accepted_idx].variant_id =
                                    Id::from_str("mutant-foreign-variant");
                            }
                            record.touch();
                        })
                        .expect("tamper variant id");
                }
            }
        }

        // ── Turn 2：draft → edit-stale（改写后 Attempt 变 Stale）──────
        let turn2_user = st
            .conv_store
            .append_user_message(&conv_id, "继续前进".into())
            .expect("append user message");
        let turn2 = TurnRecord::new(cid.clone(), conv_id.clone(), turn2_user, base_rev + 1);
        let turn2_id = turn2.turn_id.clone();
        self.ids.register_labeled("turn", turn2_id.as_str());
        self.turn_ids.push(turn2_id.clone());
        storage.save_turn(&turn2).expect("save turn 2");
        let attempt3 = Id::new();
        self.ids.register_labeled("attempt", attempt3.as_str());
        let provisional_variant2 = if self.sqlite {
            Id::new()
        } else {
            st.conv_store
                .append_ai_draft(&conv_id, "草稿丙：查看密道".to_string(), None)
                .expect("JSON pipeline lands draft node")
        };
        let draft2 = workflow
            .create_draft_attempt(storyforge_lib::DraftAttemptRequest {
                campaign_id: &cid,
                conversation_id: &conv_id,
                turn_id: &turn2_id,
                attempt_id: &attempt3,
                draft_text: "草稿丙：查看密道",
                pending_temporary_instances: vec![],
                provisional_variant_id: Some(&provisional_variant2),
                provenance: None,
            })
            .expect("create draft attempt 2");
        self.ids
            .register_labeled("variant", draft2.variant_id.as_str());
        self.push("turn2_draft", "ok", "ok".into());
        workflow
            .edit_variant_with_stale_mark(&conv_id, &draft2.variant_id, "用户改写密道段落")
            .expect("edit variant with stale mark");
        self.push("turn2_edit_stale", "ok", "ok".into());
        let turn2_after = storage.get_turn(&turn2_id).expect("read turn 2");
        assert_eq!(
            turn2_after
                .expect("turn 2 exists")
                .find_attempt(&attempt3)
                .expect("attempt 3 exists")
                .status,
            AttemptStatus::Stale,
            "edit-stale 后 Attempt 必须 Stale（双方后端等价）"
        );

        // stale typed patch：propose 在 Turn 前盖章（revision=proposed_revision），
        // accept 在 Turn 后（revision 已前进）→ 双方都拒绝。
        if let Some(patch_id) = &first_patch_id {
            match storyforge_lib::meta_accept_typed_patch(
                patch_id.clone(),
                cid.as_str().to_string(),
                tauri_state_for_test(&st),
            ) {
                Ok(()) => self.push(
                    "meta_accept_stale",
                    "ok",
                    format!("unexpected (proposed@rev{proposed_revision})"),
                ),
                Err(e) => self.push("meta_accept_stale", classify(&e), e.to_string()),
            }
        }

        // ── MVU preview/apply ─────────────────────────────────────────
        let mvu_stored = storyforge_lib::campaign_store::StoredMvuTranslation {
            source_character_id: Id::from_str("char-src-1"),
            character_name: "Alice".to_string(),
            analyzed_at: "2026-07-05T00:00:00Z".to_string(),
            translation: {
                let mut t =
                    storyforge_domain::mvu_translation::MvuTranslation::pure_data_fallback(vec![]);
                t.update_rules = vec!["hp 减伤时同步生命值".to_string()];
                t
            },
        };
        match storage.save_mvu(&mvu_stored) {
            Ok(()) => self.push("mvu_save", "ok", "ok".into()),
            Err(e) => self.push("mvu_save", "storage", e.to_string()),
        }
        match storyforge_lib::preview_mvu_apply_for_backend(
            storage,
            &mvu_stored.source_character_id,
        ) {
            Ok(previews) => self.push("mvu_preview", "ok", format!("count={}", previews.len())),
            Err(e) => self.push("mvu_preview", "storage", e.to_string()),
        }
        match storyforge_lib::apply_mvu_schema_for_backend(
            storage,
            &mvu_stored.source_character_id,
            &Id::from_str("def-alice"),
        ) {
            Ok(()) => self.push("mvu_apply", "ok", "ok".into()),
            Err(e) => self.push("mvu_apply", "storage", e.to_string()),
        }

        // ── Chronicle compression：入队/claim/成功/失败重试 状态机等价 ──
        let lineage = storage
            .get_campaign(&cid)
            .ok()
            .flatten()
            .and_then(|r| r.campaign.lineage_id.clone());
        let (uncovered_a, uncovered_b) =
            storyforge_lib::count_uncovered_chronicle_levels_for_backend(storage, &cid)
                .unwrap_or((0, 0));
        self.push(
            "chronicle_uncovered",
            "ok",
            format!("a={uncovered_a} b={uncovered_b}"),
        );
        let (job_id, created) = storage
            .enqueue_compress_job(
                &cid,
                self.conv_id.clone(),
                lineage.clone(),
                uncovered_a as u32,
                uncovered_b as u32,
            )
            .map(|(id, created)| {
                self.ids.register_labeled("compress_job", id.as_str());
                (id, created)
            })
            .map_err(|e| e.to_string())
            .unwrap_or_else(|e| {
                self.push("chronicle_enqueue", "storage", e);
                (Id::from_str("none"), false)
            });
        self.push(
            "chronicle_enqueue",
            "ok",
            format!("created={created} job={}", job_id.as_str()),
        );
        // 幂等：同一 campaign 的 open job 已存在 → created=false。
        let (_, created_dup) = storage
            .enqueue_compress_job(
                &cid,
                self.conv_id.clone(),
                lineage.clone(),
                uncovered_a as u32,
                uncovered_b as u32,
            )
            .map_err(|e| e.to_string())
            .unwrap_or_else(|e| {
                self.push("chronicle_enqueue_dup", "storage", e);
                (Id::from_str("none"), true)
            });
        self.push(
            "chronicle_enqueue_dup",
            "ok",
            format!("created={created_dup}"),
        );
        // claim：Pending → Running；重复 claim 拒绝。
        let claimed = storage
            .claim_compress_job(&job_id)
            .map_err(|e| e.to_string())
            .unwrap_or_else(|e| {
                self.push("chronicle_claim", "storage", e);
                false
            });
        self.push("chronicle_claim", "ok", format!("claimed={claimed}"));
        let claimed_again = storage
            .claim_compress_job(&job_id)
            .map_err(|e| e.to_string())
            .unwrap_or_else(|e| {
                self.push("chronicle_claim_again", "storage", e);
                true
            });
        self.push(
            "chronicle_claim_again",
            "ok",
            format!("claimed={claimed_again}"),
        );
        // 成功：Running → Succeeded。
        match storage.succeed_compress_job(&job_id) {
            Ok(done) => self.push("chronicle_succeed", "ok", format!("done={done}")),
            Err(e) => self.push("chronicle_succeed", "storage", e.to_string()),
        }
        // 迟到成功/失败结果：job 已 Succeeded → 不得改写（双后端等价守卫）。
        match storage.succeed_compress_job(&job_id) {
            Ok(done) => self.push("chronicle_late_succeed", "ok", format!("done={done}")),
            Err(e) => self.push("chronicle_late_succeed", "storage", e.to_string()),
        }
        match storage.fail_or_retry_compress_job(&job_id, "late result") {
            Ok(done) => self.push("chronicle_late_fail", "ok", format!("done={done}")),
            Err(e) => self.push("chronicle_late_fail", "storage", e.to_string()),
        }
        // 失败重试：新 job → claim → fail → 未达 max_attempts 回 Pending。
        let (job2, created2) = storage
            .enqueue_compress_job(&cid, self.conv_id.clone(), lineage, 1, 0)
            .map(|(id, created)| {
                self.ids.register_labeled("compress_job", id.as_str());
                (id, created)
            })
            .map_err(|e| e.to_string())
            .unwrap_or_else(|e| {
                self.push("chronicle_enqueue_retry", "storage", e);
                (Id::from_str("none"), false)
            });
        self.push(
            "chronicle_enqueue_retry",
            "ok",
            format!("created={created2}"),
        );
        let _ = storage.claim_compress_job(&job2);
        match storage.fail_or_retry_compress_job(&job2, "injected compress failure") {
            Ok(retryable) => self.push(
                "chronicle_fail_retry",
                "ok",
                format!("retryable={retryable}"),
            ),
            Err(e) => self.push("chronicle_fail_retry", "storage", e.to_string()),
        }
        // 终态一览：id 规范化后按 (status, attempts) 汇总比较。
        let states = storage
            .list_compress_jobs()
            .map(|jobs| {
                let mut normalized: Vec<String> = jobs
                    .iter()
                    .map(|j| format!("{}:{}:{}", j.status, j.attempts, j.id.as_str()))
                    .collect();
                normalized.sort();
                normalized.join("|")
            })
            .map_err(|e| e.to_string())
            .unwrap_or_else(|e| {
                self.push("chronicle_list", "storage", e);
                String::new()
            });
        self.push("chronicle_list", "ok", states);

        // ── export/import round-trip ──────────────────────────────────
        let bundle = storage.export_campaign_bundle(&cid);
        let bundle_shape = bundle.as_ref().ok().map(|s| {
            let mut v: serde_json::Value =
                serde_json::from_str(s).unwrap_or(serde_json::json!(null));
            // 未被 op 返回的随机 id（postprocess 落库的知识条目/叙事任务等）按
            // 结构路径登记——双后端同构输出 → 同标签；未知 UUID 不得放行。
            // 先按后端无关语义键预排序（四.3：唯一标签需要跨后端确定的注册顺序）。
            stable_sort_collections(&mut v);
            self.ids.register_uuid_ids_by_path(&mut v, "bundle");
            self.ids.canonicalize(&mut v);
            canonicalize_timestamps(&mut v);
            // 集合数组顺序与后端 list_* 实现相关（非领域语义）→ 排序后比较。
            self.ids.sort_object_arrays(&mut v);
            serde_json::to_string(&v).unwrap_or_default()
        });
        let bundle_err = bundle
            .as_ref()
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        match &bundle_shape {
            Some(shape) => self.push(
                "export_bundle",
                "ok",
                format!("shape={}", short_hash(shape)),
            ),
            None => self.push("export_bundle", "storage", bundle_err),
        }
        let bundle_typed: Option<storyforge_lib::CampaignBundle> = bundle
            .as_ref()
            .ok()
            .and_then(|s| serde_json::from_str(s).ok());
        match bundle_typed {
            Some(bundle_value) => {
                match storage.import_campaign_bundle(bundle_value, &st.conv_store) {
                    Ok(result) => {
                        self.ids
                            .register_labeled("campaign", result.campaign_id.as_str());
                        self.ids
                            .register_labeled("conversation", result.conversation_id.as_str());
                        self.push(
                            "import_bundle",
                            "ok",
                            format!(
                                "instances={} knowledge={} tasks={}",
                                result.instance_count, result.knowledge_count, result.task_count
                            ),
                        );
                    }
                    Err(e) => self.push("import_bundle", "storage", e.to_string()),
                }
            }
            None => self.push("import_bundle", "storage", "bundle parse failed".into()),
        }

        // ── 重启恢复（三.2）：真实子进程 + 生产启动恢复入口 ────────────
        // 必须在 delete_campaign **之前**：植入需要引用仍存在的 Campaign。
        // 植入 Running 态 compress job + 崩溃残留（Turn 2 保持 DraftReady），
        // 子进程执行完整生产恢复；父进程按各后端自身契约断言子进程报告，
        // 随后**重开** AppState（JSON 重读磁盘 / SQLite 读权威表）继续。
        self.run_restart_recovery();

        // 删除前的完整领域快照（Turn/Attempt/知识/任务/总结/会话/世界书 +
        // 恢复后的 compress jobs/outbox/恢复状态）。
        // ⚠️ 必须在 delete_campaign **之前**捕获——否则级联删掉的数据不在快照里，
        // "删除前等价"就成了删除后等价的假阳性（审查跟进 P1）。
        let snapshot_before_delete = self.snapshot().expect("snapshot before delete");

        // ── 删除（delete_campaign 级联；二次删除 → not_found）─────────
        let cid_now = self.campaign_id.clone().expect("campaign");
        match storyforge_lib::delete_campaign(
            cid_now.as_str().to_string(),
            tauri_state_for_test(&self.state),
        ) {
            Ok(()) => {
                self.deleted_campaign = true;
                self.push("delete_campaign", "ok", "ok".into());
            }
            Err(e) => self.push("delete_campaign", classify(&e), e.to_string()),
        }
        match storyforge_lib::delete_campaign(
            cid_now.as_str().to_string(),
            tauri_state_for_test(&self.state),
        ) {
            Ok(()) => self.push("delete_campaign_again", "ok", "unexpected".into()),
            Err(e) => self.push("delete_campaign_again", classify(&e), e.to_string()),
        }

        let snapshot = self.snapshot().expect("snapshot after delete");
        let pipeline_events = std::mem::take(&mut self.pipeline_events);
        let mut results = std::mem::take(&mut self.results);
        let ids = std::mem::take(&mut self.ids);
        for record in &mut results {
            let mut v = serde_json::json!(record.message);
            ids.canonicalize(&mut v);
            canonicalize_timestamps(&mut v);
            record.message = v.as_str().unwrap_or("").to_string();
        }
        PhaseOutput {
            results,
            snapshot,
            snapshot_before_delete,
            pipeline_events,
        }
    }

    /// 三.2 真实「重启恢复」：植入崩溃残留 → 启动**真实子进程**（同一测试
    /// 二进制 `restart_child_entry`，环境变量门控）执行生产启动恢复 →
    /// 父进程按各后端自身恢复契约断言子进程报告 → 二次恢复幂等 →
    /// 父进程**重开** AppState（后续断言读重开后的状态，不读旧内存）。
    ///
    /// 恢复契约（各后端自身语义，等价不变量=恢复后无非终态 Turn）：
    /// - JSON：Generating/DraftReady 活动 Turn → Failed；Committing → 重放。
    /// - SQLite：fail_incomplete 把非终态 Turn/Attempt → Failed（原子 accept
    ///   下 Committing 罕见）。
    /// - 两者：Running compress job → Pending；待 accept outbox 清零。
    #[allow(clippy::too_many_lines)]
    fn run_restart_recovery(&mut self) {
        let storage = self.state.storage();

        // 植入：Running compress job（崩溃中断的 worker 残留）。
        let (recovery_campaign, recovery_conv) = match (&self.campaign_id, &self.conv_id) {
            (Some(c), Some(v)) => (c.clone(), v.clone()),
            _ => {
                self.push(
                    "restart_recovery",
                    "storage",
                    "no campaign for restart".into(),
                );
                return;
            }
        };
        let (job_id, _) = match storage.enqueue_compress_job(
            &recovery_campaign,
            Some(recovery_conv.clone()),
            None,
            1,
            0,
        ) {
            Ok(x) => {
                self.ids.register_labeled("compress_job", x.0.as_str());
                x
            }
            Err(e) => {
                self.push("restart_recovery", "storage", format!("enqueue: {e}"));
                return;
            }
        };
        if !storage.claim_compress_job(&job_id).unwrap_or(false) {
            self.push(
                "restart_recovery",
                "storage",
                "could not claim seed job".into(),
            );
            return;
        }
        // 断言 seed 处于 Running（双后端一致）。
        let running_before = storage
            .list_compress_jobs()
            .map(|jobs| jobs.iter().any(|j| j.id == job_id && j.status == "running"))
            .unwrap_or(false);
        if !running_before {
            self.push("restart_recovery", "storage", "seed job not running".into());
            return;
        }

        let data_dir = storage.data_dir().to_path_buf();
        let turn_ids: Vec<String> = self
            .turn_ids
            .iter()
            .map(|id| id.as_str().to_string())
            .collect();

        // ── 启动真实子进程：完整生产 bootstrap + 恢复 ─────────────────
        let exe = std::env::current_exe().expect("test binary path");
        let mut cmd = std::process::Command::new(exe);
        cmd.args(["restart_child_entry", "--exact", "--nocapture"]);
        cmd.env("STORYFORGE_RESTART_CHILD", "1");
        cmd.env("STORYFORGE_RESTART_DATA_DIR", &data_dir);
        cmd.env(
            "STORYFORGE_RESTART_BACKEND",
            if self.sqlite { "sqlite" } else { "json" },
        );
        cmd.env("STORYFORGE_RESTART_TURN_IDS", turn_ids.join(","));
        let output = match cmd.output() {
            Ok(out) => out,
            Err(e) => {
                self.push(
                    "restart_recovery",
                    "storage",
                    format!("spawn restart child: {e}"),
                );
                return;
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let report_line = stdout
            .lines()
            .find(|line| line.starts_with("STORYFORGE_RESTART_REPORT "))
            .map(|line| {
                line.trim_start_matches("STORYFORGE_RESTART_REPORT ")
                    .to_string()
            });
        let report: RestartReport = match report_line {
            Some(line) => match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    self.push(
                        "restart_recovery",
                        "storage",
                        format!("child report parse failed: {e}\nstdout={stdout}\nstderr={stderr}"),
                    );
                    return;
                }
            },
            None => {
                self.push(
                    "restart_recovery",
                    "storage",
                    format!(
                        "child exited {} without report\nstdout={stdout}\nstderr={stderr}",
                        output.status
                    ),
                );
                return;
            }
        };
        if !output.status.success() {
            self.push(
                "restart_recovery",
                "storage",
                format!("child failed: {stderr}"),
            );
            return;
        }

        // ── 按各后端自身恢复契约断言（原始口径，不做跨后端归一化）────
        let expected_backend = if self.sqlite { "sqlite" } else { "json" };
        assert_eq!(
            report.backend, expected_backend,
            "子进程后端决议必须与父进程一致"
        );
        // 等价不变量：恢复后不得残留非终态 Turn。
        assert_eq!(
            report.turns_non_terminal, 0,
            "恢复后不得有非终态 Turn（各后端自身契约）: {report:?}"
        );
        assert!(
            report.turns_failed >= 1,
            "崩溃残留 Turn（DraftReady）必须被各后端终态化为 Failed: {report:?}"
        );
        // 二次恢复幂等：Turn 列表逐项不变 + compress reset 为 0。
        assert_eq!(
            report.turns.len(),
            report.turns_after_second_recovery.len(),
            "二次恢复不得改变 Turn 集合: {report:?}"
        );
        for (a, b) in report
            .turns
            .iter()
            .zip(report.turns_after_second_recovery.iter())
        {
            assert_eq!(
                (a.id.as_str(), a.status.as_str()),
                (b.id.as_str(), b.status.as_str()),
                "二次恢复后 Turn 状态必须不变（幂等）: {report:?}"
            );
        }
        // 三审9：重置现在在 `run_startup_recovery` 内部完成（公共入口统一处理），
        // reset 探针在恢复之后读取 → 计数可能为 0（已被内部重置）。真正的不变式是
        // 「压缩任务不得停留在 Running」，由下方断言保证。
        assert!(
            report
                .compress_jobs
                .iter()
                .all(|j| j.status == "pending" || j.status == "succeeded" || j.status == "failed"),
            "压缩任务不得停留在 Running: {report:?}"
        );
        assert_eq!(
            report.outbox_pending, 0,
            "恢复后待 accept 的 outbox 必须清零（各后端原始口径）: {report:?}"
        );
        // 活跃 Campaign：JSON 从 active_campaign.json 恢复（父进程设置过指针）；
        // SQLite 为进程内指针，重启后为 null（后端自身语义）。
        if self.sqlite {
            assert!(
                report.active_campaign.is_none(),
                "SQLite 活跃指针是进程内状态，重启子进程必须为 null: {report:?}"
            );
        } else {
            assert_eq!(
                report.active_campaign.as_deref(),
                Some(recovery_campaign.as_str()),
                "JSON 活跃指针必须跨真实重启存活: {report:?}"
            );
        }

        self.push(
            "restart_recovery",
            "ok",
            format!(
                "reset={} second_reset={} non_terminal={} turns_failed={} outbox={}",
                report.compress_reset_first,
                report.compress_reset_second,
                report.turns_non_terminal,
                report.turns_failed,
                report.outbox_pending
            ),
        );

        // ── 重开 AppState：后续断言读重开后的状态（不读旧内存）───────
        let reopened_storage = Arc::new(StorageFacade::new(
            data_dir.clone(),
            PinnedBackend::new(
                if self.sqlite {
                    StorageBackend::Sqlite
                } else {
                    StorageBackend::Json
                },
                if self.sqlite {
                    BackendSource::Env
                } else {
                    BackendSource::Default
                },
            ),
        ));
        if self.sqlite {
            reopened_storage
                .validate_runtime_authority()
                .expect("reopened facade must match the still-active SQLite runtime");
        }
        let reopened_state = match AppState::new_with_backend(data_dir, reopened_storage) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                self.push(
                    "restart_recovery",
                    "storage",
                    format!("reopen AppState: {e}"),
                );
                return;
            }
        };
        // 断言重开后的状态确实反映了子进程恢复结果（Turn 2 → Failed）。
        let recovered_turn_statuses: Vec<String> = self
            .turn_ids
            .iter()
            .map(|tid| {
                reopened_state
                    .storage()
                    .get_turn(tid)
                    .ok()
                    .flatten()
                    .map(|t| format!("{:?}", t.status))
                    .unwrap_or_else(|| "missing".to_string())
            })
            .collect();
        assert!(
            recovered_turn_statuses.iter().any(|s| s == "Failed"),
            "重开后的状态必须反映子进程恢复（存在 Failed Turn）: {recovered_turn_statuses:?}"
        );
        self.state = reopened_state;
    }

    /// 规范化领域快照（从驱动跟踪的 id + storage 读取）。
    ///
    /// 四.1：**任何读取失败直接返回 Err**（缺失文件/表不得静默成空数组）；
    /// 快照覆盖：卡片/角色库/全部 Campaign（含其 instances/knowledge/tasks/
    /// summaries/world_info）/Turn/会话/MVU + compress jobs + outbox +
    /// 重启恢复状态。归一化管线：未登记随机 id 按结构路径登记（唯一标签）→
    /// 已知 id 换 ⟨label⟩ → 时间戳换 ⟨ts⟩ → 对象数组排序。
    fn snapshot(&mut self) -> Result<serde_json::Value, String> {
        let storage = self.state.storage();

        let mut cards: Vec<serde_json::Value> = storage
            .list_cards()
            .map_err(|e| format!("snapshot: list_cards: {e}"))?
            .iter()
            .map(|c| serde_json::to_value(c).map_err(|e| format!("snapshot: card: {e}")))
            .collect::<Result<_, _>>()?;
        for card in &mut cards {
            if let Some(map) = card.as_object_mut() {
                map.remove("imported_at");
            }
        }

        let campaigns = storage
            .list_campaigns(None)
            .map_err(|e| format!("snapshot: list_campaigns: {e}"))?;

        // 全部 Campaign 的聚合数据（四.1：不得只取跟踪的 campaign）。
        let mut instances = Vec::new();
        let mut knowledge = Vec::new();
        let mut tasks = Vec::new();
        let mut summaries = Vec::new();
        let mut world_info = Vec::new();
        for record in &campaigns {
            let cid = &record.campaign.id;
            for inst in storage
                .list_instances(cid)
                .map_err(|e| format!("snapshot: list_instances {cid}: {e}"))?
            {
                instances.push(
                    serde_json::to_value(&inst).map_err(|e| format!("snapshot: instance: {e}"))?,
                );
            }
            for k in storage
                .list_knowledge(cid)
                .map_err(|e| format!("snapshot: list_knowledge {cid}: {e}"))?
            {
                knowledge.push(
                    serde_json::to_value(&k).map_err(|e| format!("snapshot: knowledge: {e}"))?,
                );
            }
            for t in storage
                .list_tasks(cid)
                .map_err(|e| format!("snapshot: list_tasks {cid}: {e}"))?
            {
                tasks.push(serde_json::to_value(&t).map_err(|e| format!("snapshot: task: {e}"))?);
            }
            for s in storage
                .list_summaries(cid)
                .map_err(|e| format!("snapshot: list_summaries {cid}: {e}"))?
            {
                summaries
                    .push(serde_json::to_value(&s).map_err(|e| format!("snapshot: summary: {e}"))?);
            }
            world_info.push(
                serde_json::to_value(
                    &storage
                        .get_world_info(cid)
                        .map_err(|e| format!("snapshot: get_world_info {cid}: {e}"))?,
                )
                .map_err(|e| format!("snapshot: world_info: {e}"))?,
            );
        }

        let mut turns = Vec::new();
        for tid in &self.turn_ids {
            let turn = storage
                .get_turn(tid)
                .map_err(|e| format!("snapshot: get_turn {tid}: {e}"))?;
            let Some(turn) = turn else {
                if self.deleted_campaign {
                    // delete_campaign 级联删除后的合法缺失（等价性由另一侧对比保证）。
                    continue;
                }
                return Err(format!("snapshot: turn {tid} 缺失（不允许静默跳过）"));
            };
            // 四.2：**不再归一化** pending_state_changes.status——JSON 与 SQLite
            // 提交后持久化的原始状态必须逐字节等价（"committed"），归一化只会
            // 掩盖分叉。
            turns.push(serde_json::to_value(&turn).map_err(|e| format!("snapshot: turn: {e}"))?);
        }

        let mut convs = Vec::new();
        if let Some(conv_id) = &self.conv_id {
            let conv = self.state.conv_store.get(conv_id);
            if let Some(conv) = conv {
                convs.push(
                    serde_json::to_value(&conv)
                        .map_err(|e| format!("snapshot: conversation: {e}"))?,
                );
            } else if !self.deleted_campaign {
                return Err(format!("snapshot: conversation {conv_id} 缺失"));
            }
        }

        let chars: Vec<serde_json::Value> = storage
            .list_characters()
            .map_err(|e| format!("snapshot: list_characters: {e}"))?
            .iter()
            .map(|c| serde_json::to_value(&c.info).map_err(|e| format!("snapshot: char: {e}")))
            .collect::<Result<_, _>>()?;

        let mvu: Vec<serde_json::Value> = storage
            .list_mvu()
            .map_err(|e| format!("snapshot: list_mvu: {e}"))?
            .iter()
            .map(|m| serde_json::to_value(m).map_err(|e| format!("snapshot: mvu: {e}")))
            .collect::<Result<_, _>>()?;

        // compress jobs（四.1：快照必须覆盖任务队列）。
        let jobs: Vec<serde_json::Value> = storage
            .list_compress_jobs()
            .map_err(|e| format!("snapshot: list_compress_jobs: {e}"))?
            .iter()
            .map(|j| {
                serde_json::json!({
                    "job_id": j.id.as_str(),
                    "status": j.status,
                    "attempts": j.attempts,
                })
            })
            .collect();

        // outbox（四.1）：待 accept 的 pending 单位数。
        // JSON：attempt.pending_state_changes 且未 Committed；SQLite：outbox
        // 行未 Applied。快照存原始计数——各后端自身语义（等价不变量=两者都为 0）。
        let mut outbox_pending = 0usize;
        for tid in &self.turn_ids {
            let turn = storage
                .get_turn(tid)
                .map_err(|e| format!("snapshot: outbox get_turn {tid}: {e}"))?;
            let Some(turn) = turn else {
                if self.deleted_campaign {
                    continue;
                }
                return Err(format!("snapshot: outbox turn {tid} 缺失"));
            };
            if self.sqlite {
                let rows = sqlite_runtime::list_preaccept_outbox_for_turn(tid)
                    .map_err(|e| format!("snapshot: outbox {tid}: {e}"))?;
                // 只计仍待 accept 消费的 Pending 行；Applied/Skipped/Failed 是
                // 终态台账（Skipped = 无操作标记，与 JSON 不产生行等价）。
                outbox_pending += rows
                    .iter()
                    .filter(|r| {
                        matches!(
                            r.status,
                            storyforge_infra_sqlite::preaccept::PreacceptOutboxStatus::Pending
                        )
                    })
                    .count();
            } else {
                outbox_pending += turn
                    .attempts
                    .iter()
                    .filter(|a| {
                        a.pending_state_changes.as_ref().is_some_and(|b| {
                            b.status != storyforge_domain::turn::MutationBatchStatus::Committed
                        })
                    })
                    .count();
            }
        }

        // 重启恢复状态（四.1）：恢复后无非终态 Turn 的等价不变量。
        let recovery_non_terminal = {
            let mut n = 0usize;
            for tid in &self.turn_ids {
                let turn = storage
                    .get_turn(tid)
                    .map_err(|e| format!("snapshot: recovery get_turn {tid}: {e}"))?;
                if let Some(turn) = turn
                    && !turn.status.is_terminal()
                {
                    n += 1;
                }
            }
            n
        };

        let mut snap = serde_json::Map::new();
        snap.insert("cards".into(), serde_json::Value::Array(cards));
        snap.insert(
            "campaigns".into(),
            serde_json::Value::Array(
                campaigns
                    .iter()
                    .map(|r| {
                        serde_json::to_value(&r.campaign)
                            .map_err(|e| format!("snapshot: campaign: {e}"))
                    })
                    .collect::<Result<_, _>>()?,
            ),
        );
        snap.insert("instances".into(), serde_json::Value::Array(instances));
        snap.insert("knowledge".into(), serde_json::Value::Array(knowledge));
        snap.insert("tasks".into(), serde_json::Value::Array(tasks));
        snap.insert("summaries".into(), serde_json::Value::Array(summaries));
        snap.insert("world_info".into(), serde_json::Value::Array(world_info));
        snap.insert("turns".into(), serde_json::Value::Array(turns));
        snap.insert("conversations".into(), serde_json::Value::Array(convs));
        snap.insert("characters".into(), serde_json::Value::Array(chars));
        snap.insert("mvu".into(), serde_json::Value::Array(mvu));
        snap.insert("jobs".into(), serde_json::Value::Array(jobs));
        snap.insert("outbox_pending".into(), serde_json::json!(outbox_pending));
        snap.insert(
            "recovery_non_terminal".into(),
            serde_json::json!(recovery_non_terminal),
        );

        let mut value = serde_json::Value::Object(snap);
        // 四.3：集合数组先按后端无关语义键预排序，再登记路径 id——双后端
        // list_* 顺序不同不会导致同一实体获得不同标签。
        stable_sort_collections(&mut value);
        self.ids.register_uuid_ids_by_path(&mut value, "snap");
        self.ids.canonicalize(&mut value);
        canonicalize_timestamps(&mut value);
        normalize_recovery_reasons(&mut value);
        self.ids.sort_object_arrays(&mut value);
        Ok(value)
    }
}

fn short_hash(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn json_app_state(dir: &Path) -> Arc<AppState> {
    let storage = Arc::new(StorageFacade::new(
        dir.to_path_buf(),
        PinnedBackend::new(StorageBackend::Json, BackendSource::Default),
    ));
    Arc::new(AppState::new_with_backend(dir.to_path_buf(), storage).expect("JSON AppState"))
}

fn compare_phase_outputs(json_out: &PhaseOutput, sqlite_out: &PhaseOutput) {
    // ── 比较：操作结果 ────────────────────────────────────────────────
    assert_eq!(
        json_out.results.len(),
        sqlite_out.results.len(),
        "op 序列长度必须一致\nJSON:   {:#?}\nSQLite: {:#?}",
        json_out.results,
        sqlite_out.results
    );
    for (j, s) in json_out.results.iter().zip(sqlite_out.results.iter()) {
        assert_eq!(
            (j.name, j.kind, j.message.as_str()),
            (s.name, s.kind, s.message.as_str()),
            "op 结果必须等价（成功/失败/错误类别/规范化消息）"
        );
    }

    // ── 比较：规范化领域快照（四.1：任何读取失败已在 snapshot() 内 Err）──
    assert_eq!(
        json_out.snapshot, sqlite_out.snapshot,
        "JSON 与 SQLite 的规范化领域快照必须等价（删除后，含 jobs/outbox/recovery）"
    );
    assert_eq!(
        json_out.snapshot_before_delete, sqlite_out.snapshot_before_delete,
        "JSON 与 SQLite 的规范化领域快照必须等价（删除前，含 Turn/Attempt/知识/任务/总结）"
    );

    // ── 比较：关键 Pipeline 事件序列（三.3）────────────────────────────
    assert_eq!(
        json_out.pipeline_events, sqlite_out.pipeline_events,
        "JSON 与 SQLite 的关键 Pipeline 事件序列必须等价（经真实 helper 派生）"
    );
}

/// 主测试：同一 op 序列跑 JSON 与 SQLite，比较操作结果 + 领域快照 + 事件序列。
///
/// 四.3 突变阶段嵌在 JSON 与 SQLite 之间：JSON AppState 拒绝与活动 SQLite
/// runtime 共存（fail-closed），任何 JSON 阶段必须在 sqlite_runtime::activate
/// **之前**完成——独立突变测试无法保证与主测试的执行顺序，因此并入主测试。
#[tokio::test]
async fn backend_parity_equivalent_domain_snapshots() {
    // ── JSON 真值阶段 ────────────────────────────────────────────────
    let json_dir = tempfile::tempdir().unwrap();
    write_fixture(json_dir.path());
    let json_state = json_app_state(json_dir.path());
    let mut json_driver = ParityDriver::new(json_state, false);
    let json_out = json_driver.run(None).await;

    // ── 四.3 突变阶段：同一 op 序列注入数据损坏（JSON 后端，SQLite 未激活）──
    // 交换 accepted_attempt_id / variant 值后，删除前快照必须与真值不等——
    // IdRegistry 唯一标签（⟨label:N⟩）不得掩盖差异；旧实现把所有 id 映射成
    // 同一标签，该断言会 panic（判别点）。
    for (kind, name) in [
        (TamperKind::SwapAcceptedAttempt, "accepted_attempt"),
        (TamperKind::SwapVariant, "variant"),
    ] {
        let dir_mutant = tempfile::tempdir().unwrap();
        write_fixture(dir_mutant.path());
        let state_mutant = json_app_state(dir_mutant.path());
        let mut driver_mutant = ParityDriver::new(state_mutant, false);
        let out_mutant = driver_mutant.run(Some(kind)).await;
        assert_ne!(
            json_out.snapshot_before_delete, out_mutant.snapshot_before_delete,
            "突变（{name}）后删除前快照必须不等——IdRegistry 不得掩盖该差异"
        );
        assert_eq!(
            json_out.results.len(),
            out_mutant.results.len(),
            "突变（{name}）不得改变 op 序列长度"
        );
    }

    // ── 四.1：缺失实体必须使 snapshot 失败（不得静默成空数组）────────
    // 独立驱动：追踪一个不存在的 Turn——snapshot() 必须返回 Err。
    {
        let dir_missing = tempfile::tempdir().unwrap();
        write_fixture(dir_missing.path());
        let state_missing = json_app_state(dir_missing.path());
        let mut driver_missing = ParityDriver::new(state_missing, false);
        driver_missing.turn_ids.push(Id::from_str("phantom-turn"));
        let result = driver_missing.snapshot();
        assert!(
            result.is_err(),
            "缺失 Turn 必须使 snapshot 失败——不得静默跳过/成空数组"
        );
        assert!(
            result.unwrap_err().contains("缺失"),
            "错误信息必须指明缺失实体"
        );
    }

    // ── SQLite 阶段：同一 fixture cutover → 激活 → 同一 op 序列 ──────
    let sqlite_dir = tempfile::tempdir().unwrap();
    write_fixture(sqlite_dir.path());
    let db_path = sqlite_dir.path().join("storyforge.sqlite3");
    let cutover = CutoverRequest {
        plan: CutoverPlan::new(sqlite_dir.path(), &db_path),
        label: "gate5-parity".into(),
    };
    match recover_or_verify(&cutover).expect("cutover") {
        CutoverOutcome::Completed(report) => {
            assert_eq!(report.cards, 1);
            assert_eq!(report.characters, 2);
            assert_eq!(report.campaigns, 0);
        }
        other => panic!("cutover must complete, got {other:?}"),
    }
    sqlite_runtime::activate(&db_path).expect("activate sqlite");
    let sqlite_storage = Arc::new(StorageFacade::new(
        sqlite_dir.path().to_path_buf(),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    ));
    sqlite_storage
        .validate_runtime_authority()
        .expect("facade/runtime authority");
    let sqlite_state = Arc::new(
        AppState::new_with_backend(sqlite_dir.path().to_path_buf(), sqlite_storage)
            .expect("SQLite AppState"),
    );
    let mut sqlite_driver = ParityDriver::new(sqlite_state, true);
    let sqlite_out = sqlite_driver.run(None).await;

    compare_phase_outputs(&json_out, &sqlite_out);
}

// ─── 三.2 重启子进程入口 ────────────────────────────────────────────────
//
// 环境变量门控；无 env 时直接返回（正常 cargo test 下的 no-op pass）。
// 有 env 时执行**完整生产启动恢复**：resolve_backend → StorageFacade →
// AppState::new_with_backend → recover_turns → recover_compress_jobs，
// 等待压缩 worker 收敛后输出单行 JSON 报告并退出 0。

fn read_reported_turns(state: &AppState, turn_ids: &[String]) -> Vec<ReportTurn> {
    let mut out = Vec::new();
    for id in turn_ids {
        if let Ok(Some(turn)) = state.storage().get_turn(&Id::from_str(id)) {
            out.push(ReportTurn {
                id: id.clone(),
                status: format!("{:?}", turn.status).to_lowercase(),
                attempts: turn
                    .attempts
                    .iter()
                    .map(|a| ReportAttempt {
                        id: a.attempt_id.as_str().to_string(),
                        status: format!("{:?}", a.status).to_lowercase(),
                    })
                    .collect(),
            });
        }
    }
    out
}

fn read_reported_outbox(
    state: &AppState,
    turn_ids: &[String],
    sqlite: bool,
) -> Result<usize, String> {
    let mut pending = 0usize;
    for id in turn_ids {
        let turn = state
            .storage()
            .get_turn(&Id::from_str(id))
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("turn {id} missing"))?;
        if sqlite {
            let rows = sqlite_runtime::list_preaccept_outbox_for_turn(&Id::from_str(id))
                .map_err(|e| e.to_string())?;
            // 只计仍待 accept 消费的 Pending 行（Skipped/Applied/Failed 为终态台账）。
            pending += rows
                .iter()
                .filter(|r| {
                    matches!(
                        r.status,
                        storyforge_infra_sqlite::preaccept::PreacceptOutboxStatus::Pending
                    )
                })
                .count();
        } else {
            pending += turn
                .attempts
                .iter()
                .filter(|a| {
                    a.pending_state_changes.as_ref().is_some_and(|b| {
                        b.status != storyforge_domain::turn::MutationBatchStatus::Committed
                    })
                })
                .count();
        }
    }
    Ok(pending)
}

#[tokio::test]
async fn restart_child_entry() {
    if std::env::var("STORYFORGE_RESTART_CHILD").ok().as_deref() != Some("1") {
        // 正常测试运行：no-op。
        return;
    }
    let data_dir = std::path::PathBuf::from(
        std::env::var("STORYFORGE_RESTART_DATA_DIR").expect("restart child requires data dir"),
    );
    let backend = std::env::var("STORYFORGE_RESTART_BACKEND").expect("restart backend");
    let turn_ids: Vec<String> = std::env::var("STORYFORGE_RESTART_TURN_IDS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    // ── 完整生产 bootstrap ───────────────────────────────────────────
    let resolution = storyforge_lib::storage_backend::resolve_backend(&data_dir)
        .expect("resolve_backend must succeed in restart child");
    assert_eq!(
        resolution.diagnostics.backend,
        if backend == "sqlite" {
            "sqlite"
        } else {
            "json"
        },
        "marker-first 决议必须与父进程阶段一致"
    );
    let storage = Arc::new(StorageFacade::new(
        data_dir.clone(),
        resolution.pinned.clone(),
    ));
    if let Some(db_path) = resolution.db_path.as_deref() {
        sqlite_runtime::activate(db_path).expect("child sqlite activation");
    }
    let state =
        Arc::new(AppState::new_with_backend(data_dir, storage.clone()).expect("child AppState"));

    // ── 生产启动恢复第 1 轮（单一公共入口）──────────────────────────────
    // 三审9：子进程与生产 setup hook 共用 `startup_recovery::run_startup_recovery`
    // （内部：turn 重放 + compress Running→Pending + spawn worker），不再各自内联。
    storyforge_lib::startup_recovery::run_startup_recovery(&state);
    let reset_first = storage
        .reset_running_compress_jobs_to_pending()
        .expect("child compress reset");

    // 等待压缩 worker 收敛（无 LLM → 快速失败回队；job 最终停在 Pending）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let any_running = storage
            .list_compress_jobs()
            .expect("list jobs")
            .iter()
            .any(|j| j.status == "running");
        if !any_running || std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // ── 二次恢复（幂等证明）──────────────────────────────────────────
    let turns_after_first = read_reported_turns(&state, &turn_ids);
    // 三审9：二次恢复走同一公共入口（幂等：Running→Pending 已迁移、Committing 已重放）。
    storyforge_lib::startup_recovery::run_startup_recovery(&state);
    let reset_second = storage
        .reset_running_compress_jobs_to_pending()
        .expect("second compress reset");
    let turns_after_second = read_reported_turns(&state, &turn_ids);

    let turns_non_terminal = turns_after_first
        .iter()
        .filter(|t| {
            !matches!(
                t.status.as_str(),
                "committed" | "failed" | "degraded" | "abandoned"
            )
        })
        .count();
    let turns_failed = turns_after_first
        .iter()
        .filter(|t| t.status == "failed")
        .count();
    let compress_jobs = storage
        .list_compress_jobs()
        .expect("list jobs")
        .into_iter()
        .map(|j| ReportJob {
            id: j.id.as_str().to_string(),
            status: j.status.clone(),
            attempts: j.attempts,
        })
        .collect();
    let outbox_pending =
        read_reported_outbox(&state, &turn_ids, backend == "sqlite").expect("outbox count");
    let active_campaign = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .map(|id| id.as_str().to_string());

    let report = RestartReport {
        backend: backend.clone(),
        turns: turns_after_first,
        turns_after_second_recovery: turns_after_second,
        turns_non_terminal,
        turns_failed,
        compress_jobs,
        compress_reset_first: reset_first,
        compress_reset_second: reset_second,
        outbox_pending,
        active_campaign,
    };
    println!(
        "STORYFORGE_RESTART_REPORT {}",
        serde_json::to_string(&report).expect("serialize restart report")
    );
}
