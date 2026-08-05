use std::path::Path;

use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::connection::Database;
use crate::error::{Result, SqliteError};
use crate::migrations;
use crate::unit_of_work::UnitOfWork;

/// JSON → SQLite 一次性导入结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub run_id: String,
    pub source_manifest_hash: String,
    pub status: ImportStatus,
    pub cards: usize,
    pub campaigns: usize,
    pub instances: usize,
    pub knowledge: usize,
    pub tasks: usize,
    pub summaries: usize,
    pub conversations: usize,
    pub turns: usize,
    pub mvu_translations: usize,
    /// 导入的本局世界书条目数（Gate 4）。
    pub world_info: usize,
    /// 导入的 Chronicle 压缩任务数（Gate 4）。
    pub compress_jobs: usize,
    /// 导入的角色库条目数（V007 characters，Gate 5）。
    pub characters: usize,
    /// Gate 7 发现 #3：被跳过的孤儿行总数（父对象在源数据中不存在，
    /// JSON 应用里本就不可达；SQLite FK 拒绝插入，导入时跳过并计数）。
    pub skipped_orphan_rows: usize,
    pub skipped_as_duplicate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStatus {
    Completed,
    SkippedDuplicate,
}

/// 幂等 JSON importer：只读 JSON，写入 SQLite；不修改源文件。
pub struct JsonImporter<'a> {
    db: &'a mut Database,
}

impl<'a> JsonImporter<'a> {
    pub fn new(db: &'a mut Database) -> Self {
        Self { db }
    }

    /// 从 `data_dir` 导入核心 JSON 文件。
    ///
    /// 期望布局（缺失的可选文件按空集合处理）：
    /// - cards.json / campaigns.json / instances.json / knowledge.json
    /// - tasks.json / round_summaries.json / turns.json
    /// - conversations/*.json
    pub fn import_data_dir(&mut self, data_dir: impl AsRef<Path>) -> Result<ImportReport> {
        self.import_data_dir_inner(data_dir.as_ref(), || {})
    }

    #[cfg(test)]
    fn import_data_dir_after_precheck(
        &mut self,
        data_dir: impl AsRef<Path>,
        after_precheck: impl FnOnce(),
    ) -> Result<ImportReport> {
        self.import_data_dir_inner(data_dir.as_ref(), after_precheck)
    }

    fn import_data_dir_inner(
        &mut self,
        data_dir: &Path,
        after_precheck: impl FnOnce(),
    ) -> Result<ImportReport> {
        if !data_dir.exists() {
            return Err(SqliteError::ImportSourceMissing(data_dir.to_path_buf()));
        }

        migrations::migrate(self.db)?;

        let snapshot = read_source_snapshot(data_dir)?;
        let source_manifest_hash = snapshot.manifest_hash.clone();

        if let Some(run_id) = find_completed_run(self.db, &source_manifest_hash)? {
            return Ok(duplicate_report(run_id, &snapshot));
        }

        // Test hook makes the check-to-BEGIN race deterministic. Production uses a no-op.
        after_precheck();

        let run_id = new_id();
        let started_at = chrono::Utc::now().to_rfc3339();

        let result = self.import_in_transaction(&run_id, data_dir, &snapshot, &started_at);
        match result {
            Ok(report) => Ok(report),
            Err(e) => {
                // 尽力记录失败 run（独立自动提交语句；不影响已回滚的数据事务）
                let _ = self.db.connection().execute(
                    r#"
                    INSERT OR REPLACE INTO import_runs
                        (run_id, source_root, source_manifest_hash, status, started_at, finished_at, error)
                    VALUES (?1, ?2, ?3, 'failed', ?4, ?5, ?6)
                    "#,
                    rusqlite::params![
                        run_id,
                        data_dir.display().to_string(),
                        source_manifest_hash,
                        started_at,
                        chrono::Utc::now().to_rfc3339(),
                        e.to_string(),
                    ],
                );
                Err(e)
            }
        }
    }

