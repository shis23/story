//! Campaign 版本化提交协调器（Phase A）
//!
//! `CampaignMutationCoordinator` 是 TurnCommit 和 MetaCommit 共用的版本化提交入口。
//!
//! 核心职责（收敛决策步骤 5/7/13/16/18）：
//! 1. revision CAS：校验 `Campaign.revision == expected_revision`。
//! 2. 逐条执行 Mutation（upsert 三态 / 绝对值写入）。
//! 3. revision bump 一次（target_revision = expected + 1）。
//! 4. per-campaign mutation lock：防止 TurnCommit 与 MetaCommit 并发。
//!
//! 物理原子性：阶段 A 只提供逻辑原子性（journal + 幂等 + 启动恢复），
//! 真正的跨集合事务留给阶段 D（UnitOfWork/SQLite）。

use std::sync::{Mutex, OnceLock};

use storyforge_domain::Id;
#[cfg(test)]
use storyforge_domain::turn::MutationBatchStatus;
use storyforge_domain::turn::{Mutation, MutationBatch};

use crate::campaign_store::{CampaignStore, UpsertResult};

// ─── 提交错误 ───────────────────────────────────────────────────────────────

/// 版本化提交的错误类型。
#[derive(Debug, Clone)]
pub enum CommitError {
    /// Campaign 不存在
    CampaignNotFound(Id),
    /// revision CAS 失败（expected != current）
    RevisionConflict { expected: u64, actual: u64 },
    /// Mutation 的 upsert 发现 payload 冲突
    MutationConflict(String),
    /// 持久化失败
    Storage(String),
    /// per-campaign lock 获取失败（中毒锁）
    LockPoisoned,
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CampaignNotFound(id) => write!(f, "Campaign 不存在: {id}"),
            Self::RevisionConflict { expected, actual } => {
                write!(f, "revision 冲突: expected {expected}, actual {actual}")
            }
            Self::MutationConflict(msg) => write!(f, "mutation 冲突: {msg}"),
            Self::Storage(msg) => write!(f, "存储失败: {msg}"),
            Self::LockPoisoned => write!(f, "campaign mutation lock 中毒"),
        }
    }
}

impl std::error::Error for CommitError {}

// ─── per-campaign mutation lock ─────────────────────────────────────────────

/// 全局提交锁（Phase A 简化版）。
///
/// 阶段 A 用单个全局 Mutex 序列化所有 Campaign 的版本化提交。
/// 不同 Campaign 之间会互相等待，但提交操作本身很快（JSON 写盘 + revision bump）。
/// 阶段 D 可以升级为 per-campaign 细粒度锁。
static GLOBAL_COMMIT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 在全局提交锁保护下执行闭包。
///
/// 保证 TurnCommit 和 MetaCommit 不会并发修改同一 Campaign。
pub fn with_campaign_lock<R>(f: impl FnOnce() -> Result<R, CommitError>) -> Result<R, CommitError> {
    let lock = GLOBAL_COMMIT_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().map_err(|_| CommitError::LockPoisoned)?;
    f()
}

// ─── CampaignMutationCoordinator ────────────────────────────────────────────

pub struct CampaignMutationCoordinator;

