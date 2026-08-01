//! Gate 5 后端等价套件：同一组输入分别跑在 JSON 与 SQLite 上，比较
//! 操作结果（成功/失败/错误类别）与规范化领域快照。
//!
//! `sqlite_runtime::activate` 是进程全局 OnceLock，JSON facade 拒绝与活动
//! SQLite runtime 共存——因此本二进制在**同一进程内先完整跑 JSON 阶段**（无需
//! activation），随后激活 SQLite 并跑同一 op 序列，最后统一比较（与既有
//! sqlite_* 集成测试的单 authority 约束一致）。
//!
//! 比较规则（PLAN §10.3）：
//! - 领域快照：卡片/角色库/Campaign/实例/知识/任务/总结/Turn/会话/世界书/MVU，
//!   经 id 规范（已知 id → ⟨label⟩）与时间戳规范（→ ⟨ts⟩）后逐项相等；
//!   SQLite-native 台账（outbox/mutation_commits）为物理布局差异，显式排除。
//! - 操作结果：op 名 + 成功/失败类别 + 规范化消息。

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
                "spec_version": "2.0", "extensions": {},
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
#[derive(Debug, Default, Clone)]
struct IdRegistry {
    ids: Vec<(String, String)>, // (label, actual id)
}

impl IdRegistry {
    fn register(&mut self, label: &str, id: &str) {
        if !self.ids.iter().any(|(_, actual)| actual == id) {
            self.ids.push((label.to_string(), id.to_string()));
        }
    }
    fn contains_actual(&self, id: &str) -> bool {
        self.ids.iter().any(|(_, actual)| actual == id)
    }
    /// 把结构内尚未登记的随机 UUID id 按字段路径登记（如 `tasks[].id`，数组无
    /// 下标——instances/tasks 等是集合语义，双后端顺序不同（见 list_* 实现），
    /// 标签必须与顺序无关；内容差异仍显式可见）。
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
                        self.register(&format!("path:{child}"), s);
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

/// 归一化 Turn/Attempt 的 `pending_state_changes.status` 字段。
///
/// `MutationBatchStatus`（Prepared/Applying/Committed）是**执行态标志**，不是领域
/// 语义：JSON postprocess 后 batch 暂存为 Prepared（待 accept 落库），SQLite 的
/// preaccept UoW 已把 batch 落进 outbox 并标 Committed（同样待 accept）。两者 batch
/// 内容（mutations/commit_id/expected/target_revision）相同，该 flag 仅随执行进度变化
/// 且双后端在不同阶段合法不同——按归一化规则统一替换为 ⟨batch_status⟩，领域差异仍
/// 显式可见。适用于 turns[] 以及 bundle（export 产物）里携带的同类节点。
fn normalize_pending_batch_status(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_pending_batch_status(item);
            }
        }
        serde_json::Value::Object(map) => {
            // 仅命中 MutationBatch 形态的对象（同时含 commit_id + mutations + status），
            // 绝不误伤 Turn/Attempt 自身的领域 status（它们的对象没有 mutations/commit_id）。
            let is_batch = map.contains_key("commit_id")
                && map.contains_key("mutations")
                && map.contains_key("expected_revision");
            if is_batch
                && let Some(status) = map.get_mut("status")
                && matches!(
                    status.as_str(),
                    Some("prepared") | Some("applying") | Some("committed")
                )
            {
                *status = serde_json::json!("⟨batch_status⟩");
            }
            for (_k, v) in map.iter_mut() {
                normalize_pending_batch_status(v);
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
struct PhaseOutput {
    results: Vec<OpRecord>,
    snapshot: serde_json::Value,
    /// 删除前的领域快照（含 Turn/Attempt/知识/任务/总结等删除级联会清掉的数据）。
    snapshot_before_delete: serde_json::Value,
    /// 关键 Pipeline 事件序列（postprocess Done/Failed/Skipped + 计数/原因），
    /// 按 production 后处理契约（runtime_support）从 `apply_outcome` 结果派生。
    /// 双后端输入相同 → 事件序列必须逐项相等（PLAN §10.4）。
    pipeline_events: Vec<String>,
}

// ─── 驱动 ──────────────────────────────────────────────────────────────

struct ParityDriver<'a> {
    state: &'a Arc<AppState>,
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
}

impl<'a> ParityDriver<'a> {
    fn new(state: &'a Arc<AppState>, sqlite: bool) -> Self {
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
        }
    }

    fn push(&mut self, name: &'static str, kind: &'static str, message: String) {
        self.results.push(OpRecord {
            name,
            kind,
            message,
        });
    }

    /// 按 production 后处理事件契约（runtime_support.rs 同款）记录关键 Pipeline 事件：
    /// applied → PostProcessDone{k,v,t}；skipped（cancel）→ PostProcessSkipped{reason}。
    /// 这是双后端输入相同时**真正可比较**的事件序列——apply_outcome 是共享写回逻辑，
    /// 事件只由其结果驱动，双后端结果结构相同 → 事件序列逐项相等。
    fn record_postprocess_events(&mut self, result: &pp::ProductionPostprocessResult) {
        if result.applied {
            let (knowledge_count, variable_count, task_count) = result
                .outcome
                .as_ref()
                .and_then(|o| o.post_process.as_ref())
                .map(|pp| {
                    (
                        pp.knowledge_updates.len(),
                        pp.variable_updates.len(),
                        pp.task_updates.len(),
                    )
                })
                .unwrap_or((0, 0, 0));
            self.pipeline_events.push(format!(
                "PostProcessDone:k={knowledge_count},v={variable_count},t={task_count}"
            ));
        } else if let Some(reason) = result.skipped_reason.as_deref() {
            self.pipeline_events
                .push(format!("PostProcessSkipped:{reason}"));
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
            if let Ok(instances) = self.state.storage().list_instances(cid) {
                for inst in &instances {
                    self.ids.register("instance", inst.id.as_str());
                    if !self.instance_ids.contains(&inst.id) {
                        self.instance_ids.push(inst.id.clone());
                    }
                }
            }
            if let Ok(tasks) = self.state.storage().list_tasks(cid) {
                for task in &tasks {
                    self.ids.register("task", task.id.as_str());
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
    async fn run(&mut self) -> PhaseOutput {
        let st = self.state;
        let storage = st.storage();

        // ── create：Campaign lifecycle ─────────────────────────────────
        match storyforge_lib::create_campaign(
            "card-1".to_string(),
            "等价局".to_string(),
            Some("你好，冒险者。".to_string()),
            tauri_state_for_test(st),
        ) {
            Ok(dto) => {
                self.campaign_id = Some(Id::from_str(&dto.id));
                self.conv_id = dto.conversation_id.as_ref().map(Id::from_str);
                self.ids.register("campaign", &dto.id);
                if let Some(conv) = &dto.conversation_id {
                    self.ids.register("conversation", conv);
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

        // definition 重复加入 → validation（Bob 已被 create_campaign 实例化）
        match storyforge_lib::add_campaign_instance(
            cid.as_str().to_string(),
            Some("def-bob".to_string()),
            None,
            None,
            None,
            tauri_state_for_test(st),
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
            tauri_state_for_test(st),
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
            tauri_state_for_test(st),
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
                tauri_state_for_test(st),
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
            tauri_state_for_test(st),
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
            tauri_state_for_test(st),
        ) {
            Ok(()) => self.push("add_var_bad_key", "ok", "unexpected".into()),
            Err(e) => self.push("add_var_bad_key", classify(&e), e.to_string()),
        }
        match storyforge_lib::sync_campaign_variable_schema(
            cid.as_str().to_string(),
            tauri_state_for_test(st),
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
            tauri_state_for_test(st),
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
            match storyforge_lib::complete_task(tid.as_str().to_string(), tauri_state_for_test(st))
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
            tauri_state_for_test(st),
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
            match storyforge_lib::abandon_task(tid.as_str().to_string(), tauri_state_for_test(st)) {
                Ok(()) => self.push("abandon_task", "ok", "ok".into()),
                Err(e) => self.push("abandon_task", classify(&e), e.to_string()),
            }
        }
        let missing_task = Id::new().to_string();
        self.ids.register("missing_task", &missing_task);
        match storyforge_lib::complete_task(missing_task.clone(), tauri_state_for_test(st)) {
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
            tauri_state_for_test(st),
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
            tauri_state_for_test(st),
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
                tauri_state_for_test(st),
            ) {
                Ok(()) => self.push(name, "ok", "ok".into()),
                Err(e) => self.push(name, classify(&e), e.to_string()),
            }
        }

        // ── opening ───────────────────────────────────────────────────
        match storyforge_lib::apply_campaign_opening(
            cid.as_str().to_string(),
            "改写后的开场".to_string(),
            tauri_state_for_test(st),
        ) {
            Ok(()) => self.push("apply_opening", "ok", "ok".into()),
            Err(e) => self.push("apply_opening", classify(&e), e.to_string()),
        }

        // ── cards（extract / list / get / delete）─────────────────────
        let card_count_before = storage.list_cards().map(|c| c.len()).unwrap_or(0);
        match storyforge_lib::extract_characters(
            "char-src-2".to_string(),
            None,
            tauri_state_for_test(st),
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
        match storyforge_lib::list_cards(tauri_state_for_test(st)) {
            Ok(cards) => self.push(
                "list_cards",
                "ok",
                format!("count={} (before={card_count_before})", cards.len()),
            ),
            Err(e) => self.push("list_cards", classify(&e), e.to_string()),
        }
        if let Some(card_id) = self.extracted_card_id.clone() {
            match storyforge_lib::get_card(card_id.as_str().to_string(), tauri_state_for_test(st)) {
                Ok(dto) => self.push(
                    "get_extracted_card",
                    "ok",
                    format!("name={} defs={}", dto.name, dto.definition_count),
                ),
                Err(e) => self.push("get_extracted_card", classify(&e), e.to_string()),
            }
            match storyforge_lib::delete_card(
                card_id.as_str().to_string(),
                tauri_state_for_test(st),
            ) {
                Ok(()) => self.push("delete_extracted_card", "ok", "ok".into()),
                Err(e) => self.push("delete_extracted_card", classify(&e), e.to_string()),
            }
        }
        match storyforge_lib::list_cards(tauri_state_for_test(st)) {
            Ok(cards) => self.push(
                "list_cards_after_delete",
                "ok",
                format!("count={}", cards.len()),
            ),
            Err(e) => self.push("list_cards_after_delete", classify(&e), e.to_string()),
        }

        // ── Meta typed patch：propose + accept（跨 Turn 的 stale 拒绝）──
        // 注入 schema 漂移（instance 变量不在定义 schema 中）→ 健康检查产出 patch。
        let drift_key = "extra_key".to_string();
        match storyforge_lib::set_character_variable(
            cid.as_str().to_string(),
            alice.as_str().to_string(),
            drift_key.clone(),
            serde_json::json!("drift"),
            Some(0),
            tauri_state_for_test(st),
        ) {
            Ok(()) => self.push("drift_inject", "ok", "ok".into()),
            Err(e) => self.push("drift_inject", classify(&e), e.to_string()),
        }
        let proposed = storyforge_lib::meta_propose_campaign_repairs(
            cid.as_str().to_string(),
            tauri_state_for_test(st),
        );
        let patch_count = proposed.as_ref().map(|p| p.len()).unwrap_or(0);
        match &proposed {
            Ok(_patches) => self.push("meta_propose", "ok", format!("patches={patch_count}")),
            Err(e) => self.push("meta_propose", classify(e), e.to_string()),
        }
        // 保存 propose 盖章的 revision（stale 判定基准）。
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
                tauri_state_for_test(st),
            ) {
                Ok(()) => self.push("meta_accept", "ok", "ok".into()),
                Err(e) => self.push("meta_accept", classify(&e), e.to_string()),
            }
        }

        // ── Turn 1 流水线：draft → regenerate → postprocess → Accept ──
        // （edit-stale 单独放在 Turn 2：SQLite 拒绝在 Stale Attempt 上
        //   regenerate，JSON 允许——该序列无法等价，故不放进等价矩阵。）
        let base_rev = self.campaign_revision();
        let user_node = st
            .conv_store
            .append_user_message(&conv_id, "打开大门".into())
            .expect("append user message");
        let turn = TurnRecord::new(cid.clone(), conv_id.clone(), user_node, base_rev);
        let turn_id = turn.turn_id.clone();
        self.ids.register("turn", turn_id.as_str());
        self.turn_ids.push(turn_id.clone());
        storage.save_turn(&turn).expect("save turn");

        let attempt1 = Id::new();
        let provisional_variant = Id::new();
        self.ids.register("attempt", attempt1.as_str());
        self.ids.register("variant", provisional_variant.as_str());
        let workflow = TurnWorkflow::new(storage.clone(), st.conv_store.clone());
        // JSON 流水线先落 draft 节点（SQLite 由 preaccept UoW 落）；
        // JSON 的 provisional variant id 必须是真实节点 id。
        let provisional_variant = if self.sqlite {
            provisional_variant
        } else {
            st.conv_store
                .append_ai_draft(&conv_id, "草稿甲：众人进入地下城".to_string(), None)
                .expect("JSON pipeline lands draft node")
        };
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
        self.ids.register("variant", draft.variant_id.as_str());
        self.push("draft", "ok", "ok".into());

        // regenerate：JSON 流水线先 replace_active_variant（SQLite 由 UoW 完成）
        let attempt2 = Id::new();
        self.ids.register("attempt", attempt2.as_str());
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
        // 事件捕获（PLAN §10.4 关键 Pipeline 事件）：按 production 后处理契约
        // （runtime_support.rs 同款 if/else）从 `apply_outcome` 结果派生
        // PostProcessDone / Skipped / Failed 事件——这是双后端真正可比较的事件
        // 序列（apply_outcome 是共享的写回逻辑，事件只由其结果驱动）。
        let pp_result: Result<pp::ProductionPostprocessResult, String> = (|| {
            let sink = storyforge_lib::BackendTurnAttemptSink::production(storage.clone());
            let runtime;
            let service = if self.sqlite {
                let campaign_rec = storage
                    .get_campaign(&cid)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "campaign missing for sqlite postprocess runtime".to_string())?;
                let instances = storage.list_instances(&cid).map_err(|e| e.to_string())?;
                let knowledge = storage.list_knowledge(&cid).map_err(|e| e.to_string())?;
                let tasks = storage.list_tasks(&cid).map_err(|e| e.to_string())?;
                let definitions_by_id = storage
                    .list_cards()
                    .map_err(|e| e.to_string())?
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
                    .map_err(|e| e.to_string())?;
                pp::ProductionPostprocessService::new_json(store, &sink)
            };
            let (_tx, cancel_rx) = tokio::sync::watch::channel(false);
            service
                .apply_outcome(
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
                .map_err(|e| e.to_string())
        })();
        match &pp_result {
            Ok(result) => {
                assert!(result.applied, "postprocess must apply");
                self.record_postprocess_events(result);
                self.push("postprocess", "ok", "ok".into());
            }
            Err(e) => {
                // Failed 路径仍记录事件（与 runtime_support 一致：发 PostProcessFailed）。
                self.pipeline_events
                    .push(format!("PostProcessFailed:{}", e));
                self.push("postprocess", "storage", e.clone());
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

        // ── Turn 2：draft → edit-stale（改写后 Attempt 变 Stale）──────
        let turn2_user = st
            .conv_store
            .append_user_message(&conv_id, "继续前进".into())
            .expect("append user message");
        let turn2 = TurnRecord::new(cid.clone(), conv_id.clone(), turn2_user, base_rev + 1);
        let turn2_id = turn2.turn_id.clone();
        self.ids.register("turn", turn2_id.as_str());
        self.turn_ids.push(turn2_id.clone());
        storage.save_turn(&turn2).expect("save turn 2");
        let attempt3 = Id::new();
        self.ids.register("attempt", attempt3.as_str());
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
                tauri_state_for_test(st),
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
        // 不 spawn worker（worker 需要 LLM）；等价矩阵只比较持久化状态机与
        // uncovered 计数（阈值判定是共享纯函数 should_enqueue_compress）。
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
                self.ids.register("compress_job", id.as_str());
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
                self.ids.register("compress_job", id.as_str());
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
            self.ids.register_uuid_ids_by_path(&mut v, "bundle");
            self.ids.canonicalize(&mut v);
            canonicalize_timestamps(&mut v);
            normalize_pending_batch_status(&mut v);
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
                        self.ids.register("campaign", result.campaign_id.as_str());
                        self.ids
                            .register("conversation", result.conversation_id.as_str());
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

        // ── restart recovery：真正的「重启 = 同目录、新 AppState」────────
        // 必须在 delete_campaign **之前**：植入需要引用仍存在的 Campaign（SQLite
        // compress_jobs 对 campaign_id 有外键）。不是只发一条字符串。植入一个
        // Running 态 compress job + 一个 Committing 态 Turn（崩溃中断残留），
        // 丢弃当前进程内状态，重新打开同一 authority（SQLite 重开 DB；JSON 重读
        // 磁盘文件），再跑生产启动恢复原语，断言恢复幂等且崩溃残留被收敛。
        self.run_restart_recovery();

        // 删除前的完整领域快照（Turn/Attempt/知识/任务/总结/会话/世界书 + 上一步
        // 植入并被 reset 回 Pending 的 recovery compress job）。
        // ⚠️ 必须在 delete_campaign **之前**捕获——否则级联删掉的数据不在快照里，
        // “删除前等价”就成了删除后等价的假阳性（审查跟进 P1）。
        let snapshot_before_delete = self.snapshot();

        // ── 删除（delete_campaign 级联；二次删除 → not_found）─────────
        match storyforge_lib::delete_campaign(cid.as_str().to_string(), tauri_state_for_test(st)) {
            Ok(()) => self.push("delete_campaign", "ok", "ok".into()),
            Err(e) => self.push("delete_campaign", classify(&e), e.to_string()),
        }
        match storyforge_lib::delete_campaign(cid.as_str().to_string(), tauri_state_for_test(st)) {
            Ok(()) => self.push("delete_campaign_again", "ok", "unexpected".into()),
            Err(e) => self.push("delete_campaign_again", classify(&e), e.to_string()),
        }

        let snapshot = self.snapshot();
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

    /// 真实「重启恢复」证明（审查跟进 P1）：植入崩溃残留 → 重新打开同一 authority
    /// → 跑生产启动恢复原语 → 断言收敛。
    ///
    /// - 植入一个 Running 态 compress job（崩溃中断的 worker 残留）。
    /// - 植入一个 Committing 态 Turn（崩溃中断的 accept 残留，JSON/SQLite 都能造）。
    /// - 重新打开：构造新的 StorageFacade + AppState（SQLite 仍指向同一 DB 文件；
    ///   JSON 重新 load 磁盘文件），即「重启」语义。
    /// - 跑恢复：SQLite 调 `recover_turns_on_startup` + `compress_reset_running_to_pending`；
    ///   JSON 走 `TurnLifecycleService::recover_turns_on_startup` + `reset_running_to_pending`。
    /// - 断言：Running compress job → Pending；再次恢复幂等（0 reset）；恢复 op 入表。
    #[allow(clippy::too_many_lines)]
    fn run_restart_recovery(&mut self) {
        let storage = self.state.storage();

        // 植入：Running compress job（崩溃中断的 worker）。
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
                self.ids.register("compress_job", x.0.as_str());
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

        // Turn 恢复（Committing 重放）由专门的 sqlite_chronicle_crash_recovery.rs /
        // gate5_fault_matrix.rs 覆盖，且其 dispatch 在 lib.rs::recover_turns_on_startup
        // 中是共享的；此处不重复植入 Committing Turn（已有活动 Turn 会冲突），
        // 只验证 compress-job 重置的「重启」收敛 + 幂等——这是双后端在此 fixture
        // 下可比较的恢复证明。

        // 重新打开同一 authority（「重启」）。
        let data_dir = storage.data_dir().to_path_buf();
        let reopened_storage = Arc::new(StorageFacade::new(
            data_dir.clone(),
            if self.sqlite {
                PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env)
            } else {
                PinnedBackend::new(StorageBackend::Json, BackendSource::Default)
            },
        ));
        if self.sqlite {
            reopened_storage
                .validate_runtime_authority()
                .expect("reopened facade must match the still-active SQLite runtime");
        }
        let reopened_state = match AppState::new_with_backend(data_dir, reopened_storage.clone()) {
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
        // 仅驱动 compress-job 重置原语（双后端可比、无 Turn 副作用）。
        //
        // 不在此处调 `recover_turns_on_startup`：JSON 与 SQLite 的 Turn 恢复语义
        // 不同（SQLite 会 fail 非 terminal Turn；JSON 只重放 Committing），会把
        // 本 fixture 中 Turn 2（edit-stale）推成不同状态、破坏删除前快照等价。
        // Turn 恢复的可比证明由专门的 sqlite_chronicle_crash_recovery.rs /
        // gate5_fault_matrix.rs（infra 级，独立 authority）覆盖；此处只验证
        // 「持久化 Running compress job 跨重开存活、reset 收敛、二次幂等」——
        // 这是不带 Turn 副作用、双后端真正可比的重启恢复断言。
        let _ = &reopened_state;
        let compress_reset = reopened_storage
            .reset_running_compress_jobs_to_pending()
            .unwrap_or(0);

        // 断言：seed Running job 被 reset 为 Pending（≥1）。
        let job_now_pending = reopened_storage
            .list_compress_jobs()
            .map(|jobs| jobs.iter().any(|j| j.id == job_id && j.status == "pending"))
            .unwrap_or(false);

        // 幂等：再跑一次恢复，Running 已清零 → reset=0。
        let second_reset = reopened_storage
            .reset_running_compress_jobs_to_pending()
            .unwrap_or(0);

        let ok = running_before && compress_reset >= 1 && job_now_pending && second_reset == 0;
        self.push(
            "restart_recovery",
            if ok { "ok" } else { "storage" },
            format!(
                "running_before={running_before} \
                 compress_reset={compress_reset} job_now_pending={job_now_pending} \
                 second_reset_idempotent={second_reset}"
            ),
        );
    }

    /// 规范化领域快照（从驱动跟踪的 id + storage 读取）。
    ///
    /// 归一化管线：未登记随机 id（导入产物等）按结构路径登记 → 已知 id 换
    /// ⟨label⟩ → 时间戳换 ⟨ts⟩ → 对象数组排序（集合语义，双后端 list_* 顺序无关）。
    fn snapshot(&mut self) -> serde_json::Value {
        let storage = self.state.storage();

        let mut cards: Vec<serde_json::Value> = storage
            .list_cards()
            .map(|cards| {
                cards
                    .iter()
                    .map(|c| serde_json::to_value(c).unwrap_or(serde_json::json!(null)))
                    .collect()
            })
            .unwrap_or_default();
        for card in &mut cards {
            if let Some(map) = card.as_object_mut() {
                map.remove("imported_at");
            }
        }

        let campaigns: Vec<serde_json::Value> = storage
            .list_campaigns(None)
            .map(|records| {
                records
                    .iter()
                    .map(|r| serde_json::to_value(&r.campaign).unwrap_or(serde_json::json!(null)))
                    .collect()
            })
            .unwrap_or_default();

        let mut instances = Vec::new();
        let mut knowledge = Vec::new();
        let mut tasks = Vec::new();
        let mut summaries = Vec::new();
        let mut world_info = Vec::new();
        if let Some(cid) = &self.campaign_id {
            if let Ok(list) = storage.list_instances(cid) {
                instances = list
                    .iter()
                    .map(|i| serde_json::to_value(i).unwrap_or(serde_json::json!(null)))
                    .collect();
            }
            if let Ok(list) = storage.list_knowledge(cid) {
                knowledge = list
                    .iter()
                    .map(|k| serde_json::to_value(k).unwrap_or(serde_json::json!(null)))
                    .collect();
            }
            if let Ok(list) = storage.list_tasks(cid) {
                tasks = list
                    .iter()
                    .map(|t| serde_json::to_value(t).unwrap_or(serde_json::json!(null)))
                    .collect();
            }
            if let Ok(list) = storage.list_summaries(cid) {
                summaries = list
                    .iter()
                    .map(|s| serde_json::to_value(s).unwrap_or(serde_json::json!(null)))
                    .collect();
            }
            if let Ok(book) = storage.get_world_info(cid) {
                world_info.push(serde_json::to_value(&book).unwrap_or(serde_json::json!(null)));
            }
        }

        let mut turns = Vec::new();
        for tid in &self.turn_ids {
            if let Ok(Some(turn)) = storage.get_turn(tid) {
                let mut v: serde_json::Value =
                    serde_json::to_value(&turn).unwrap_or(serde_json::json!(null));
                // 归一化：`pending_state_changes.status`（Prepared/Applying/Committed）是
                // **执行态标志**，不是领域语义——JSON 在 postprocess 后停在 Prepared
                // （batch 暂存待 accept），SQLite preaccept UoW 已落 Committed（batch 进
                // outbox 待 accept）。两者领域内容（mutations/commit_id/revision）相同，
                // 该 flag 随执行进度变化且双后端合法不同，按归一化规则剔除。
                normalize_pending_batch_status(&mut v);
                turns.push(v);
            }
        }

        let mut convs = Vec::new();
        if let Some(conv_id) = &self.conv_id
            && let Some(conv) = self.state.conv_store.get(conv_id)
        {
            convs.push(serde_json::to_value(&conv).unwrap_or(serde_json::json!(null)));
        }

        let chars: Vec<serde_json::Value> = storage
            .list_characters()
            .map(|list| {
                list.iter()
                    .map(|c| serde_json::to_value(&c.info).unwrap_or(serde_json::json!(null)))
                    .collect()
            })
            .unwrap_or_default();

        let mvu: Vec<serde_json::Value> = storage
            .list_mvu()
            .map(|list| {
                list.iter()
                    .map(|m| serde_json::to_value(m).unwrap_or(serde_json::json!(null)))
                    .collect()
            })
            .unwrap_or_default();

        let mut snap = serde_json::Map::new();
        snap.insert("cards".into(), serde_json::Value::Array(cards));
        snap.insert("campaigns".into(), serde_json::Value::Array(campaigns));
        snap.insert("instances".into(), serde_json::Value::Array(instances));
        snap.insert("knowledge".into(), serde_json::Value::Array(knowledge));
        snap.insert("tasks".into(), serde_json::Value::Array(tasks));
        snap.insert("summaries".into(), serde_json::Value::Array(summaries));
        snap.insert("world_info".into(), serde_json::Value::Array(world_info));
        snap.insert("turns".into(), serde_json::Value::Array(turns));
        snap.insert("conversations".into(), serde_json::Value::Array(convs));
        snap.insert("characters".into(), serde_json::Value::Array(chars));
        snap.insert("mvu".into(), serde_json::Value::Array(mvu));

        let mut value = serde_json::Value::Object(snap);
        self.ids.register_uuid_ids_by_path(&mut value, "snap");
        self.ids.canonicalize(&mut value);
        canonicalize_timestamps(&mut value);
        self.ids.sort_object_arrays(&mut value);
        value
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

/// 主测试：同一 op 序列跑 JSON 与 SQLite，比较操作结果 + 领域快照。
#[tokio::test]
async fn backend_parity_equivalent_domain_snapshots() {
    // ── JSON 阶段 ────────────────────────────────────────────────────
    let json_dir = tempfile::tempdir().unwrap();
    write_fixture(json_dir.path());
    let json_state = json_app_state(json_dir.path());
    let mut json_driver = ParityDriver::new(&json_state, false);
    let json_out = json_driver.run().await;
    drop(json_state);

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
    let mut sqlite_driver = ParityDriver::new(&sqlite_state, true);
    let sqlite_out = sqlite_driver.run().await;
    drop(sqlite_state);

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

    // ── 比较：规范化领域快照 ──────────────────────────────────────────
    assert_eq!(
        json_out.snapshot, sqlite_out.snapshot,
        "JSON 与 SQLite 的规范化领域快照必须等价（删除后）"
    );
    assert_eq!(
        json_out.snapshot_before_delete, sqlite_out.snapshot_before_delete,
        "JSON 与 SQLite 的规范化领域快照必须等价（删除前，含 Turn/Attempt/知识/任务/总结）"
    );

    // ── 比较：关键 Pipeline 事件序列（PLAN §10.4）────────────────────
    // postprocess 的 Done/Skipped/Failed 事件按 production 契约从 apply_outcome
    // 结果派生——双后端输入相同 → 事件序列逐项相等。
    assert_eq!(
        json_out.pipeline_events, sqlite_out.pipeline_events,
        "JSON 与 SQLite 的关键 Pipeline 事件序列必须等价"
    );
}