    fn import_in_transaction(
        &mut self,
        run_id: &str,
        data_dir: &Path,
        snapshot: &SourceSnapshot,
        started_at: &str,
    ) -> Result<ImportReport> {
        let uow = UnitOfWork::begin(self.db.connection_mut())?;
        let completed_after_lock = {
            let tx = uow.transaction()?;
            find_completed_run_tx(tx, &snapshot.manifest_hash)?
        };
        if let Some(existing_run_id) = completed_after_lock {
            uow.commit()?;
            return Ok(duplicate_report(existing_run_id, snapshot));
        }
        {
            let tx = uow.transaction()?;
            tx.execute(
                r#"
                INSERT INTO import_runs
                    (run_id, source_root, source_manifest_hash, status, started_at, finished_at, error)
                VALUES (?1, ?2, ?3, 'running', ?4, NULL, NULL)
                "#,
                rusqlite::params![
                    run_id,
                    data_dir.display().to_string(),
                    snapshot.manifest_hash,
                    started_at,
                ],
            )?;

            for card in &snapshot.cards {
                upsert_card(tx, card)?;
            }
            for campaign in &snapshot.campaigns {
                upsert_campaign(tx, campaign)?;
            }
            for instance in &snapshot.instances {
                upsert_instance(tx, instance)?;
            }
            for entry in &snapshot.knowledge {
                upsert_knowledge(tx, entry)?;
            }
            for task in &snapshot.tasks {
                upsert_task(tx, task)?;
            }
            for conv in &snapshot.conversations {
                upsert_conversation(tx, conv)?;
            }

            // Fail closed with diagnostics before any summary/turn writes.
            reject_invalid_source_graphs(&snapshot.summaries, &snapshot.turns)?;

            // summaries 可能互相引用 covered_by / covers：
            // 1) 先写入行（covered_by 置空，避免插入顺序触发 FK）
            // 2) 回填 covered_by
            // 3) 写入 covers 关联
            for summary in &snapshot.summaries {
                upsert_summary(tx, summary)?;
            }
            for summary in &snapshot.summaries {
                backfill_summary_covered_by(tx, summary)?;
            }
            for summary in &snapshot.summaries {
                upsert_summary_covers(tx, summary)?;
            }
            for turn in &snapshot.turns {
                upsert_turn(tx, turn)?;
            }
            for mvu in &snapshot.mvu_translations {
                upsert_mvu_translation(tx, mvu)?;
            }
            for (campaign_id, book) in &snapshot.world_info {
                upsert_world_info(tx, campaign_id, book)?;
            }
            for job in &snapshot.compress_jobs {
                upsert_compress_job(tx, job)?;
            }
            for character in &snapshot.characters {
                upsert_character(tx, character)?;
            }

            tx.execute(
                r#"
                UPDATE import_runs
                SET status = 'completed', finished_at = ?1, error = NULL
                WHERE run_id = ?2
                "#,
                rusqlite::params![chrono::Utc::now().to_rfc3339(), run_id],
            )?;
        }
        uow.commit()?;

        Ok(ImportReport {
            run_id: run_id.to_string(),
            source_manifest_hash: snapshot.manifest_hash.clone(),
            status: ImportStatus::Completed,
            cards: snapshot.cards.len(),
            campaigns: snapshot.campaigns.len(),
            instances: snapshot.instances.len(),
            knowledge: snapshot.knowledge.len(),
            tasks: snapshot.tasks.len(),
            summaries: snapshot.summaries.len(),
            conversations: snapshot.conversations.len(),
            turns: snapshot.turns.len(),
            mvu_translations: snapshot.mvu_translations.len(),
            world_info: snapshot.world_info.len(),
            compress_jobs: snapshot.compress_jobs.len(),
            characters: snapshot.characters.len(),
            skipped_orphan_rows: snapshot.skipped_orphan_rows,
            skipped_as_duplicate: false,
        })
    }
}

fn duplicate_report(run_id: String, snapshot: &SourceSnapshot) -> ImportReport {
    ImportReport {
        run_id,
        source_manifest_hash: snapshot.manifest_hash.clone(),
        status: ImportStatus::SkippedDuplicate,
        cards: snapshot.cards.len(),
        campaigns: snapshot.campaigns.len(),
        instances: snapshot.instances.len(),
        knowledge: snapshot.knowledge.len(),
        tasks: snapshot.tasks.len(),
        summaries: snapshot.summaries.len(),
        conversations: snapshot.conversations.len(),
        turns: snapshot.turns.len(),
        mvu_translations: snapshot.mvu_translations.len(),
        world_info: snapshot.world_info.len(),
        compress_jobs: snapshot.compress_jobs.len(),
        characters: snapshot.characters.len(),
        skipped_orphan_rows: snapshot.skipped_orphan_rows,
        skipped_as_duplicate: true,
    }
}

#[derive(Debug)]
struct SourceSnapshot {
    manifest_hash: String,
    cards: Vec<Value>,
    campaigns: Vec<Value>,
    instances: Vec<Value>,
    knowledge: Vec<Value>,
    tasks: Vec<Value>,
    summaries: Vec<Value>,
    conversations: Vec<Value>,
    turns: Vec<Value>,
    mvu_translations: Vec<Value>,
    /// (campaign_id, WorldInfoBook payload) — 本局世界书（Gate 4）。
    world_info: Vec<(String, Value)>,
    /// compress_jobs.json 中的 CompressJob 条目（Gate 4）。
    compress_jobs: Vec<Value>,
    /// characters.json 中的 StoredCharacter 条目（Gate 5，归一化为
    /// `{id, info, imported_at}` 三字段契约形态）。
    characters: Vec<Value>,
    /// Gate 7 发现 #3：被跳过的孤儿行总数（父对象在源数据中不存在）。
    skipped_orphan_rows: usize,
}

fn read_source_snapshot(data_dir: &Path) -> Result<SourceSnapshot> {
    // 委托 readiness 的单一权威快照构建：读取（缺失 = 空，Gate 7 发现 #1）
    // → 严格校验（原始数组）→ 孤儿行过滤（发现 #3）→ 同口径 manifest hash。
    // importer 与 readiness 的 counts/hash 由此保证完全一致（verify 依赖）。
    let snapshot = crate::readiness::build_import_snapshot(data_dir)?;
    Ok(SourceSnapshot {
        manifest_hash: snapshot.manifest_hash,
        cards: snapshot.cards,
        campaigns: snapshot.campaigns,
        instances: snapshot.instances,
        knowledge: snapshot.knowledge,
        tasks: snapshot.tasks,
        summaries: snapshot.summaries,
        conversations: snapshot.conversations,
        turns: snapshot.turns,
        mvu_translations: snapshot.mvu_translations,
        world_info: snapshot.world_info,
        compress_jobs: snapshot.compress_jobs,
        characters: snapshot.characters,
        skipped_orphan_rows: snapshot.skipped_orphan_rows,
    })
}

