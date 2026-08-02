//! Gate 5 数据矩阵：JSON → SQLite 迁移等价、幂等重试、marker-last、
//! 故障保留、上一版 schema、损坏/缺失/磁盘写失败。
//!
//! 与既有 cutover.rs / migration_readiness.rs 的区别：本文件用**全量数据矩阵**
//! fixture（多角色/多轮/同名与临时角色/全部集合非空/Pending·Failed·Committed
//! Turn/Attempt/MVU+story_clock/四态 compress job/角色库/世界书）证明
//! 「迁移前后规范化领域快照等价」，而不只是行数或哈希。

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::cutover::{
    CutoverFault, CutoverOutcome, CutoverPlan, CutoverRequest, MarkerStatus, inspect_marker,
    recover_or_verify, run_cutover_with_fault,
};
use storyforge_infra_sqlite::importer::{ImportStatus, JsonImporter};
use storyforge_infra_sqlite::migrations::{
    builtin_migrations, current_version, migrate, migrate_with,
};

/// 与 importer/exporter 相同的 compress_jobs 列投影（hash 与快照比较共用）。
fn project_compress_job(job: &Value) -> Value {
    let opt_str = |k: &str| -> Option<String> {
        job.get(k).and_then(|v| match v {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        })
    };
    let opt_u64 = |k: &str| -> u64 {
        job.get(k)
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_i64().map(|i| i.max(0) as u64))
                    .or_else(|| v.as_str()?.parse().ok())
            })
            .unwrap_or(0)
    };
    let mut v = json!({
        "id": job["id"],
        "campaign_id": job["campaign_id"],
        "kind": opt_str("kind").unwrap_or_else(|| "auto".into()),
        "status": opt_str("status").unwrap_or_else(|| "pending".into()),
        "attempts": opt_u64("attempts"),
        "max_attempts": opt_u64("max_attempts").max(5),
        "uncovered_a_at_enqueue": opt_u64("uncovered_a_at_enqueue"),
        "uncovered_b_at_enqueue": opt_u64("uncovered_b_at_enqueue"),
        "created_at": opt_str("created_at").unwrap_or_default(),
        "updated_at": opt_str("updated_at").unwrap_or_default(),
    });
    if let Some(x) = opt_str("conversation_id") {
        v["conversation_id"] = json!(x);
    }
    if let Some(x) = opt_str("lineage_id") {
        v["lineage_id"] = json!(x);
    }
    if let Some(x) = opt_str("last_error") {
        v["last_error"] = json!(x);
    }
    v
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn json_array(items: &[Value]) -> Value {
    Value::Array(items.to_vec())
}