impl CampaignMutationCoordinator {
    /// 应用一个 MutationBatch 到 CampaignStore（TurnCommit / MetaCommit 共用入口）。
    ///
    /// 流程：
    /// 1. CAS 校验 revision（区分首次提交与幂等重放）。
    /// 2. 逐条执行 Mutation（upsert 三态 / 绝对值写入）。
    /// 3. revision bump 一次（target_revision = expected + 1）。
    ///
    /// 幂等性：如果 `Campaign.revision == batch.target_revision`，
    /// 说明 batch 已被重放过（同一 commit），只校验/补齐 mutation，不 bump revision。
    /// 如果 revision 既不等于 expected 也不等于 target，说明被外部写入推进 → Conflict。
    pub fn apply_mutation_batch(
        store: &CampaignStore,
        campaign_id: &Id,
        batch: &MutationBatch,
    ) -> Result<Id, CommitError> {
        let campaign = store
            .get_campaign(campaign_id)
            .ok_or_else(|| CommitError::CampaignNotFound(campaign_id.clone()))?;

        let is_replay = campaign.revision == batch.target_revision;
        let is_first_apply = campaign.revision == batch.expected_revision;

        if !is_replay && !is_first_apply {
            // revision 既不是 expected 也不是 target → 被外部推进
            return Err(CommitError::RevisionConflict {
                expected: batch.expected_revision,
                actual: campaign.revision,
            });
        }

        // 逐条执行 mutation
        for mutation in &batch.mutations {
            Self::apply_single_mutation(store, campaign_id, mutation)?;
        }

        // revision bump（仅在首次提交时，幂等重放不 bump）
        if is_first_apply {
            let mut updated = store
                .get_campaign(campaign_id)
                .ok_or_else(|| CommitError::CampaignNotFound(campaign_id.clone()))?;
            updated.revision = batch.target_revision;
            store
                .update_campaign(updated)
                .map_err(CommitError::Storage)?;
        }

        Ok(batch.commit_id.clone())
    }

