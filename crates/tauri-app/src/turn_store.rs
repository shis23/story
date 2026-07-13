//! Turn 提交屏障持久化（Phase A）
//!
//! `TurnStore` 负责持久化 `TurnRecord`（含 `TurnAttempt` 和 `MutationBatch`），
//! 提供状态转换 CAS、崩溃恢复扫描和活动 Turn 查询。
//!
//! 设计原则（收敛决策步骤 4）：
//! - 独立于 `CampaignStore`，避免"同一 Store = 一项事务"的错觉。
//! - 复用 `infra-util` 的 `atomic_write_json` 和 `json_store` 的 .tmp 恢复。
//! - write-ahead journal：在任何副作用发生前，Prepared MutationBatch 已原子落盘。
//! - 崩溃恢复读取同一 MutationBatch，自然复用同一批 ID。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use storyforge_domain::Id;
use storyforge_domain::turn::{TurnRecord, TurnStatus};

pub struct TurnStore {
    turns: Mutex<Vec<TurnRecord>>,
    turns_path: PathBuf,
}

impl TurnStore {
    pub fn new(data_dir: &Path) -> Self {
        let turns_path = data_dir.join("turns.json");
        Self {
            turns: Mutex::new(load_turns(&turns_path)),
            turns_path,
        }
    }

    // ─── 查询 ──────────────────────────────────────────────────────────────

    /// 获取某 Campaign 的活动 Turn（status 非 terminal）。
    ///
    /// 屏障检查用：存在活动 Turn → start_writing 拒绝。
    pub fn get_active_turn(&self, campaign_id: &Id) -> Option<TurnRecord> {
        let turns = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        turns
            .iter()
            .find(|t| &t.campaign_id == campaign_id && t.status.is_active())
            .cloned()
    }

    /// 按 turn_id 获取 TurnRecord。
    pub fn get_turn(&self, turn_id: &Id) -> Option<TurnRecord> {
        let turns = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        turns.iter().find(|t| &t.turn_id == turn_id).cloned()
    }

    /// 按 variant_id 查找包含该变体的 TurnRecord。
    pub fn get_turn_by_variant(&self, variant_id: &Id) -> Option<TurnRecord> {
        let turns = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        turns
            .iter()
            .find(|t| t.find_attempt_by_variant(variant_id).is_some())
            .cloned()
    }

    /// 列出所有需要恢复的 Turn（status == Committing）。
    ///
    /// 启动恢复用：这些 Turn 的 MutationBatch 可能已部分写入，需要幂等重放。
    pub fn list_recoverable_turns(&self) -> Vec<TurnRecord> {
        let turns = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        turns
            .iter()
            .filter(|t| t.status == TurnStatus::Committing)
            .cloned()
            .collect()
    }

    /// 列出所有活动 Turn（启动恢复时标记 Failed 用）。
    pub fn list_active_turns(&self) -> Vec<TurnRecord> {
        let turns = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        turns
            .iter()
            .filter(|t| t.status.is_active())
            .cloned()
            .collect()
    }

    // ─── 写入 ──────────────────────────────────────────────────────────────

    /// 创建新 TurnRecord（如果该 Campaign 已有活动 Turn 则拒绝）。
    pub fn create_turn(&self, record: TurnRecord) -> Result<(), String> {
        let mut turns = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        // 唯一约束：同一 Campaign 只能有一个活动 Turn
        if turns
            .iter()
            .any(|t| t.campaign_id == record.campaign_id && t.status.is_active())
        {
            return Err(format!(
                "Campaign {} 已有活动 Turn，不能创建新 Turn",
                record.campaign_id
            ));
        }
        turns.push(record);
        persist_turns(&self.turns_path, &turns)
    }

    /// 全量替换 TurnRecord（by turn_id）。
    ///
    /// 调用方负责保证状态转换合法性，本方法只做持久化。
    pub fn save_turn(&self, record: TurnRecord) -> Result<(), String> {
        let mut turns = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = turns.iter().position(|t| t.turn_id == record.turn_id) {
            turns[idx] = record;
        } else {
            turns.push(record);
        }
        persist_turns(&self.turns_path, &turns)
    }