/// 全量数据矩阵 fixture：覆盖 PLAN §10.1 除损坏/缺文件外的全部独立场景。
pub(crate) fn write_matrix_source(dir: &Path) {
    write_json(
        &dir.join("cards.json"),
        &json_array(&[
            json!({
                "card": {
                    "id": "card-1",
                    "name": "双角色卡",
                    "source_character_id": "char-src-1",
                    "character_definitions": [
                        {
                            "id": "def-alice", "name": "Alice",
                            "role_type": "Protagonist",
                            "persona_prompt": "勇敢", "behavior_rules": "不撒谎",
                            "base_backstory": ["孤儿"], "group": "主角团",
                            "variable_schema": []
                        },
                        {
                            "id": "def-bob", "name": "Bob",
                            "role_type": "Supporting",
                            "persona_prompt": "沉稳", "behavior_rules": "守约",
                            "base_backstory": ["骑士"], "group": "主角团",
                            "variable_schema": []
                        }
                    ],
                    "campaign_variable_schema": [],
                    "raw_card_json": {},
                    "extraction_status": "extracted",
                    "extraction_message": null
                },
                "imported_at": "2026-07-01T00:00:00Z"
            }),
            json!({
                "card": {
                    "id": "card-2",
                    "name": "单角色卡",
                    "source_character_id": "char-src-2",
                    "character_definitions": [
                        {
                            "id": "def-carol", "name": "Carol",
                            "role_type": "Protagonist",
                            "persona_prompt": "机智", "behavior_rules": "隐藏身份",
                            "base_backstory": [], "group": null,
                            "variable_schema": []
                        }
                    ],
                    "campaign_variable_schema": [],
                    "raw_card_json": {},
                    "extraction_status": "fallback",
                    "extraction_message": "单角色降级"
                },
                "imported_at": "2026-07-02T00:00:00Z"
            }),
        ]),
    );
    write_json(
        &dir.join("campaigns.json"),
        &json_array(&[
            json!({
                "id": "camp-1",
                "card_id": "card-1",
                "name": "多角色多轮局",
                "conversation_id": "conv-1",
                "revision": 3,
                "chronicle_revision": 2,
                "lineage_id": "lin-1",
                // 权威 story_clock 在 variables；顶层字段是旧双表示（分歧态）。
                "story_clock": "Day 1",
                "variables": [
                    { "key": "story_clock", "value": "Day 3", "turn": 2 },
                    { "key": "gold", "value": 100, "turn": 0 }
                ],
                "variable_schema": [
                    { "key": "gold", "label": "金币", "value_type": "int",
                      "default": 0, "description": null, "group": "全局" }
                ],
                "created_at": "2026-07-03T00:00:00Z"
            }),
            json!({
                "id": "camp-2",
                "card_id": "card-2",
                "name": "旧式单轮局",
                "conversation_id": "conv-2",
                "revision": 0,
                "chronicle_revision": 0,
                "lineage_id": "lin-2",
                "story_clock": "Day 1",
                "variables": [],
                "variable_schema": [],
                "created_at": "2026-07-04T00:00:00Z"
            }),
        ]),
    );
    write_json(
        &dir.join("instances.json"),
        &json_array(&[
            json!({
                "id": "inst-alice-1", "campaign_id": "camp-1",
                "definition_id": "def-alice", "name": "Alice",
                "is_temporary": false, "variables": []
            }),
            json!({
                "id": "inst-bob-1", "campaign_id": "camp-1",
                "definition_id": "def-bob", "name": "Bob",
                "is_temporary": false, "variables": []
            }),
            // 临时角色（无 definition）
            json!({
                "id": "inst-temp-1", "campaign_id": "camp-1",
                "definition_id": null, "name": "临时旅人",
                "is_temporary": true, "variables": []
            }),
            // 同名角色（跨 Campaign 与 inst-alice-1 同名）
            json!({
                "id": "inst-alice-2", "campaign_id": "camp-2",
                "definition_id": "def-carol", "name": "Alice",
                "is_temporary": false, "variables": []
            }),
            json!({
                "id": "inst-carol-2", "campaign_id": "camp-2",
                "definition_id": "def-carol", "name": "Carol",
                "is_temporary": false, "variables": []
            }),
        ]),
    );
    write_json(
        &dir.join("knowledge.json"),
        &json_array(&[
            json!({
                "id": "know-1", "campaign_id": "camp-1",
                "character_id": "inst-alice-1", "content": "知道密道位置",
                "source_turn": 1
            }),
            json!({
                "id": "know-2", "campaign_id": "camp-2",
                "character_id": "inst-carol-2", "content": "伪装成商贩",
                "source_turn": 0
            }),
        ]),
    );
    write_json(
        &dir.join("tasks.json"),
        &json_array(&[
            json!({
                "id": "task-1", "campaign_id": "camp-1",
                "title": "寻找钥匙", "description": "地下城三层",
                "triggers": [{"kind": "keyword", "value": "钥匙"}],
                "status": "active", "created_turn": 1
            }),
            json!({
                "id": "task-2", "campaign_id": "camp-1",
                "title": "护送商队", "description": "穿过峡谷",
                "triggers": [],
                "status": "completed", "created_turn": 2
            }),
        ]),
    );
    // A←B 摘要图（双向一致，importer 诊断要求）
    write_json(
        &dir.join("round_summaries.json"),
        &json_array(&[
            json!({
                "id": "sum-a1", "campaign_id": "camp-1", "conversation_id": "conv-1",
                "lineage_id": "lin-1", "level": 0, "turn": 1, "turn_end": 1,
                "code": "A0001", "headline": "进入地下城", "content": "队伍进入地下城",
                "covered_by": "sum-b1", "covers": [],
                "created_at": "2026-07-03T01:00:00Z"
            }),
            json!({
                "id": "sum-b1", "campaign_id": "camp-1", "conversation_id": "conv-1",
                "lineage_id": "lin-1", "level": 1, "turn": 1, "turn_end": 1,
                "code": "B0001", "headline": "第一轮结束", "content": "第一轮总结",
                "covered_by": null, "covers": ["sum-a1"],
                "created_at": "2026-07-03T02:00:00Z"
            }),
        ]),
    );
    write_json(
        &dir.join("conversations").join("conv-1.json"),
        &json!({
            "id": "conv-1", "campaign_id": "camp-1", "character_id": "char-src-1",
            "nodes": [], "archived_upto": 1,
            "created_at": "2026-07-03T00:00:00Z", "updated_at": "2026-07-03T03:00:00Z"
        }),
    );
    write_json(
        &dir.join("conversations").join("conv-2.json"),
        &json!({
            "id": "conv-2", "campaign_id": "camp-2", "character_id": "char-src-2",
            "nodes": [], "archived_upto": 0,
            "created_at": "2026-07-04T00:00:00Z", "updated_at": "2026-07-04T00:00:00Z"
        }),
    );
    // Pending / Failed / Committed Turn + 各状态 Attempt
    write_json(
        &dir.join("turns.json"),
        &json_array(&[
            json!({
                "turn_id": "turn-committed",
                "campaign_id": "camp-1", "conversation_id": "conv-1",
                "input_node_id": "node-1", "base_campaign_revision": 0,
                "status": "committed", "accepted_attempt_id": "att-committed",
                "failure_reason": null,
                "attempts": [{
                    "attempt_id": "att-committed", "variant_id": "var-1",
                    "draft_hash": "hash-committed", "status": "committed",
                    "created_at": "2026-07-03T00:10:00Z"
                }],
                "created_at": "2026-07-03T00:00:00Z", "updated_at": "2026-07-03T00:20:00Z"
            }),
            json!({
                "turn_id": "turn-pending",
                "campaign_id": "camp-1", "conversation_id": "conv-1",
                "input_node_id": "node-2", "base_campaign_revision": 1,
                "status": "generating", "accepted_attempt_id": null,
                "failure_reason": null,
                "attempts": [{
                    "attempt_id": "att-pending", "variant_id": "var-2",
                    "draft_hash": "", "status": "generating",
                    "created_at": "2026-07-03T01:10:00Z"
                }],
                "created_at": "2026-07-03T01:00:00Z", "updated_at": "2026-07-03T01:10:00Z"
            }),
            json!({
                "turn_id": "turn-failed",
                "campaign_id": "camp-2", "conversation_id": "conv-2",
                "input_node_id": "node-3", "base_campaign_revision": 0,
                "status": "failed", "accepted_attempt_id": null,
                "failure_reason": "模型超时",
                "attempts": [{
                    "attempt_id": "att-failed", "variant_id": "var-3",
                    "draft_hash": "hash-failed", "status": "failed",
                    "created_at": "2026-07-04T00:10:00Z"
                }],
                "created_at": "2026-07-04T00:00:00Z", "updated_at": "2026-07-04T00:15:00Z"
            }),
        ]),
    );
    // MVU translation + story_clock 权威
    write_json(
        &dir.join("mvu_translations.json"),
        &json_array(&[json!({
            "source_character_id": "char-src-1",
            "character_name": "Alice",
            "analyzed_at": "2026-07-05T00:00:00Z",
            "translation": {
                "update_rules": ["damage reduces hp"],
                "fallback_fragments": [],
                "ui_bindings": []
            }
        })]),
    );
    // 本局世界书
    write_json(
        &dir.join("campaign_world_info").join("camp-1.json"),
        &json!({
            "entries": [
                {
                    "keys": ["密道"], "content": "密道在酒窖",
                    "constant": true, "route": "Both",
                    "is_global": false, "depth": 2, "order": 100,
                    "extensions": {}
                }
            ],
            "source": "native",
            "metadata": {"seeded_from": "card"}
        }),
    );
    // 四态 compress job：Pending / Running / Failed / Succeeded
    write_json(
        &dir.join("compress_jobs.json"),
        &json_array(&[
            json!({
                "id": "job-pending", "campaign_id": "camp-1", "conversation_id": "conv-1",
                "lineage_id": "lin-1", "kind": "auto", "status": "pending",
                "attempts": 0, "max_attempts": 5, "last_error": null,
                "uncovered_a_at_enqueue": 12, "uncovered_b_at_enqueue": 0,
                "created_at": "2026-07-03T04:00:00Z", "updated_at": "2026-07-03T04:00:00Z"
            }),
            json!({
                "id": "job-running", "campaign_id": "camp-2", "conversation_id": "conv-2",
                "lineage_id": "lin-2", "kind": "auto", "status": "running",
                "attempts": 1, "max_attempts": 5, "last_error": null,
                "uncovered_a_at_enqueue": 12, "uncovered_b_at_enqueue": 0,
                "created_at": "2026-07-03T04:10:00Z", "updated_at": "2026-07-03T04:10:00Z"
            }),
            json!({
                "id": "job-failed", "campaign_id": "camp-2", "conversation_id": "conv-2",
                "lineage_id": "lin-2", "kind": "manual", "status": "failed",
                "attempts": 5, "max_attempts": 5, "last_error": "模型拒绝压缩",
                "uncovered_a_at_enqueue": 3, "uncovered_b_at_enqueue": 1,
                "created_at": "2026-07-04T01:00:00Z", "updated_at": "2026-07-04T01:30:00Z"
            }),
            json!({
                "id": "job-succeeded", "campaign_id": "camp-2", "conversation_id": "conv-2",
                "lineage_id": "lin-2", "kind": "manual", "status": "succeeded",
                "attempts": 2, "max_attempts": 5, "last_error": null,
                "uncovered_a_at_enqueue": 3, "uncovered_b_at_enqueue": 1,
                "created_at": "2026-07-04T02:00:00Z", "updated_at": "2026-07-04T02:05:00Z"
            }),
        ]),
    );
    // 角色库（V007 characters，Gate 5 加入迁移）
    write_json(
        &dir.join("characters.json"),
        &json_array(&[
            json!({
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
            }),
            json!({
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
            }),
        ]),
    );
}