    /// 执行单条 Mutation（不 bump revision，由调用方统一 bump）。
    fn apply_single_mutation(
        store: &CampaignStore,
        campaign_id: &Id,
        mutation: &Mutation,
    ) -> Result<(), CommitError> {
        match mutation {
            Mutation::SetVariable {
                instance_id,
                key,
                value,
                turn,
            } => {
                if let Some(inst_id) = instance_id {
                    // 角色级变量
                    let mut inst = store.get_instance(campaign_id, inst_id).ok_or_else(|| {
                        CommitError::Storage(format!(
                            "instance {inst_id} 不存在于 campaign {campaign_id}"
                        ))
                    })?;
                    inst.set_variable(key, value.clone(), *turn);
                    store.update_instance(inst).map_err(CommitError::Storage)?;
                } else {
                    // 全局 Campaign 变量
                    let mut camp = store
                        .get_campaign(campaign_id)
                        .ok_or_else(|| CommitError::CampaignNotFound(campaign_id.clone()))?;
                    camp.set_variable(key, value.clone(), *turn);
                    store.update_campaign(camp).map_err(CommitError::Storage)?;
                }
                Ok(())
            }

            Mutation::UpsertKnowledge(km) => {
                let entry = km.to_entry();
                match store
                    .upsert_knowledge(entry)
                    .map_err(CommitError::Storage)?
                {
                    UpsertResult::Inserted | UpsertResult::AlreadyPresent => Ok(()),
                    UpsertResult::Conflict(msg) => Err(CommitError::MutationConflict(msg)),
                }
            }

            Mutation::SetTaskStatus { task_id, status } => {
                let mut task = store
                    .get_task(task_id)
                    .ok_or_else(|| CommitError::Storage(format!("task {task_id} 不存在")))?;
                task.status = status.clone();
                store.update_task(task).map_err(CommitError::Storage)?;
                Ok(())
            }

            Mutation::UpsertNewTask(task) => {
                match store
                    .upsert_task((**task).clone())
                    .map_err(CommitError::Storage)?
                {
                    UpsertResult::Inserted | UpsertResult::AlreadyPresent => Ok(()),
                    UpsertResult::Conflict(msg) => Err(CommitError::MutationConflict(msg)),
                }
            }

            Mutation::UpsertSummary(summary) => {
                match store
                    .upsert_summary((**summary).clone())
                    .map_err(CommitError::Storage)?
                {
                    UpsertResult::Inserted => {
                        // 记忆规格：Accept 新 Chronicle A → chronicle_revision++
                        if let Some(mut camp) = store.get_campaign(campaign_id) {
                            camp.bump_chronicle_revision();
                            let _ = store.update_campaign(camp);
                        }
                        // M4 最小：阈值达到则规划压缩组（后台 LLM 发布尚未接线）
                        let all = store.list_summaries(campaign_id);
                        let uncovered: Vec<_> = all
                            .iter()
                            .filter(|s| s.covered_by.is_none())
                            .collect();
                        let ids: Vec<_> = uncovered.iter().map(|s| s.id.clone()).collect();
                        let spans: Vec<(u32, u32)> =
                            uncovered.iter().map(|s| (s.turn, s.turn)).collect();
                        match storyforge_domain::chronicle::plan_compress_batch_for_uncovered(
                            &ids,
                            &spans,
                            storyforge_domain::chronicle::DEFAULT_COMPRESS_ACTIVE_A_THRESHOLD,
                            storyforge_domain::chronicle::DEFAULT_COMPRESS_GROUP_SIZE,
                        ) {
                            Ok(Some(groups)) => {
                                tracing::info!(
                                    target: "chronicle_compressor",
                                    campaign_id = %campaign_id,
                                    uncovered = uncovered.len(),
                                    groups = groups.len(),
                                    "A→B compress batch planned (enqueue stub; no LLM publish yet)"
                                );
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::warn!(
                                    target: "chronicle_compressor",
                                    "compress plan failed: {e:?}"
                                );
                            }
                        }
                        Ok(())
                    }
                    UpsertResult::AlreadyPresent => Ok(()),
                    UpsertResult::Conflict(msg) => Err(CommitError::MutationConflict(msg)),
                }
            }

            Mutation::FinalizeVariant { variant_id: _ } => {
                // Draft → Final 由 ConversationStore 处理，不在 CampaignStore 范围
                // TurnCoordinator.commit 会单独调 conv_store.accept_variant
                Ok(())
            }

            Mutation::UpsertInstance(instance) => {
                // accept 时落盘临时角色：
                // - 同 id + 关键字段一致 → AlreadyPresent / no-op
                // - 同 id + payload 不一致 → Conflict
                // - 不同 id 同名 → Conflict
                if instance.campaign_id != *campaign_id {
                    return Err(CommitError::Storage(format!(
                        "UpsertInstance campaign 不匹配: instance={}, expected={}",
                        instance.campaign_id, campaign_id
                    )));
                }
                let existing = store.list_instances(campaign_id);
                if let Some(prev) = existing.iter().find(|i| i.id == instance.id) {
                    let same_payload = prev.name == instance.name
                        && prev.definition_id == instance.definition_id
                        && prev.is_temporary == instance.is_temporary
                        && prev.persona_override == instance.persona_override
                        && prev.behavior_override == instance.behavior_override
                        && prev.variables == instance.variables;
                    if same_payload {
                        return Ok(());
                    }
                    return Err(CommitError::MutationConflict(format!(
                        "UpsertInstance id={} payload 与已有实例冲突",
                        instance.id
                    )));
                }
                if existing.iter().any(|i| i.name == instance.name) {
                    return Err(CommitError::MutationConflict(format!(
                        "临时 instance '{}' 与已有同名角色冲突",
                        instance.name
                    )));
                }
                store
                    .add_instance((**instance).clone())
                    .map_err(CommitError::Storage)?;
                Ok(())
            }
        }
    }

    /// 获取某 Campaign 的当前 revision（用于构建 MutationBatch 的 expected_revision）。
    pub fn current_revision(store: &CampaignStore, campaign_id: &Id) -> Option<u64> {
        store.get_campaign(campaign_id).map(|c| c.revision)
    }