    /// CAS（Compare-And-Swap）Turn 状态转换。
    ///
    /// 仅当当前状态 == expected 时才更新为 new，返回是否成功。
    /// 用于并发安全的状态转换（如 AwaitingAcceptance → Committing）。
    pub fn cas_turn_status(
        &self,
        turn_id: &Id,
        expected: TurnStatus,
        new: TurnStatus,
    ) -> Result<bool, String> {
        self.mutate_if(
            turn_id,
            |record| record.status == expected,
            |record| {
                record.status = new;
                record.touch();
            },
        )
    }

    /// 持锁读取-条件校验-修改-原子持久化。
    ///
    /// - `Ok(true)`：predicate 通过且已写入
    /// - `Ok(false)`：predicate 失败，未改动
    /// - `Err`：Turn 不存在或持久化失败
    ///
    /// accept / 后台 postprocess 必须走此路径，避免 get→改→save 的 TOCTOU 覆盖。
    pub fn mutate_if<P, M>(&self, turn_id: &Id, predicate: P, mutate: M) -> Result<bool, String>
    where
        P: FnOnce(&TurnRecord) -> bool,
        M: FnOnce(&mut TurnRecord),
    {
        let mut turns = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        let Some(idx) = turns.iter().position(|t| &t.turn_id == turn_id) else {
            return Err(format!("TurnRecord {} 不存在", turn_id));
        };
        if !predicate(&turns[idx]) {
            return Ok(false);
        }
        let before = turns[idx].clone();
        mutate(&mut turns[idx]);
        if let Err(error) = persist_turns(&self.turns_path, &turns) {
            // Keep the in-memory journal aligned with the durable copy. Callers may
            // retry/recover a Committing turn after a transient terminal-write error.
            turns[idx] = before;
            return Err(error);
        }
        Ok(true)
    }

    /// 持锁修改并持久化（无条件；Turn 不存在则 Err）。
    pub fn with_turn_mut<F>(&self, turn_id: &Id, mutate: F) -> Result<(), String>
    where
        F: FnOnce(&mut TurnRecord),
    {
        let applied = self.mutate_if(turn_id, |_| true, mutate)?;
        if !applied {
            return Err(format!("TurnRecord {} 未能更新", turn_id));
        }
        Ok(())
    }

    // ─── 测试辅助 ──────────────────────────────────────────────────────────