fn stable_json(value: &Value) -> String {
    // serde_json Value 序列化对 object 键有序（BTreeMap），足够稳定
    serde_json::to_string(value).unwrap_or_default()
}

fn find_completed_run(db: &Database, hash: &str) -> Result<Option<String>> {
    let mut stmt = db.connection().prepare(
        r#"
        SELECT run_id FROM import_runs
        WHERE source_manifest_hash = ?1 AND status = 'completed'
        ORDER BY finished_at DESC
        LIMIT 1
        "#,
    )?;
    let run_id = stmt
        .query_row(rusqlite::params![hash], |row| row.get(0))
        .optional()?;
    Ok(run_id)
}

fn find_completed_run_tx(tx: &rusqlite::Transaction<'_>, hash: &str) -> Result<Option<String>> {
    tx.query_row(
        r#"
        SELECT run_id FROM import_runs
        WHERE source_manifest_hash = ?1 AND status = 'completed'
        ORDER BY finished_at DESC
        LIMIT 1
        "#,
        [hash],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn upsert_card(tx: &rusqlite::Transaction<'_>, card: &Value) -> Result<()> {
    // StoredCard { card: CharacterCard, imported_at }
    let inner = card.get("card").unwrap_or(card);
    let card_id = required_str(inner, "id", "card")?;
    let name = optional_str(inner, "name").unwrap_or_default();
    let source_character_id = optional_str(inner, "source_character_id");
    let imported_at = optional_str(card, "imported_at");
    let payload = stable_json(card);
    tx.execute(
        r#"
        INSERT INTO character_cards (card_id, source_character_id, name, imported_at, payload_json)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(card_id) DO UPDATE SET
            source_character_id = excluded.source_character_id,
            name = excluded.name,
            imported_at = excluded.imported_at,
            payload_json = excluded.payload_json
        "#,
        rusqlite::params![card_id, source_character_id, name, imported_at, payload],
    )?;
    Ok(())
}

/// Gate 4：本局世界书（JSON 布局 campaign_world_info/{campaign_id}.json）。
fn upsert_world_info(
    tx: &rusqlite::Transaction<'_>,
    campaign_id: &str,
    book: &Value,
) -> Result<()> {
    tx.execute(
        r#"
        INSERT INTO campaign_world_info (campaign_id, payload_json, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(campaign_id) DO UPDATE SET
            payload_json = excluded.payload_json,
            updated_at = excluded.updated_at
        "#,
        rusqlite::params![
            campaign_id,
            stable_json(book),
            chrono::Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Gate 4：Chronicle 压缩任务（JSON 布局 compress_jobs.json，CompressJob 形态）。
fn upsert_compress_job(tx: &rusqlite::Transaction<'_>, job: &Value) -> Result<()> {
    let job_id = required_str(job, "id", "compress_job")?;
    let campaign_id = required_str(job, "campaign_id", "compress_job")?;
    let conversation_id = optional_str(job, "conversation_id");
    let lineage_id = optional_str(job, "lineage_id");
    let kind = optional_str(job, "kind").unwrap_or_else(|| "auto".to_string());
    let status = optional_str(job, "status").unwrap_or_else(|| "pending".to_string());
    let attempts = optional_u64(job, "attempts").unwrap_or(0) as i64;
    let max_attempts = optional_u64(job, "max_attempts").unwrap_or(5) as i64;
    let last_error = optional_str(job, "last_error");
    let uncovered_a = optional_u64(job, "uncovered_a_at_enqueue").unwrap_or(0) as i64;
    let uncovered_b = optional_u64(job, "uncovered_b_at_enqueue").unwrap_or(0) as i64;
    let created_at = optional_str(job, "created_at").unwrap_or_default();
    let updated_at = optional_str(job, "updated_at").unwrap_or_default();
    tx.execute(
        r#"
        INSERT INTO chronicle_compress_jobs (
            job_id, campaign_id, conversation_id, lineage_id, kind, status, attempts,
            max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue,
            created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ON CONFLICT(job_id) DO UPDATE SET
            campaign_id = excluded.campaign_id,
            conversation_id = excluded.conversation_id,
            lineage_id = excluded.lineage_id,
            kind = excluded.kind,
            status = excluded.status,
            attempts = excluded.attempts,
            max_attempts = excluded.max_attempts,
            last_error = excluded.last_error,
            uncovered_a_at_enqueue = excluded.uncovered_a_at_enqueue,
            uncovered_b_at_enqueue = excluded.uncovered_b_at_enqueue,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at
        "#,
        rusqlite::params![
            job_id,
            campaign_id,
            conversation_id,
            lineage_id,
            kind,
            status,
            attempts,
            max_attempts,
            last_error,
            uncovered_a,
            uncovered_b,
            created_at,
            updated_at,
        ],
    )?;
    Ok(())
}

/// Gate 5：角色库（V007 characters 表，JSON 布局 characters.json 的
/// `StoredCharacter` 条目）。条目在 readiness 归一化为 `{id, info, imported_at}`
/// 契约形态；此处从归一化条目拆列，info_json 保持 info 对象原样。
fn upsert_character(tx: &rusqlite::Transaction<'_>, entry: &Value) -> Result<()> {
    let character_id = required_str(entry, "id", "character")?;
    let info = entry
        .get("info")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            SqliteError::CorruptImportInput("character entry missing object field 'info'".into())
        })?;
    let source_character_id = info
        .get("source_character_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let name = info
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let imported_at = optional_str(entry, "imported_at").unwrap_or_default();
    let info_json = stable_json(&Value::Object(info.clone()));
    tx.execute(
        r#"
        INSERT INTO characters (character_id, source_character_id, name, info_json, imported_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(character_id) DO UPDATE SET
            source_character_id = excluded.source_character_id,
            name = excluded.name,
            info_json = excluded.info_json,
            imported_at = excluded.imported_at
        "#,
        rusqlite::params![
            character_id,
            source_character_id,
            name,
            info_json,
            imported_at,
        ],
    )?;
    Ok(())
}

/// StoredMvuTranslation { source_character_id, character_name, translation, analyzed_at }
fn upsert_mvu_translation(tx: &rusqlite::Transaction<'_>, mvu: &Value) -> Result<()> {
    let source_character_id = required_str(mvu, "source_character_id", "mvu_translation")?;
    let character_name = optional_str(mvu, "character_name").unwrap_or_default();
    let updated_at = optional_str(mvu, "analyzed_at").unwrap_or_default();
    let payload = stable_json(mvu);
    tx.execute(
        r#"
        INSERT INTO mvu_translations (source_character_id, character_name, payload_json, updated_at)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(source_character_id) DO UPDATE SET
            character_name = excluded.character_name,
            payload_json = excluded.payload_json,
            updated_at = excluded.updated_at
        "#,
        rusqlite::params![source_character_id, character_name, payload, updated_at],
    )?;
    Ok(())
}

/// Contract / 内部适配用：在已有事务中 upsert campaign。
pub(crate) fn upsert_campaign_for_contract(
    tx: &rusqlite::Transaction<'_>,
    campaign: &Value,
) -> Result<()> {
    upsert_campaign(tx, campaign)
}

fn upsert_campaign(tx: &rusqlite::Transaction<'_>, campaign: &Value) -> Result<()> {
    let campaign_id = required_str(campaign, "id", "campaign")?;
    let card_id = required_str(campaign, "card_id", "campaign")?;
    let name = optional_str(campaign, "name").unwrap_or_default();
    let conversation_id = optional_str(campaign, "conversation_id");
    let revision = optional_u64(campaign, "revision").unwrap_or(0) as i64;
    let chronicle_revision = optional_u64(campaign, "chronicle_revision").unwrap_or(0) as i64;
    let lineage_id = optional_str(campaign, "lineage_id");
    // Gate 4 story-clock authority：索引列取 variables["story_clock"] 权威值，
    // 仅在缺失时回退旧顶层字段——与运行时 `current_story_clock()` 同口径。
    let story_clock = campaign
        .get("variables")
        .and_then(|v| v.as_array())
        .and_then(|variables| {
            variables
                .iter()
                .find(|v| v.get("key").and_then(|k| k.as_str()) == Some("story_clock"))
        })
        .and_then(|v| {
            v.get("value")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .or_else(|| optional_str(campaign, "story_clock"))
        .unwrap_or_else(|| "Day 1".to_string());
    let created_at = optional_str(campaign, "created_at").unwrap_or_default();
    let payload = stable_json(campaign);
    tx.execute(
        r#"
        INSERT INTO campaigns (
            campaign_id, card_id, name, conversation_id, revision, chronicle_revision,
            lineage_id, story_clock, created_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(campaign_id) DO UPDATE SET
            card_id = excluded.card_id,
            name = excluded.name,
            conversation_id = excluded.conversation_id,
            revision = excluded.revision,
            chronicle_revision = excluded.chronicle_revision,
            lineage_id = excluded.lineage_id,
            story_clock = excluded.story_clock,
            created_at = excluded.created_at,
            payload_json = excluded.payload_json
        "#,
        rusqlite::params![
            campaign_id,
            card_id,
            name,
            conversation_id,
            revision,
            chronicle_revision,
            lineage_id,
            story_clock,
            created_at,
            payload
        ],
    )?;
    Ok(())
}

fn upsert_instance(tx: &rusqlite::Transaction<'_>, instance: &Value) -> Result<()> {
    let instance_id = required_str(instance, "id", "instance")?;
    let campaign_id = required_str(instance, "campaign_id", "instance")?;
    let definition_id = optional_str(instance, "definition_id");
    let name = optional_str(instance, "name").unwrap_or_default();
    let is_temporary = optional_bool(instance, "is_temporary") as i64;
    let payload = stable_json(instance);
    tx.execute(
        r#"
        INSERT INTO character_instances (
            instance_id, campaign_id, definition_id, name, is_temporary, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(instance_id) DO UPDATE SET
            campaign_id = excluded.campaign_id,
            definition_id = excluded.definition_id,
            name = excluded.name,
            is_temporary = excluded.is_temporary,
            payload_json = excluded.payload_json
        "#,
        rusqlite::params![
            instance_id,
            campaign_id,
            definition_id,
            name,
            is_temporary,
            payload
        ],
    )?;
    Ok(())
}

fn upsert_knowledge(tx: &rusqlite::Transaction<'_>, entry: &Value) -> Result<()> {
    let knowledge_id = required_str(entry, "id", "knowledge")?;
    let campaign_id = required_str(entry, "campaign_id", "knowledge")?;
    let payload = stable_json(entry);
    tx.execute(
        r#"
        INSERT INTO character_knowledge (knowledge_id, campaign_id, payload_json)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(knowledge_id) DO UPDATE SET
            campaign_id = excluded.campaign_id,
            payload_json = excluded.payload_json
        "#,
        rusqlite::params![knowledge_id, campaign_id, payload],
    )?;
    Ok(())
}

fn upsert_task(tx: &rusqlite::Transaction<'_>, task: &Value) -> Result<()> {
    let task_id = required_str(task, "id", "task")?;
    let campaign_id = required_str(task, "campaign_id", "task")?;
    let payload = stable_json(task);
    tx.execute(
        r#"
        INSERT INTO story_tasks (task_id, campaign_id, payload_json)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(task_id) DO UPDATE SET
            campaign_id = excluded.campaign_id,
            payload_json = excluded.payload_json
        "#,
        rusqlite::params![task_id, campaign_id, payload],
    )?;
    Ok(())
}

fn upsert_conversation(tx: &rusqlite::Transaction<'_>, conv: &Value) -> Result<()> {
    let conversation_id = required_str(conv, "id", "conversation")?;
    let campaign_id = optional_str(conv, "campaign_id");
    let character_id = optional_str(conv, "character_id");
    let archived_upto = optional_u64(conv, "archived_upto").unwrap_or(0) as i64;
    let created_at = optional_str(conv, "created_at").unwrap_or_default();
    let updated_at = optional_str(conv, "updated_at").unwrap_or_default();
    let payload = stable_json(conv);
    tx.execute(
        r#"
        INSERT INTO conversations (
            conversation_id, campaign_id, character_id, archived_upto,
            created_at, updated_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(conversation_id) DO UPDATE SET
            campaign_id = excluded.campaign_id,
            character_id = excluded.character_id,
            archived_upto = excluded.archived_upto,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            payload_json = excluded.payload_json
        "#,
        rusqlite::params![
            conversation_id,
            campaign_id,
            character_id,
            archived_upto,
            created_at,
            updated_at,
            payload
        ],
    )?;
    Ok(())
}

fn upsert_summary(tx: &rusqlite::Transaction<'_>, summary: &Value) -> Result<()> {
    let summary_id = required_str(summary, "id", "round_summary")?;
    let campaign_id = required_str(summary, "campaign_id", "round_summary")?;
    let conversation_id = required_str(summary, "conversation_id", "round_summary")?;
    let lineage_id = optional_str(summary, "lineage_id");
    let level = optional_u64(summary, "level").unwrap_or(0) as i64;
    let turn = optional_u64(summary, "turn").unwrap_or(0) as i64;
    let turn_end = optional_u64(summary, "turn_end").unwrap_or(0) as i64;
    let code = optional_str(summary, "code");
    let headline = optional_str(summary, "headline");
    // covered_by 第二遍回填，避免同批插入顺序触发 FK
    let content = optional_str(summary, "content").unwrap_or_default();
    let created_at = optional_str(summary, "created_at").unwrap_or_default();
    let payload = stable_json(summary);
    tx.execute(
        r#"
        INSERT INTO round_summaries (
            summary_id, campaign_id, conversation_id, lineage_id, level, turn, turn_end,
            code, headline, covered_by, content, created_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12)
        ON CONFLICT(summary_id) DO UPDATE SET
            campaign_id = excluded.campaign_id,
            conversation_id = excluded.conversation_id,
            lineage_id = excluded.lineage_id,
            level = excluded.level,
            turn = excluded.turn,
            turn_end = excluded.turn_end,
            code = excluded.code,
            headline = excluded.headline,
            content = excluded.content,
            created_at = excluded.created_at,
            payload_json = excluded.payload_json
        "#,
        rusqlite::params![
            summary_id,
            campaign_id,
            conversation_id,
            lineage_id,
            level,
            turn,
            turn_end,
            code,
            headline,
            content,
            created_at,
            payload
        ],
    )?;
    Ok(())
}

fn backfill_summary_covered_by(tx: &rusqlite::Transaction<'_>, summary: &Value) -> Result<()> {
    let summary_id = required_str(summary, "id", "round_summary")?;
    let covered_by = optional_str(summary, "covered_by");
    tx.execute(
        "UPDATE round_summaries SET covered_by = ?1 WHERE summary_id = ?2",
        rusqlite::params![covered_by, summary_id],
    )?;
    Ok(())
}

fn upsert_summary_covers(tx: &rusqlite::Transaction<'_>, summary: &Value) -> Result<()> {
    let parent_id = required_str(summary, "id", "round_summary")?;
    tx.execute(
        "DELETE FROM round_summary_covers WHERE parent_id = ?1",
        rusqlite::params![parent_id],
    )?;
    if let Some(Value::Array(covers)) = summary.get("covers") {
        for child in covers {
            let child_id = child.as_str().ok_or_else(|| {
                SqliteError::CorruptImportInput("covers item must be string".into())
            })?;
            tx.execute(
                r#"
                INSERT OR IGNORE INTO round_summary_covers (parent_id, child_id)
                VALUES (?1, ?2)
                "#,
                rusqlite::params![parent_id, child_id],
            )?;
        }
    }
    Ok(())
}

fn upsert_turn(tx: &rusqlite::Transaction<'_>, turn: &Value) -> Result<()> {
    let turn_id = required_str(turn, "turn_id", "turn")?;
    let campaign_id = required_str(turn, "campaign_id", "turn")?;
    let conversation_id = required_str(turn, "conversation_id", "turn")?;
    let input_node_id = required_str(turn, "input_node_id", "turn")?;
    let base_campaign_revision = optional_u64(turn, "base_campaign_revision").unwrap_or(0) as i64;
    let status = optional_str(turn, "status").unwrap_or_else(|| "generating".into());
    let accepted_attempt_id = optional_str(turn, "accepted_attempt_id");
    let failure_reason = optional_str(turn, "failure_reason");
    let created_at = optional_str(turn, "created_at").unwrap_or_default();
    let updated_at = optional_str(turn, "updated_at").unwrap_or_default();
    let payload = stable_json(turn);
    tx.execute(
        r#"
        INSERT INTO turns (
            turn_id, campaign_id, conversation_id, input_node_id, base_campaign_revision,
            status, accepted_attempt_id, failure_reason, created_at, updated_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(turn_id) DO UPDATE SET
            campaign_id = excluded.campaign_id,
            conversation_id = excluded.conversation_id,
            input_node_id = excluded.input_node_id,
            base_campaign_revision = excluded.base_campaign_revision,
            status = excluded.status,
            accepted_attempt_id = excluded.accepted_attempt_id,
            failure_reason = excluded.failure_reason,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            payload_json = excluded.payload_json
        "#,
        rusqlite::params![
            turn_id,
            campaign_id,
            conversation_id,
            input_node_id,
            base_campaign_revision,
            status,
            accepted_attempt_id,
            failure_reason,
            created_at,
            updated_at,
            payload
        ],
    )?;

    if let Some(Value::Array(attempts)) = turn.get("attempts") {
        for attempt in attempts {
            upsert_attempt(tx, turn_id, attempt)?;
        }
    }
    Ok(())
}

fn upsert_attempt(tx: &rusqlite::Transaction<'_>, turn_id: &str, attempt: &Value) -> Result<()> {
    let attempt_id = required_str(attempt, "attempt_id", "turn_attempt")?;
    let existing_owner: Option<String> = tx
        .query_row(
            "SELECT turn_id FROM turn_attempts WHERE attempt_id = ?1",
            [attempt_id],
            |row| row.get(0),
        )
        .optional()?;
    if existing_owner
        .as_deref()
        .is_some_and(|owner| owner != turn_id)
    {
        return Err(SqliteError::CorruptImportInput(format!(
            "duplicate attempt ownership: attempt {attempt_id} already owned by turn {}, refusing rehang onto {turn_id}",
            existing_owner.as_deref().unwrap_or_default()
        )));
    }
    let variant_id = required_str(attempt, "variant_id", "turn_attempt")?;
    let draft_hash = optional_str(attempt, "draft_hash").unwrap_or_default();
    let status = optional_str(attempt, "status").unwrap_or_else(|| "generating".into());
    let created_at = optional_str(attempt, "created_at").unwrap_or_default();
    let payload = stable_json(attempt);
    tx.execute(
        r#"
        INSERT INTO turn_attempts (
            attempt_id, turn_id, variant_id, draft_hash, status, created_at, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(attempt_id) DO UPDATE SET
            turn_id = excluded.turn_id,
            variant_id = excluded.variant_id,
            draft_hash = excluded.draft_hash,
            status = excluded.status,
            created_at = excluded.created_at,
            payload_json = excluded.payload_json
        "#,
        rusqlite::params![
            attempt_id, turn_id, variant_id, draft_hash, status, created_at, payload
        ],
    )?;
    Ok(())
}

fn reject_invalid_source_graphs(summaries: &[Value], turns: &[Value]) -> Result<()> {
    let mut issues = crate::readiness::validate_summary_graph(summaries);
    issues.extend(crate::readiness::validate_attempt_ownership(turns));
    if issues.is_empty() {
        return Ok(());
    }
    Err(SqliteError::CorruptImportInput(issues.join("; ")))
}

fn required_str<'v>(value: &'v Value, key: &str, entity: &str) -> Result<&'v str> {
    value.get(key).and_then(|v| v.as_str()).ok_or_else(|| {
        SqliteError::CorruptImportInput(format!("{entity} missing string field '{key}'"))
    })
}

fn optional_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    })
}

fn optional_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_i64().map(|i| i.max(0) as u64))
            .or_else(|| v.as_str()?.parse().ok())
    })
}

fn optional_bool(value: &Value, key: &str) -> bool {
    value.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn new_id() -> String {
    // 不强制依赖 uuid crate：用时间+随机填充足够做 run_id
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("import-{nanos:x}")
}

/// 测试辅助：统计表行数。
pub fn table_count(db: &Database, table: &str) -> Result<i64> {
    // table 名仅内部白名单调用
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let n = db.connection().query_row(&sql, [], |r| r.get(0))?;
    Ok(n)
}

/// 测试辅助：读取 payload 以便契约比对。
pub fn load_payload(
    db: &Database,
    table: &str,
    id_column: &str,
    id: &str,
) -> Result<Option<Value>> {
    let sql = format!("SELECT payload_json FROM {table} WHERE {id_column} = ?1");
    let mut stmt = db.connection().prepare(&sql)?;
    let text: Option<String> = stmt
        .query_row(rusqlite::params![id], |r| r.get(0))
        .optional()?;
    match text {
        Some(t) => Ok(Some(serde_json::from_str(&t)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Database;
    use serde_json::json;
    use std::fs;
    use std::sync::{Arc, Barrier};
    use tempfile::TempDir;

    fn write_json(path: &Path, value: &Value) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn sample_data_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        write_json(
            &root.join("cards.json"),
            &json!([{
                "card": {
                    "id": "card-1",
                    "name": "Demo Card",
                    "source_character_id": "char-1",
                    "character_definitions": []
                },
                "imported_at": "2026-07-13T00:00:00Z"
            }]),
        );
        write_json(
            &root.join("campaigns.json"),
            &json!([{
                "id": "camp-1",
                "card_id": "card-1",
                "name": "Demo Campaign",
                "created_at": "2026-07-13T00:00:00Z",
                "conversation_id": "conv-1",
                "revision": 2,
                "chronicle_revision": 1,
                "lineage_id": "lin-1",
                "story_clock": "Day 2",
                "variables": []
            }]),
        );
        write_json(
            &root.join("instances.json"),
            &json!([{
                "id": "inst-1",
                "campaign_id": "camp-1",
                "definition_id": "def-1",
                "name": "Alice",
                "is_temporary": false,
                "variables": []
            }]),
        );
        write_json(
            &root.join("knowledge.json"),
            &json!([{
                "id": "know-1",
                "campaign_id": "camp-1",
                "content": "knows the password"
            }]),
        );
        write_json(
            &root.join("tasks.json"),
            &json!([{
                "id": "task-1",
                "campaign_id": "camp-1",
                "title": "Find the key"
            }]),
        );
        write_json(
            &root.join("round_summaries.json"),
            &json!([
                {
                    "id": "sum-a",
                    "campaign_id": "camp-1",
                    "conversation_id": "conv-1",
                    "turn": 1,
                    "content": "leaf A",
                    "created_at": "2026-07-13T00:00:00Z",
                    "level": 0,
                    "lineage_id": "lin-1",
                    "code": "A0001",
                    "covered_by": "sum-b"
                },
                {
                    "id": "sum-b",
                    "campaign_id": "camp-1",
                    "conversation_id": "conv-1",
                    "turn": 1,
                    "turn_end": 1,
                    "content": "stage B",
                    "created_at": "2026-07-13T00:01:00Z",
                    "level": 1,
                    "lineage_id": "lin-1",
                    "code": "B0001",
                    "covers": ["sum-a"]
                }
            ]),
        );
        // covers 与 covered_by 双向一致（importer 诊断要求）

        write_json(
            &root.join("conversations").join("conv-1.json"),
            &json!({
                "id": "conv-1",
                "campaign_id": "camp-1",
                "character_id": "char-1",
                "nodes": [],
                "archived_upto": 0,
                "created_at": "2026-07-13T00:00:00Z",
                "updated_at": "2026-07-13T00:00:00Z"
            }),
        );
        write_json(
            &root.join("turns.json"),
            &json!([{
                "turn_id": "turn-1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "input_node_id": "node-1",
                "base_campaign_revision": 1,
                "status": "committed",
                "attempts": [{
                    "attempt_id": "att-1",
                    "variant_id": "var-1",
                    "draft_hash": "abc",
                    "status": "committed",
                    "created_at": "2026-07-13T00:00:00Z"
                }],
                "accepted_attempt_id": "att-1",
                "failure_reason": null,
                "created_at": "2026-07-13T00:00:00Z",
                "updated_at": "2026-07-13T00:00:00Z"
            }]),
        );
        dir
    }

    /// #22：mvu_translations.json 进 SQLite 权威表；无该文件的老目录 hash 不变。
    #[test]
    fn imports_mvu_translations_and_keeps_legacy_hash_stable() {
        let data = sample_data_dir();
        let hash_without_mvu = {
            let mut db = Database::open_in_memory().unwrap();
            let report = JsonImporter::new(&mut db)
                .import_data_dir(data.path())
                .unwrap();
            assert_eq!(report.mvu_translations, 0);
            assert_eq!(table_count(&db, "mvu_translations").unwrap(), 0);
            report.source_manifest_hash
        };

        // 老目录（无 mvu_translations.json）的 hash 必须与加字段前一致：
        // 用相同内容重新读 snapshot，确认 hash 未受可选文件缺失影响。
        let snapshot = read_source_snapshot(data.path()).unwrap();
        assert_eq!(snapshot.manifest_hash, hash_without_mvu);

        write_json(
            &data.path().join("mvu_translations.json"),
            &json!([{
                "source_character_id": "char-1",
                "character_name": "Alice",
                "analyzed_at": "2026-07-27T00:00:00Z",
                "translation": {
                    "update_rules": ["damage reduces hp"],
                    "fallback_fragments": [],
                    "ui_bindings": []
                }
            }]),
        );
        let mut db = Database::open_in_memory().unwrap();
        let report = JsonImporter::new(&mut db)
            .import_data_dir(data.path())
            .unwrap();
        assert_eq!(report.mvu_translations, 1);
        assert_ne!(report.source_manifest_hash, hash_without_mvu);
        assert_eq!(table_count(&db, "mvu_translations").unwrap(), 1);
        let stored: String = db
            .connection()
            .query_row(
                "SELECT character_name FROM mvu_translations WHERE source_character_id = 'char-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "Alice");
    }

    #[test]
    fn import_is_idempotent_across_two_runs() {
        let data = sample_data_dir();
        let mut db = Database::open_in_memory().unwrap();

        let r1 = JsonImporter::new(&mut db)
            .import_data_dir(data.path())
            .unwrap();
        assert!(!r1.skipped_as_duplicate);
        assert_eq!(r1.campaigns, 1);
        assert_eq!(r1.conversations, 1);
        assert_eq!(r1.turns, 1);
        assert_eq!(table_count(&db, "campaigns").unwrap(), 1);
        assert_eq!(table_count(&db, "turn_attempts").unwrap(), 1);
        assert_eq!(table_count(&db, "round_summary_covers").unwrap(), 1);

        let r2 = JsonImporter::new(&mut db)
            .import_data_dir(data.path())
            .unwrap();
        assert!(r2.skipped_as_duplicate);
        assert_eq!(r2.source_manifest_hash, r1.source_manifest_hash);
        assert_eq!(table_count(&db, "campaigns").unwrap(), 1);
        assert_eq!(table_count(&db, "turns").unwrap(), 1);
        assert_eq!(table_count(&db, "import_runs").unwrap(), 1);
    }

    #[test]
    fn concurrent_same_manifest_converges_to_one_completed_run() {
        let data = sample_data_dir();
        let db_dir = TempDir::new().unwrap();
        let db_path = db_dir.path().join("concurrent-import.sqlite3");
        let barrier = Arc::new(Barrier::new(2));

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let source = data.path().to_path_buf();
                let db_path = db_path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let mut db = Database::open(db_path).unwrap();
                    JsonImporter::new(&mut db)
                        .import_data_dir_after_precheck(&source, || {
                            barrier.wait();
                        })
                        .unwrap()
                })
            })
            .collect();

        let reports: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            reports
                .iter()
                .filter(|report| report.status == ImportStatus::Completed)
                .count(),
            1
        );
        assert_eq!(
            reports
                .iter()
                .filter(|report| report.status == ImportStatus::SkippedDuplicate)
                .count(),
            1
        );

        let db = Database::open(db_path).unwrap();
        let completed: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM import_runs WHERE status = 'completed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(completed, 1);
    }

    #[test]
    fn corrupt_input_fails_and_leaves_no_partial_import() {
        let data = sample_data_dir();
        fs::write(data.path().join("campaigns.json"), "{ not-json").unwrap();

        let mut db = Database::open_in_memory().unwrap();
        let err = JsonImporter::new(&mut db)
            .import_data_dir(data.path())
            .unwrap_err();
        assert!(matches!(err, SqliteError::CorruptImportInput(_)));
        // migrate 可能已执行，但业务表应为空
        assert_eq!(table_count(&db, "campaigns").unwrap(), 0);
        assert_eq!(table_count(&db, "turns").unwrap(), 0);
    }

    #[test]
    fn transaction_rollback_on_fk_violation_mid_import() {
        // campaigns 引用不存在的 card → FK 失败 → 整单回滚
        // （布局文件齐全，缺失文件的场景已由 strict-layout 测试覆盖）
        let dir = TempDir::new().unwrap();
        write_json(
            &dir.path().join("cards.json"),
            &json!([{
                "id": "card-1", "name": "Hero", "source_character_id": "char-1",
                "character_definitions": []
            }]),
        );
        write_json(
            &dir.path().join("campaigns.json"),
            &json!([{
                "id": "camp-x",
                "card_id": "missing-card",
                "name": "X",
                "created_at": "2026-07-13T00:00:00Z",
                "story_clock": "Day 1",
                "variables": []
            }]),
        );
        for name in [
            "instances.json",
            "knowledge.json",
            "tasks.json",
            "round_summaries.json",
            "turns.json",
        ] {
            write_json(&dir.path().join(name), &json!([]));
        }
        std::fs::create_dir_all(dir.path().join("conversations")).unwrap();

        let mut db = Database::open_in_memory().unwrap();
        let err = JsonImporter::new(&mut db)
            .import_data_dir(dir.path())
            .unwrap_err();
        assert!(err.to_string().contains("sqlite") || err.to_string().contains("FOREIGN"));
        assert_eq!(table_count(&db, "campaigns").unwrap(), 0);
    }

    #[test]
    fn payload_roundtrip_preserves_campaign_fields() {
        let data = sample_data_dir();
        let mut db = Database::open_in_memory().unwrap();
        JsonImporter::new(&mut db)
            .import_data_dir(data.path())
            .unwrap();
        let payload = load_payload(&db, "campaigns", "campaign_id", "camp-1")
            .unwrap()
            .unwrap();
        assert_eq!(payload["name"], "Demo Campaign");
        assert_eq!(payload["revision"], 2);
        assert_eq!(payload["lineage_id"], "lin-1");
    }
}