    /// 检查 Campaign 是否有活动 Turn（屏障检查用）。
    /// 实际检查由 TurnStore.get_active_turn 完成，这里只是转发。
    pub fn has_active_turn(turn_store: &crate::turn_store::TurnStore, campaign_id: &Id) -> bool {
        turn_store.get_active_turn(campaign_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::campaign_store::CampaignStore;
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::character_knowledge::KnowledgeSource;
    use storyforge_domain::turn::{KnowledgeMutation, Mutation};

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "storyforge-coordinator-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup_campaign(store: &CampaignStore) -> Id {
        let campaign = Campaign::new(Id::from_str("card-1"), "test".to_string());
        let campaign_id = campaign.id.clone();
        store.save_campaign(campaign).unwrap();
        campaign_id
    }

    #[test]
    fn apply_batch_bumps_revision_once() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let campaign_id = setup_campaign(&store);

        let batch = MutationBatch {
            commit_id: Id::new(),
            expected_revision: 0,
            target_revision: 1,
            status: MutationBatchStatus::Prepared,
            mutations: vec![],
        };

        CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch).unwrap();

        let updated = store.get_campaign(&campaign_id).unwrap();
        assert_eq!(updated.revision, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_batch_rejects_revision_conflict() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let campaign_id = setup_campaign(&store);

        // 手动 bump revision 到 2（模拟被另一个 TurnCommit 推进）
        let mut camp = store.get_campaign(&campaign_id).unwrap();
        camp.revision = 2;
        store.update_campaign(camp).unwrap();

        // batch 期望 0,target 1,但实际是 2 → conflict
        let batch = MutationBatch {
            commit_id: Id::new(),
            expected_revision: 0,
            target_revision: 1,
            status: MutationBatchStatus::Prepared,
            mutations: vec![],
        };

        let result =
            CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch);
        assert!(matches!(result, Err(CommitError::RevisionConflict { .. })));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_batch_idempotent_replay() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let campaign_id = setup_campaign(&store);

        let batch = MutationBatch {
            commit_id: Id::from_str("commit-1"),
            expected_revision: 0,
            target_revision: 1,
            status: MutationBatchStatus::Prepared,
            mutations: vec![],
        };