    /// 列全部 Turn（测试和调试用）
    pub fn list_all(&self) -> Vec<TurnRecord> {
        self.turns.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

// ─── 持久化辅助 ─────────────────────────────────────────────────────────────

fn load_turns(path: &Path) -> Vec<TurnRecord> {
    crate::storage::json_store::load_json_with_tmp_backup_or_default(
        path,
        |error| {
            tracing::warn!(
                "Failed to load {}; trying .tmp backup: {error}",
                path.display()
            );
        },
        |path, error| {
            tracing::error!(
                "Failed to load {} and .tmp recovery was unavailable; copied .corrupt backup: {error}",
                path.display()
            );
        },
    )
}

fn persist_turns(path: &Path, data: &[TurnRecord]) -> Result<(), String> {
    storyforge_infra_util::atomic_write_json(path, data).map_err(|e| {
        let msg = format!("持久化失败 {}: {e}", path.display());
        tracing::error!("{msg}");
        msg
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::turn::{AttemptStatus, TurnAttempt};

    fn temp_store() -> TurnStore {
        let dir = std::env::temp_dir().join(format!(
            "storyforge-turn-store-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TurnStore::new(&dir)
    }

    fn make_record(campaign_id: &str) -> TurnRecord {
        TurnRecord::new(
            Id::from_str(campaign_id),
            Id::from_str("conv-1"),
            Id::from_str("node-1"),
            0,
        )
    }

    #[test]
    fn create_turn_succeeds() {
        let store = temp_store();
        let record = make_record("camp-1");
        store.create_turn(record).unwrap();
        assert!(store.get_active_turn(&Id::from_str("camp-1")).is_some());
    }

    #[test]
    fn create_turn_rejects_duplicate_active() {
        let store = temp_store();
        store.create_turn(make_record("camp-1")).unwrap();
        let result = store.create_turn(make_record("camp-1"));
        assert!(result.is_err(), "should reject duplicate active turn");
    }

    #[test]
    fn get_active_turn_returns_none_for_terminal() {
        let store = temp_store();
        let mut record = make_record("camp-1");
        record.status = TurnStatus::Committed;
        store.save_turn(record).unwrap();
        assert!(store.get_active_turn(&Id::from_str("camp-1")).is_none());
    }

    #[test]
    fn mutate_if_skips_when_predicate_fails() {
        let store = temp_store();
        let record = make_record("camp-1");
        let turn_id = record.turn_id.clone();
        store.create_turn(record).unwrap();

        let applied = store
            .mutate_if(
                &turn_id,
                |r| r.status == TurnStatus::Committed,
                |r| r.status = TurnStatus::Failed,
            )
            .unwrap();
        assert!(!applied);
        assert_eq!(
            store.get_turn(&turn_id).unwrap().status,
            TurnStatus::Generating
        );
    }

    #[test]
    fn mutate_if_applies_under_lock_when_predicate_ok() {
        let store = temp_store();
        let record = make_record("camp-1");
        let turn_id = record.turn_id.clone();
        store.create_turn(record).unwrap();

        let applied = store
            .mutate_if(
                &turn_id,
                |r| r.status == TurnStatus::Generating,
                |r| {
                    r.status = TurnStatus::AwaitingAcceptance;
                    r.touch();
                },
            )
            .unwrap();
        assert!(applied);
        assert_eq!(
            store.get_turn(&turn_id).unwrap().status,
            TurnStatus::AwaitingAcceptance
        );
    }

    /// Phase A 契约：accept CAS 与 postprocess 写回争用时，
    /// 已进入 Committing 的 Turn 不能被 postprocess 条件写回覆盖。
    #[test]
    fn contract_postprocess_skips_when_committing() {
        let store = temp_store();
        let mut record = make_record("camp-race");
        let turn_id = record.turn_id.clone();
        let attempt_id = Id::from_str("att-race");
        record.status = TurnStatus::AwaitingAcceptance;
        record.attempts.push(TurnAttempt {
            attempt_id: attempt_id.clone(),
            variant_id: Id::from_str("var-race"),
            draft_hash: "h".into(),
            status: AttemptStatus::AwaitingAcceptance,
            pending_state_changes: None,
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        store.create_turn(record).unwrap();

        // accept 路径：AwaitingAcceptance → Committing
        let cas_ok = store
            .mutate_if(
                &turn_id,
                |r| {
                    r.status == TurnStatus::AwaitingAcceptance
                        && r.find_attempt(&attempt_id)
                            .is_some_and(|a| a.status == AttemptStatus::AwaitingAcceptance)
                },
                |r| {
                    r.status = TurnStatus::Committing;
                    if let Some(att) = r.find_attempt_mut(&attempt_id) {
                        att.status = AttemptStatus::Committing;
                    }
                    r.touch();
                },
            )
            .unwrap();
        assert!(cas_ok);

        // 迟到的 postprocess：predicate 与生产路径一致
        let pp_ok = store
            .mutate_if(
                &turn_id,
                |r| !r.status.is_terminal() && r.status != TurnStatus::Committing,
                |r| {
                    r.status = TurnStatus::AwaitingAcceptance;
                    if let Some(att) = r.find_attempt_mut(&attempt_id) {
                        att.status = AttemptStatus::AwaitingAcceptance;
                        att.pending_state_changes = None;
                    }
                },
            )
            .unwrap();
        assert!(!pp_ok, "postprocess must not overwrite Committing");
        let after = store.get_turn(&turn_id).unwrap();
        assert_eq!(after.status, TurnStatus::Committing);
        assert_eq!(
            after.find_attempt(&attempt_id).unwrap().status,
            AttemptStatus::Committing
        );
    }

    /// Phase A 契约：Discard 只标 Attempt，不把 pending_temporary_instances 写进 Campaign。
    /// （Campaign 落盘仅经 accept 的 UpsertInstance；本测断言 Attempt 侧状态机。）
    #[test]
    fn contract_discard_attempt_keeps_temps_on_attempt_only() {
        let store = temp_store();
        let mut record = make_record("camp-discard");
        let turn_id = record.turn_id.clone();
        let attempt_id = Id::from_str("att-disc");
        let mut temp = storyforge_domain::campaign::CharacterInstance::temporary(
            Id::from_str("camp-discard"),
            "GhostOnlyOnAttempt",
        );
        temp.id = Id::from_str("temp-ghost");
        record.status = TurnStatus::AwaitingAcceptance;
        record.attempts.push(TurnAttempt {
            attempt_id: attempt_id.clone(),
            variant_id: Id::from_str("var-disc"),
            draft_hash: "h".into(),
            status: AttemptStatus::AwaitingAcceptance,
            pending_state_changes: None,
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![temp.clone()],
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        store.create_turn(record).unwrap();

        // soft_delete_variant 语义：仅 Attempt → Discarded；Turn 仍 active
        store
            .with_turn_mut(&turn_id, |r| {
                if let Some(att) = r.find_attempt_mut(&attempt_id) {
                    att.status = AttemptStatus::Discarded;
                }
                r.touch();
            })
            .unwrap();

        let after = store.get_turn(&turn_id).unwrap();
        assert_eq!(after.status, TurnStatus::AwaitingAcceptance);
        let att = after.find_attempt(&attempt_id).unwrap();
        assert_eq!(att.status, AttemptStatus::Discarded);
        assert_eq!(att.pending_temporary_instances.len(), 1);
        assert_eq!(att.pending_temporary_instances[0].id, temp.id);
        // 活动 Turn 仍在（允许 regenerate），temps 未“提交”到别处
        assert!(
            store
                .get_active_turn(&Id::from_str("camp-discard"))
                .is_some()
        );
    }

    #[test]
    fn cas_turn_status_succeeds_on_match() {
        let store = temp_store();
        let record = make_record("camp-1");
        let turn_id = record.turn_id.clone();
        store.create_turn(record).unwrap();

        let ok = store
            .cas_turn_status(&turn_id, TurnStatus::Generating, TurnStatus::DraftReady)
            .unwrap();
        assert!(ok);
        assert_eq!(
            store.get_turn(&turn_id).unwrap().status,
            TurnStatus::DraftReady
        );
    }

    #[test]
    fn cas_turn_status_fails_on_mismatch() {
        let store = temp_store();
        let record = make_record("camp-1");
        let turn_id = record.turn_id.clone();
        store.create_turn(record).unwrap();

        let ok = store
            .cas_turn_status(&turn_id, TurnStatus::Committed, TurnStatus::Failed)
            .unwrap();
        assert!(!ok, "CAS should fail when expected status doesn't match");
        // 原状态不变
        assert_eq!(
            store.get_turn(&turn_id).unwrap().status,
            TurnStatus::Generating
        );
    }

    #[test]
    fn list_recoverable_turns_returns_committing_only() {
        let store = temp_store();

        let mut t1 = make_record("camp-1");
        t1.status = TurnStatus::Committing;
        store.save_turn(t1).unwrap();

        let mut t2 = make_record("camp-2");
        t2.status = TurnStatus::AwaitingAcceptance;
        store.save_turn(t2).unwrap();

        let recoverable = store.list_recoverable_turns();
        assert_eq!(recoverable.len(), 1);
        assert_eq!(recoverable[0].status, TurnStatus::Committing);
    }

    #[test]
    fn get_turn_by_variant_finds_correct_turn() {
        let store = temp_store();
        let mut record = make_record("camp-1");
        let variant_id = Id::from_str("var-99");
        record.attempts.push(TurnAttempt {
            attempt_id: Id::from_str("att-1"),
            variant_id: variant_id.clone(),
            draft_hash: "abc".into(),
            status: AttemptStatus::AwaitingAcceptance,
            pending_state_changes: None,
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        store.create_turn(record).unwrap();

        assert!(store.get_turn_by_variant(&variant_id).is_some());
        assert!(
            store
                .get_turn_by_variant(&Id::from_str("nonexistent"))
                .is_none()
        );
    }

    #[test]
    fn persist_and_reload_preserves_data() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge-turn-store-reload-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        {
            let store = TurnStore::new(&dir);
            store.create_turn(make_record("camp-1")).unwrap();
        }

        // Reopen from same dir
        let store2 = TurnStore::new(&dir);
        assert!(store2.get_active_turn(&Id::from_str("camp-1")).is_some());
    }
}