// ─── 规范化领域快照 ────────────────────────────────────────────────────

/// JSON 侧快照：读源目录各文件，条目按稳定 JSON 排序。
/// compress_jobs 与 characters 按 importer/exporter 的投影形态归一。
fn json_domain_snapshot(dir: &Path) -> Value {
    let mut snap = serde_json::Map::new();
    for (name, file) in [
        ("cards", "cards.json"),
        ("campaigns", "campaigns.json"),
        ("instances", "instances.json"),
        ("knowledge", "knowledge.json"),
        ("tasks", "tasks.json"),
        ("round_summaries", "round_summaries.json"),
        ("turns", "turns.json"),
        ("mvu_translations", "mvu_translations.json"),
        ("compress_jobs", "compress_jobs.json"),
    ] {
        let path = dir.join(file);
        let items: Vec<Value> = if path.exists() {
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap()
        } else {
            vec![]
        };
        // compress_jobs 按 importer/exporter 同投影（null 可选字段省略）
        let items = if name == "compress_jobs" {
            items.iter().map(project_compress_job).collect()
        } else {
            items
        };
        snap.insert(name.to_string(), sorted_value(items, name));
    }
    // conversations 目录
    let convs = read_dir_values(&dir.join("conversations"));
    snap.insert("conversations".into(), sorted_value(convs, "conversations"));
    // campaign_world_info 目录（文件名即 campaign_id，与 importer 同口径）
    let mut wi: Vec<Value> = Vec::new();
    if let Ok(entries) = fs::read_dir(dir.join("campaign_world_info")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let text = fs::read_to_string(&path).unwrap();
                wi.push(serde_json::from_str(&text).unwrap());
            }
        }
    }
    snap.insert(
        "campaign_world_info".into(),
        sorted_value(wi, "campaign_world_info"),
    );
    // characters：归一为 {id, info, imported_at}
    let chars = {
        let raw: Vec<Value> = if dir.join("characters.json").exists() {
            serde_json::from_str(&fs::read_to_string(dir.join("characters.json")).unwrap()).unwrap()
        } else {
            vec![]
        };
        raw.into_iter()
            .map(|entry| {
                json!({
                    "id": entry["id"],
                    "info": entry["info"],
                    "imported_at": entry.get("imported_at").and_then(|v| v.as_str()).unwrap_or(""),
                })
            })
            .collect()
    };
    snap.insert("characters".into(), sorted_value(chars, "characters"));
    Value::Object(snap)
}

