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
        turns.iter().filter(|t| t.status.is_active()).cloned().collect()
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
        let mut turns = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(idx) = turns.iter().position(|t| &t.turn_id == turn_id) {
            if turns[idx].status == expected {
                turns[idx].status = new;
                turns[idx].touch();
                return persist_turns(&self.turns_path, &turns).map(|()| true);
            }
            return Ok(false); // 状态不匹配，CAS 失败
        }
        Err(format!("TurnRecord {} 不存在", turn_id))
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
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        store.create_turn(record).unwrap();

        assert!(store.get_turn_by_variant(&variant_id).is_some());
        assert!(store
            .get_turn_by_variant(&Id::from_str("nonexistent"))
            .is_none());
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