        // 第一次应用
        CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch).unwrap();
        // 第二次（重放）：revision 已经是 1 == target_revision → 幂等 no-op
        CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch).unwrap();

        let updated = store.get_campaign(&campaign_id).unwrap();
        assert_eq!(
            updated.revision, 1,
            "revision should stay at 1 after replay"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_batch_sets_global_variable() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let campaign_id = setup_campaign(&store);

        let batch = MutationBatch {
            commit_id: Id::new(),
            expected_revision: 0,
            target_revision: 1,
            status: MutationBatchStatus::Prepared,
            mutations: vec![Mutation::SetVariable {
                instance_id: None,
                key: "story_clock".into(),
                value: serde_json::json!("Day 5"),
                turn: 1,
            }],
        };

        CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch).unwrap();

        let camp = store.get_campaign(&campaign_id).unwrap();
        assert_eq!(
            camp.get_variable("story_clock"),
            Some(&serde_json::json!("Day 5"))
        );
        assert_eq!(camp.revision, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_batch_upserts_knowledge_idempotently() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let campaign_id = setup_campaign(&store);

        let km = KnowledgeMutation {
            entry_id: Id::from_str("k-1"),
            campaign_id: campaign_id.clone(),
            character_id: Id::from_str("char-1"),
            knowledge_text: "看到了刀".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            turn_number: 1,
            event_id: None,
            pinned: false,
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        };

        let batch = MutationBatch {
            commit_id: Id::from_str("commit-1"),
            expected_revision: 0,
            target_revision: 1,
            status: MutationBatchStatus::Prepared,
            mutations: vec![Mutation::UpsertKnowledge(Box::new(km.clone()))],
        };

        // 第一次
        CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch).unwrap();
        assert_eq!(store.list_knowledge(&campaign_id).len(), 1);

        // 重放（revision 已是 1 == target）
        CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch).unwrap();
        assert_eq!(
            store.list_knowledge(&campaign_id).len(),
            1,
            "knowledge should not be duplicated on replay"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_batch_unknown_campaign_returns_error() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);

        let batch = MutationBatch {
            commit_id: Id::new(),
            expected_revision: 0,
            target_revision: 1,
            status: MutationBatchStatus::Prepared,
            mutations: vec![],
        };

        let result = CampaignMutationCoordinator::apply_mutation_batch(
            &store,
            &Id::from_str("nonexistent"),
            &batch,
        );
        assert!(matches!(result, Err(CommitError::CampaignNotFound(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_batch_upsert_instance_idempotent() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let campaign_id = Id::from_str("camp-temp");
        let mut camp =
            storyforge_domain::campaign::Campaign::new(Id::from_str("card-1"), "temp camp");
        camp.id = campaign_id.clone();
        store.save_campaign(camp).unwrap();

        let mut inst = storyforge_domain::campaign::CharacterInstance::temporary(
            campaign_id.clone(),
            "GhostTemp",
        );
        inst.id = Id::from_str("temp-1");
        let batch = MutationBatch {
            commit_id: Id::from_str("commit-temp"),
            expected_revision: 0,
            target_revision: 1,
            status: MutationBatchStatus::Prepared,
            mutations: vec![Mutation::UpsertInstance(Box::new(inst.clone()))],
        };
        CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch).unwrap();
        assert_eq!(store.list_instances(&campaign_id).len(), 1);
        // 同 id 同 payload 重放不重复
        CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch).unwrap();
        assert_eq!(store.list_instances(&campaign_id).len(), 1);
        // 同 id 不同 persona → Conflict
        let mut conflict = inst.clone();
        conflict.persona_override = Some("changed".into());
        let batch2 = MutationBatch {
            commit_id: Id::from_str("commit-temp-2"),
            expected_revision: 1,
            target_revision: 2,
            status: MutationBatchStatus::Prepared,
            mutations: vec![Mutation::UpsertInstance(Box::new(conflict))],
        };
        let err = CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch2)
            .expect_err("payload conflict");
        assert!(matches!(err, CommitError::MutationConflict(_)));
        // 同 id 不同 variables → Conflict
        let mut conflict_vars = inst.clone();
        conflict_vars.variables = vec![storyforge_domain::variables::VariableValue::new(
            "hp",
            serde_json::json!(1),
            0,
        )];
        let batch3 = MutationBatch {
            commit_id: Id::from_str("commit-temp-3"),
            expected_revision: 1,
            target_revision: 2,
            status: MutationBatchStatus::Prepared,
            mutations: vec![Mutation::UpsertInstance(Box::new(conflict_vars))],
        };
        let err = CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch3)
            .expect_err("variables conflict");
        assert!(matches!(err, CommitError::MutationConflict(_)));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Phase A 契约：Discard 不经 Coordinator → Campaign 无临时角色。
    /// Accept 才 UpsertInstance 落盘。
    #[test]
    fn contract_temps_only_persisted_via_accept_upsert() {
        let dir = temp_dir();
        let store = CampaignStore::new(&dir);
        let campaign_id = Id::from_str("camp-no-temp-on-discard");
        let mut camp =
            storyforge_domain::campaign::Campaign::new(Id::from_str("card-1"), "no temp");
        camp.id = campaign_id.clone();
        store.save_campaign(camp).unwrap();

        // Discard 路径：不调用 apply_mutation_batch（只标 Attempt Discarded）
        assert!(store.list_instances(&campaign_id).is_empty());

        // Accept 路径：batch 含 UpsertInstance 才落盘
        let mut temp = storyforge_domain::campaign::CharacterInstance::temporary(
            campaign_id.clone(),
            "AcceptOnlyGhost",
        );
        temp.id = Id::from_str("temp-accept-only");
        let batch = MutationBatch {
            commit_id: Id::from_str("commit-accept-temp"),
            expected_revision: 0,
            target_revision: 1,
            status: MutationBatchStatus::Prepared,
            mutations: vec![Mutation::UpsertInstance(Box::new(temp.clone()))],
        };
        CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign_id, &batch).unwrap();
        let instances = store.list_instances(&campaign_id);
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].id, temp.id);
        assert!(instances[0].is_temporary);
        std::fs::remove_dir_all(&dir).ok();
    }
}