/// SQLite 侧快照：从 payload 行重建同一投影，条目按稳定 JSON 排序。
fn sqlite_domain_snapshot(db: &Database) -> Value {
    let mut snap = serde_json::Map::new();
    for (name, table) in [
        ("cards", "character_cards"),
        ("campaigns", "campaigns"),
        ("instances", "character_instances"),
        ("knowledge", "character_knowledge"),
        ("tasks", "story_tasks"),
        ("round_summaries", "round_summaries"),
        ("turns", "turns"),
        ("conversations", "conversations"),
        ("mvu_translations", "mvu_translations"),
    ] {
        snap.insert(
            name.to_string(),
            sorted_value(load_payloads(db, table), name),
        );
    }
    let wi = {
        let mut stmt = db
            .connection()
            .prepare("SELECT payload_json FROM campaign_world_info ORDER BY campaign_id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows.iter()
            .map(|raw| serde_json::from_str(raw).unwrap())
            .collect::<Vec<Value>>()
    };
    snap.insert(
        "campaign_world_info".into(),
        sorted_value(wi, "campaign_world_info"),
    );
    // compress_jobs 投影（与 exporter 同列）
    let jobs = {
        let mut stmt = db
            .connection()
            .prepare(
                "SELECT job_id, campaign_id, conversation_id, lineage_id, kind, status, attempts, \
                 max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue, \
                 created_at, updated_at FROM chronicle_compress_jobs ORDER BY job_id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, i64>(9)?,
                    r.get::<_, i64>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, String>(12)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows.into_iter()
            .map(
                |(
                    id,
                    cid,
                    conv,
                    lin,
                    kind,
                    status,
                    attempts,
                    max_attempts,
                    last_error,
                    ua,
                    ub,
                    created,
                    updated,
                )| {
                    let mut v = json!({
                        "id": id, "campaign_id": cid, "kind": kind, "status": status,
                        "attempts": attempts, "max_attempts": max_attempts,
                        "uncovered_a_at_enqueue": ua, "uncovered_b_at_enqueue": ub,
                        "created_at": created, "updated_at": updated,
                    });
                    if let Some(x) = conv {
                        v["conversation_id"] = json!(x);
                    }
                    if let Some(x) = lin {
                        v["lineage_id"] = json!(x);
                    }
                    if let Some(x) = last_error {
                        v["last_error"] = json!(x);
                    }
                    v
                },
            )
            .collect::<Vec<Value>>()
    };
    snap.insert("compress_jobs".into(), sorted_value(jobs, "compress_jobs"));
    // characters 投影
    let chars = {
        let mut stmt = db
            .connection()
            .prepare(
                "SELECT character_id, info_json, imported_at FROM characters ORDER BY character_id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows.into_iter()
            .map(|(id, info, imported)| {
                json!({
                    "id": id,
                    "info": serde_json::from_str::<Value>(&info).unwrap(),
                    "imported_at": imported,
                })
            })
            .collect::<Vec<Value>>()
    };
    snap.insert("characters".into(), sorted_value(chars, "characters"));
    Value::Object(snap)
}

fn load_payloads(db: &Database, table: &str) -> Vec<Value> {
    let sql = format!("SELECT payload_json FROM {table}");
    let mut stmt = db.connection().prepare(&sql).unwrap();
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    rows.iter()
        .map(|raw| serde_json::from_str(raw).unwrap())
        .collect()
}

fn read_dir_values(dir: &Path) -> Vec<Value> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap())
        .collect()
}

fn sorted_value(items: Vec<Value>, _label: &str) -> Value {
    let mut encoded: Vec<String> = items
        .iter()
        .map(|v| serde_json::to_string(v).unwrap())
        .collect();
    encoded.sort();
    Value::Array(
        encoded
            .iter()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect(),
    )
}

// ─── 测试 ────────────────────────────────────────────────────────────────

/// 1. JSON → SQLite 迁移前后规范化领域快照等价（全量数据矩阵）。
#[test]
fn json_to_sqlite_full_matrix_snapshot_is_equivalent() {
    let dir = tempfile::tempdir().unwrap();
    write_matrix_source(dir.path());

    let db_dir = tempfile::tempdir().unwrap();
    let mut db = Database::open(db_dir.path().join("matrix.sqlite3")).unwrap();
    let report = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap();
    assert_eq!(report.status, ImportStatus::Completed);
    assert_eq!(report.cards, 2);
    assert_eq!(report.campaigns, 2);
    assert_eq!(report.instances, 5);
    assert_eq!(report.knowledge, 2);
    assert_eq!(report.tasks, 2);
    assert_eq!(report.summaries, 2);
    assert_eq!(report.conversations, 2);
    assert_eq!(report.turns, 3);
    assert_eq!(report.mvu_translations, 1);
    assert_eq!(report.world_info, 1);
    assert_eq!(report.compress_jobs, 4);
    assert_eq!(report.characters, 2);

    let json_snap = json_domain_snapshot(dir.path());
    let db_snap = sqlite_domain_snapshot(&db);
    assert_eq!(
        json_snap, db_snap,
        "JSON→SQLite 迁移后领域快照必须等价（差异见上方）"
    );

    // story_clock 索引列以 variables 为权威（"Day 3"），不是分歧的顶层 "Day 1"。
    let stored_clock: String = db
        .connection()
        .query_row(
            "SELECT story_clock FROM campaigns WHERE campaign_id = 'camp-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored_clock, "Day 3");
    // 角色库去重覆盖语义：同 id 重导入不产生重复行。
    assert_eq!(
        storyforge_infra_sqlite::importer::table_count(&db, "characters").unwrap(),
        2
    );
}

/// 2. 同一迁移可安全重试并保持幂等（importer 去重 + cutover AlreadyCutover）。
#[test]
fn migration_retry_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    write_matrix_source(dir.path());

    let mut db = Database::open_in_memory().unwrap();
    let r1 = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap();
    assert_eq!(r1.status, ImportStatus::Completed);
    let r2 = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap();
    assert_eq!(r2.status, ImportStatus::SkippedDuplicate);
    assert_eq!(r2.source_manifest_hash, r1.source_manifest_hash);
    assert_eq!(
        storyforge_infra_sqlite::importer::table_count(&db, "campaigns").unwrap(),
        2
    );
    assert_eq!(
        storyforge_infra_sqlite::importer::table_count(&db, "characters").unwrap(),
        2
    );

    // cutover 级重试：第二次 AlreadyCutover，数据不变。
    let cut_dir = tempfile::tempdir().unwrap();
    write_matrix_source(cut_dir.path());
    let db_path = cut_dir.path().join("storyforge.sqlite3");
    let request = CutoverRequest {
        plan: CutoverPlan::new(cut_dir.path(), &db_path),
        label: "gate5-matrix-retry".into(),
    };
    match recover_or_verify(&request).unwrap() {
        CutoverOutcome::Completed(report) => {
            assert_eq!(report.campaigns, 2);
            assert_eq!(report.characters, 2);
        }
        other => panic!("first cutover must complete, got {other:?}"),
    }
    match recover_or_verify(&request).unwrap() {
        CutoverOutcome::AlreadyCutover(report) => {
            assert_eq!(report.campaigns, 2);
            assert_eq!(report.characters, 2);
        }
        other => panic!("retry must report AlreadyCutover, got {other:?}"),
    }
}

/// 3. marker 只在完整成功后写入；任意阶段失败：
///    - JSON 不被修改/删除；
///    - backup 保留、无半迁移状态、可安全重试。
#[test]
fn marker_last_and_failure_retention_for_every_stage() {
    let faults = [
        CutoverFault::AfterLock,
        CutoverFault::AfterValidate,
        CutoverFault::AfterBackup,
        CutoverFault::AfterImport,
        CutoverFault::AfterVerify,
        CutoverFault::AfterPublishBeforeMarker,
    ];
    for fault in faults {
        let dir = tempfile::tempdir().unwrap();
        write_matrix_source(dir.path());
        let db_path = dir.path().join("storyforge.sqlite3");
        // 快照 JSON 字节用于证明源文件未被修改/删除。
        let campaigns_before = fs::read(dir.path().join("campaigns.json")).unwrap();
        let characters_before = fs::read(dir.path().join("characters.json")).unwrap();
        let request = CutoverRequest {
            plan: CutoverPlan::new(dir.path(), &db_path),
            label: format!("gate5-fault-{fault:?}"),
        };

        let err = run_cutover_with_fault(&request, fault).expect_err("fault must fail");
        assert!(
            err.to_string().contains("injected fault"),
            "错误必须来自注入点（证明确实进入目标阶段）: {err}"
        );

        // 失败后：marker 不声称 SQLite 权威。
        // 三审3：publish 后的 fault（AfterPublishBeforeMarker / AfterAudit）会留下
        // 孤儿 StoryForge DB（带 authority_binding）→ inspect_marker 判 Stale
        // （ambiguous，比 Absent 更安全）；publish 前的 fault 无 DB → Absent。
        // 两者都不声称 SqliteAuthoritative（核心不变式）。
        let status = inspect_marker(&request.plan);
        let published_but_no_marker = matches!(
            fault,
            CutoverFault::AfterPublishBeforeMarker | CutoverFault::AfterAudit
        );
        if published_but_no_marker {
            assert!(
                matches!(status, MarkerStatus::Stale { .. }),
                "fault {fault:?}: orphan DB must be Stale, got {status:?}"
            );
        } else {
            assert_eq!(
                status,
                MarkerStatus::Absent,
                "fault {fault:?}: marker must be absent (no DB published)"
            );
        }
        // 原 JSON 不被修改或删除。
        assert_eq!(
            fs::read(dir.path().join("campaigns.json")).unwrap(),
            campaigns_before
        );
        assert_eq!(
            fs::read(dir.path().join("characters.json")).unwrap(),
            characters_before
        );
        // 无最终 DB（半迁移状态不得被误认为成功）。唯一例外：
        // AfterPublishBeforeMarker 是设计中的歧义窗口——DB 已发布但 marker
        // 未写，JSON 仍权威；marker 缺失保证不会被误认为成功，重试可收敛。
        if fault != CutoverFault::AfterPublishBeforeMarker {
            assert!(
                !db_path.exists(),
                "fault {fault:?}: final DB must not exist"
            );
        } else {
            assert!(
                !db_path.exists() || !request.plan.marker_path().exists(),
                "fault AfterPublishBeforeMarker: marker must never exist without a full commit"
            );
        }
        // 临时 DB 被清理。
        assert!(!request.plan.temp_db_path().exists());

        // backup 保留：只有进入备份阶段后的 fault 才产生 checkpoint。
        let backup_entries: Vec<_> = if dir.path().join("sqlite-backups").exists() {
            fs::read_dir(dir.path().join("sqlite-backups"))
                .unwrap()
                .flatten()
                .collect()
        } else {
            vec![]
        };
        let reached_backup = matches!(
            fault,
            CutoverFault::AfterBackup
                | CutoverFault::AfterVerify
                | CutoverFault::AfterPublishBeforeMarker
        );
        assert_eq!(
            !backup_entries.is_empty(),
            reached_backup,
            "fault {fault:?}: backup presence must match stage reached"
        );

        // 可安全重试并最终成功。
        let retry = recover_or_verify(&request).unwrap();
        assert!(
            matches!(retry, CutoverOutcome::Completed(_)),
            "fault {fault:?}: retry must complete, got {retry:?}"
        );
        assert!(db_path.exists());
        // marker 与数据库一致：marker 声称的 schema 版本与 DB 实际一致。
        match inspect_marker(&request.plan) {
            MarkerStatus::SqliteAuthoritative { schema_version, .. } => {
                let db = Database::open(&db_path).unwrap();
                assert_eq!(current_version(&db).unwrap(), schema_version);
            }
            other => panic!("marker must be sqlite authoritative after retry, got {other:?}"),
        }
    }
}

/// 4. 上一版 schema（v6）数据库可迁移到 v7 且数据保留。
#[test]
fn previous_schema_v6_migrates_to_v7_preserving_data() {
    let v1_to_v6: Vec<_> = builtin_migrations()
        .into_iter()
        .filter(|m| m.version <= 6)
        .collect();
    let db_dir = tempfile::tempdir().unwrap();
    let path = db_dir.path().join("v6.sqlite3");
    let mut db = Database::open(&path).unwrap();
    migrate_with(&mut db, &v1_to_v6).unwrap();
    assert_eq!(current_version(&db).unwrap(), 6);

    // 在 v6 上写入业务行（campaign + world info + compress job + card）。
    db.connection()
        .execute(
            "INSERT INTO character_cards (card_id, source_character_id, name, imported_at, payload_json) \
             VALUES ('card-x', 'src-x', 'X', '2026-01-01', '{\"card\":{\"id\":\"card-x\"},\"imported_at\":\"2026-01-01\"}')",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO campaigns (campaign_id, card_id, name, revision, chronicle_revision, story_clock, created_at, payload_json) \
             VALUES ('camp-x', 'card-x', 'X', 4, 1, 'Day 9', '2026-01-01', '{\"id\":\"camp-x\",\"story_clock\":\"Day 9\",\"variables\":[]}')",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO campaign_world_info (campaign_id, payload_json, updated_at) \
             VALUES ('camp-x', '{\"entries\":[]}', '2026-01-01')",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO chronicle_compress_jobs (job_id, campaign_id, kind, status, attempts, max_attempts, created_at, updated_at) \
             VALUES ('job-x', 'camp-x', 'auto', 'pending', 0, 5, '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();

    // v6 → latest：数据保留、schema 前进、新表可用。
    migrate(&mut db).unwrap();
    assert_eq!(current_version(&db).unwrap(), 8);
    let campaigns: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM campaigns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(campaigns, 1);
    let clock: String = db
        .connection()
        .query_row(
            "SELECT story_clock FROM campaigns WHERE campaign_id='camp-x'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(clock, "Day 9");
    let jobs: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM chronicle_compress_jobs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(jobs, 1);
    // v7 新表 characters 可写。
    db.connection()
        .execute(
            "INSERT INTO characters (character_id, source_character_id, name, info_json, imported_at) \
             VALUES ('c1', 'src-x', 'X', '{\"name\":\"X\"}', '2026-01-01')",
            [],
        )
        .unwrap();
    assert_eq!(
        storyforge_infra_sqlite::importer::table_count(&db, "characters").unwrap(),
        1
    );
}

/// 5. 损坏/缺失源文件 fail-closed：整体回滚、不产生半导入状态。
#[test]
fn corrupt_and_missing_sources_fail_closed() {
    // 损坏 campaigns.json
    let dir = tempfile::tempdir().unwrap();
    write_matrix_source(dir.path());
    fs::write(dir.path().join("campaigns.json"), "{ not-json").unwrap();
    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap_err();
    assert!(err.to_string().contains("corrupt") || err.to_string().contains("Corrupt"));
    for (table, _) in [
        ("character_cards", "cards"),
        ("campaigns", "campaigns"),
        ("characters", "characters"),
        ("character_instances", "instances"),
    ] {
        assert_eq!(
            storyforge_infra_sqlite::importer::table_count(&db, table).unwrap(),
            0,
            "corrupt source must leave {table} empty"
        );
    }

    // 损坏 characters.json（Gate 5 新集合）：同样整体回滚。
    let dir2 = tempfile::tempdir().unwrap();
    write_matrix_source(dir2.path());
    fs::write(dir2.path().join("characters.json"), "[{broken").unwrap();
    let mut db2 = Database::open_in_memory().unwrap();
    let err2 = JsonImporter::new(&mut db2)
        .import_data_dir(dir2.path())
        .unwrap_err();
    assert!(err2.to_string().contains("corrupt") || err2.to_string().contains("Corrupt"));
    assert_eq!(
        storyforge_infra_sqlite::importer::table_count(&db2, "campaigns").unwrap(),
        0,
        "corrupt characters.json must roll back the whole import"
    );

    // 缺失 characters.json = 可选集合，导入正常（turns 引用 conversations，
    // 故 conversations 目录不是可选——缺失会触发 FK，属预期 fail-closed）。
    let dir3 = tempfile::tempdir().unwrap();
    write_matrix_source(dir3.path());
    fs::remove_file(dir3.path().join("characters.json")).unwrap();
    let mut db3 = Database::open_in_memory().unwrap();
    let report = JsonImporter::new(&mut db3)
        .import_data_dir(dir3.path())
        .unwrap();
    assert_eq!(report.status, ImportStatus::Completed);
    assert_eq!(report.characters, 0);
    assert_eq!(report.conversations, 2);
    assert_eq!(report.campaigns, 2);
}

/// 6. 磁盘写失败（DB 路径被目录占用 → 打开/写入失败）fail-closed：
///    JSON 保持权威、marker 不写、可重试（列表续行缩进对齐）。
#[test]
fn disk_write_failure_leaves_json_authoritative() {
    let dir = tempfile::tempdir().unwrap();
    write_matrix_source(dir.path());
    let db_path = dir.path().join("storyforge.sqlite3");
    // 用目录占住临时 DB 路径：Database::open 在导入阶段必然失败（写入目标
    // 无法创建，等价磁盘写失败），错误来自注入阶段而非更早的校验分支。
    let temp_path = dir.path().join("storyforge.sqlite3.cutover-tmp");
    fs::create_dir(&temp_path).unwrap();
    let campaigns_before = fs::read(dir.path().join("campaigns.json")).unwrap();
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), &db_path),
        label: "gate5-disk-fail".into(),
    };
    let err = recover_or_verify(&request).expect_err("occupied temp db path must fail");
    assert!(
        err.to_string().contains("sqlite") || err.to_string().contains("open"),
        "错误必须来自数据库打开/写入阶段: {err}"
    );
    assert_eq!(inspect_marker(&request.plan), MarkerStatus::Absent);
    assert_eq!(
        fs::read(dir.path().join("campaigns.json")).unwrap(),
        campaigns_before
    );
    assert!(
        !db_path.exists(),
        "no final DB may exist after a write failure"
    );

    // 释放路径后重试成功。
    fs::remove_dir(&temp_path).unwrap();
    let retry = recover_or_verify(&request).unwrap();
    assert!(matches!(retry, CutoverOutcome::Completed(_)));
    assert!(matches!(
        inspect_marker(&request.plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
}
