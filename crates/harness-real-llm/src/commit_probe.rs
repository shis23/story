//! 生产忠实 CommitTurn / Accept 探针（不经 Tauri command 私有路径）。
//!
//! 用公开的 `TurnStore` + `CampaignMutationCoordinator` + `ConversationStore`
//! 复刻生产 `commit_turn_attempt` 的关键断言与副作用：
//! - draft_hash 校验
//! - QualityGate Error 拦截 / force → Degraded
//! - AwaitingAcceptance 才可 accept
//! - MutationBatch 应用 + revision bump
//! - Chronicle A（UpsertSummary）经 Accept 落盘
//!
//! 这不是 `m5_s6` 的手工 `add_summary` 旁路。

use std::path::{Path, PathBuf};

use storyforge_app_conversation::ConversationStore;
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::chronicle::{ChronicleCode, ChronicleLevel, truncate_headline};
use storyforge_domain::turn::{
    AttemptStatus, Mutation, MutationBatch, MutationBatchStatus, QualityAcceptDecision,
    QualityReport, TurnAttempt, TurnRecord, TurnStatus, quality_accept_decision,
};
use storyforge_tauri_app::campaign_store::CampaignStore;
use storyforge_tauri_app::turn_coordinator::{CampaignMutationCoordinator, with_campaign_lock};
use storyforge_tauri_app::turn_store::TurnStore;

use crate::evidence::{AssertionResult, draft_hash_hex, short_hash16};

/// 一次生产忠实 Accept 的输入。
#[derive(Debug, Clone)]
pub struct ProductionAcceptInput {
    pub campaign_id: Id,
    pub conversation_id: Id,
    pub variant_id: Id,
    pub draft_text: String,
    /// postprocess 产出的摘要正文；None = 空 diff（仍会 FinalizeVariant + revision bump）
    pub summary_text: Option<String>,
    pub turn_number: u32,
    pub quality_report: Option<QualityReport>,
    pub force_accept: bool,
}

/// Accept 结果快照（供断言与 JSONL）。
#[derive(Debug, Clone)]
pub struct ProductionAcceptResult {
    pub ok: bool,
    pub error: Option<String>,
    pub force_accept: bool,
    pub turn_status: Option<TurnStatus>,
    pub attempt_status: Option<AttemptStatus>,
    pub campaign_revision_before: u64,
    pub campaign_revision_after: u64,
    pub chronicle_revision_before: u64,
    pub chronicle_revision_after: u64,
    pub summary_code: Option<String>,
    pub draft_hash: String,
    pub assertions: Vec<AssertionResult>,
}

/// 隔离的 CommitTurn 环境（独立 tempdir，不触全局 OnceLock 生产 store）。
pub struct CommitProbeEnv {
    pub data_dir: PathBuf,
    pub campaign_store: CampaignStore,
    pub turn_store: TurnStore,
    pub conv_store: ConversationStore,
}

impl CommitProbeEnv {
    pub fn new() -> Self {
        let data_dir =
            std::env::temp_dir().join(format!("sf_eval_commit_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).expect("create commit probe tempdir");
        let campaign_store = CampaignStore::new(&data_dir);
        let turn_store = TurnStore::new(&data_dir);
        let conv_store = ConversationStore::new(data_dir.join("conversations"));
        Self {
            data_dir,
            campaign_store,
            turn_store,
            conv_store,
        }
    }

    pub fn path(&self) -> &Path {
        &self.data_dir
    }

    pub fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }

    /// 创建 Campaign + 绑定 conversation，返回 (campaign_id, conversation_id)。
    pub fn bootstrap_campaign(&self, name: &str) -> (Id, Id) {
        let card_id = Id::new();
        let mut campaign = Campaign::new(card_id, name);
        if campaign.lineage_id.is_none() {
            campaign.lineage_id = Some(Id::new());
        }
        let campaign_id = campaign.id.clone();
        self.campaign_store
            .save_campaign(campaign)
            .expect("save campaign");
        let conversation = self.conv_store.create(None, None);
        let conversation_id = conversation.id.clone();
        // bind conversation onto campaign (mirrors production campaign.conversation_id)
        if let Some(mut camp) = self.campaign_store.get_campaign(&campaign_id) {
            camp.conversation_id = Some(conversation_id.clone());
            self.campaign_store
                .update_campaign(camp)
                .expect("bind conversation");
        }
        (campaign_id, conversation_id)
    }

    /// 写入 AI Draft 节点，返回 node/variant id。
    pub fn append_ai_draft(&self, conversation_id: &Id, text: &str) -> Id {
        self.conv_store
            .append_ai_draft(conversation_id, text.to_string(), None)
            .expect("append_ai_draft")
    }

    /// 构造 AwaitingAcceptance 的 TurnRecord + Attempt（含可选 UpsertSummary batch）。
    pub fn prepare_awaiting_accept(&self, input: &ProductionAcceptInput) -> TurnRecord {
        let camp = self
            .campaign_store
            .get_campaign(&input.campaign_id)
            .expect("campaign exists");
        let base_revision = camp.revision;
        let draft_hash = draft_hash_hex(&input.draft_text);

        let mut batch = MutationBatch {
            commit_id: Id::new(),
            expected_revision: base_revision,
            target_revision: base_revision + 1,
            status: MutationBatchStatus::Prepared,
            mutations: vec![],
        };

        let mut summary_code = None;
        if let Some(summary) = &input.summary_text {
            let existing = self.campaign_store.list_summaries(&input.campaign_id);
            let seq = next_chronicle_a_seq(&existing);
            let code = ChronicleCode::new(ChronicleLevel::A, seq);
            summary_code = Some(code.as_str().to_string());
            let lineage = camp.lineage_id.clone().unwrap_or_default();
            let headline = truncate_headline(summary, 40);
            batch.mutations.push(Mutation::UpsertSummary(Box::new(
                RoundSummary::new(
                    input.campaign_id.clone(),
                    input.conversation_id.clone(),
                    input.turn_number,
                    summary.clone(),
                )
                .with_code(code.as_str())
                .with_headline(headline)
                .with_lineage(lineage),
            )));
        }

        // production always injects FinalizeVariant if missing
        batch.mutations.push(Mutation::FinalizeVariant {
            variant_id: input.variant_id.clone(),
        });

        let attempt = TurnAttempt {
            attempt_id: Id::new(),
            variant_id: input.variant_id.clone(),
            draft_hash,
            status: AttemptStatus::AwaitingAcceptance,
            pending_state_changes: Some(batch),
            derivation: None,
            quality_report: input.quality_report.clone(),
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        let mut record = TurnRecord::new(
            input.campaign_id.clone(),
            input.conversation_id.clone(),
            Id::from_str("eval-input-node"),
            base_revision,
        );
        record.status = TurnStatus::AwaitingAcceptance;
        record.attempts.push(attempt);
        // keep summary_code accessible via quality of prepared batch for tests
        let _ = summary_code;
        self.turn_store
            .create_turn(record.clone())
            .expect("create turn");
        record
    }

    /// 生产忠实 Accept：镜像 `commit_turn_attempt` 的校验与副作用。
    pub fn accept_production(&self, input: &ProductionAcceptInput) -> ProductionAcceptResult {
        let mut assertions = Vec::new();
        let draft_hash = draft_hash_hex(&input.draft_text);

        let camp_before = self
            .campaign_store
            .get_campaign(&input.campaign_id)
            .expect("campaign");
        let campaign_revision_before = camp_before.revision;
        let chronicle_revision_before = camp_before.chronicle_revision;

        let turn = match self.turn_store.get_turn_by_variant(&input.variant_id) {
            Some(t) => t,
            None => {
                return ProductionAcceptResult {
                    ok: false,
                    error: Some("no TurnRecord for variant".into()),
                    force_accept: input.force_accept,
                    turn_status: None,
                    attempt_status: None,
                    campaign_revision_before,
                    campaign_revision_after: campaign_revision_before,
                    chronicle_revision_before,
                    chronicle_revision_after: chronicle_revision_before,
                    summary_code: None,
                    draft_hash,
                    assertions: vec![AssertionResult {
                        name: "turn_record_present".into(),
                        passed: false,
                        detail: Some("missing".into()),
                    }],
                };
            }
        };

        assertions.push(AssertionResult {
            name: "turn_record_present".into(),
            passed: true,
            detail: None,
        });

        let attempt = match turn.find_attempt_by_variant(&input.variant_id) {
            Some(a) => a.clone(),
            None => {
                return fail_result(
                    input,
                    campaign_revision_before,
                    chronicle_revision_before,
                    draft_hash,
                    assertions,
                    "no active TurnAttempt for variant",
                );
            }
        };

        if attempt.status != AttemptStatus::AwaitingAcceptance {
            return fail_result(
                input,
                campaign_revision_before,
                chronicle_revision_before,
                draft_hash,
                assertions,
                &format!("attempt status {:?} not AwaitingAcceptance", attempt.status),
            );
        }
        assertions.push(AssertionResult {
            name: "attempt_awaiting_acceptance".into(),
            passed: true,
            detail: None,
        });

        // QualityGate policy
        let decision = quality_accept_decision(attempt.quality_report.as_ref(), input.force_accept);
        let commit_as_degraded = match &decision {
            QualityAcceptDecision::AllowCommit => false,
            QualityAcceptDecision::ForceDegraded { error_count } => {
                assertions.push(AssertionResult {
                    name: "quality_force_degraded".into(),
                    passed: true,
                    detail: Some(format!("error_count={error_count}")),
                });
                true
            }
            QualityAcceptDecision::Block { error_count } => {
                assertions.push(AssertionResult {
                    name: "quality_block".into(),
                    passed: true,
                    detail: Some(format!("error_count={error_count}")),
                });
                return ProductionAcceptResult {
                    ok: false,
                    error: Some(format!(
                        "quality gate blocked accept: {error_count} error(s)"
                    )),
                    force_accept: input.force_accept,
                    turn_status: Some(turn.status.clone()),
                    attempt_status: Some(attempt.status.clone()),
                    campaign_revision_before,
                    campaign_revision_after: campaign_revision_before,
                    chronicle_revision_before,
                    chronicle_revision_after: chronicle_revision_before,
                    summary_code: None,
                    draft_hash,
                    assertions,
                };
            }
        };

        // revision CAS
        if turn.base_campaign_revision != campaign_revision_before {
            return fail_result(
                input,
                campaign_revision_before,
                chronicle_revision_before,
                draft_hash,
                assertions,
                &format!(
                    "revision conflict turn.base={} current={}",
                    turn.base_campaign_revision, campaign_revision_before
                ),
            );
        }
        assertions.push(AssertionResult {
            name: "revision_cas".into(),
            passed: true,
            detail: Some(format!("rev={campaign_revision_before}")),
        });

        // draft_hash re-check against live variant content
        let live_text =
            read_variant_content(&self.conv_store, &input.conversation_id, &input.variant_id);
        let live_hash = draft_hash_hex(&live_text);
        if live_hash != attempt.draft_hash {
            return fail_result(
                input,
                campaign_revision_before,
                chronicle_revision_before,
                draft_hash,
                assertions,
                "draft_hash mismatch vs live variant",
            );
        }
        assertions.push(AssertionResult {
            name: "draft_hash_match".into(),
            passed: true,
            detail: Some(short_hash16(&live_hash)),
        });

        // ensure FinalizeVariant present
        let mut batch = attempt
            .pending_state_changes
            .clone()
            .unwrap_or_else(|| MutationBatch::new(Id::new(), campaign_revision_before));
        let has_finalize = batch
            .mutations
            .iter()
            .any(|m| matches!(m, Mutation::FinalizeVariant { .. }));
        if !has_finalize {
            batch.mutations.push(Mutation::FinalizeVariant {
                variant_id: input.variant_id.clone(),
            });
        }

        let summary_code = batch.mutations.iter().find_map(|m| match m {
            Mutation::UpsertSummary(s) => s.code.clone(),
            _ => None,
        });

        // CAS turn → Committing
        let turn_id = turn.turn_id.clone();
        let attempt_id = attempt.attempt_id.clone();
        let batch_for_store = batch.clone();
        let cas_ok = self
            .turn_store
            .mutate_if(
                &turn_id,
                |record| {
                    record.status == TurnStatus::AwaitingAcceptance
                        && record
                            .find_attempt(&attempt_id)
                            .is_some_and(|a| a.status == AttemptStatus::AwaitingAcceptance)
                },
                |record| {
                    record.status = TurnStatus::Committing;
                    if let Some(att) = record.find_attempt_mut(&attempt_id) {
                        att.status = AttemptStatus::Committing;
                        att.pending_state_changes = Some(batch_for_store);
                    }
                    record.touch();
                },
            )
            .expect("mutate_if");
        if !cas_ok {
            return fail_result(
                input,
                campaign_revision_before,
                chronicle_revision_before,
                draft_hash,
                assertions,
                "turn CAS into Committing failed",
            );
        }

        // side effects under campaign lock
        let apply_result = with_campaign_lock(|| {
            self.conv_store
                .accept_variant(&input.conversation_id, &input.variant_id)
                .map_err(|e| {
                    storyforge_tauri_app::turn_coordinator::CommitError::Storage(format!(
                        "Draft→Final failed: {e}"
                    ))
                })?;
            CampaignMutationCoordinator::apply_mutation_batch(
                &self.campaign_store,
                &input.campaign_id,
                &batch,
            )?;
            Ok(())
        });

        if let Err(e) = apply_result {
            let _ = self.turn_store.with_turn_mut(&turn_id, |record| {
                record.status = TurnStatus::Failed;
                record.failure_reason = Some(e.to_string());
                record.touch();
            });
            return fail_result(
                input,
                campaign_revision_before,
                chronicle_revision_before,
                draft_hash,
                assertions,
                &format!("commit side effects failed: {e}"),
            );
        }

        let final_status = if commit_as_degraded {
            TurnStatus::Degraded
        } else {
            TurnStatus::Committed
        };
        let _ = self.turn_store.with_turn_mut(&turn_id, |record| {
            record.status = final_status.clone();
            record.accepted_attempt_id = Some(attempt_id.clone());
            for att in &mut record.attempts {
                if att.attempt_id != attempt_id && att.status.is_active() {
                    att.status = AttemptStatus::Superseded;
                } else if att.attempt_id == attempt_id {
                    att.status = AttemptStatus::Committed;
                }
            }
            record.touch();
        });

        let camp_after = self
            .campaign_store
            .get_campaign(&input.campaign_id)
            .expect("campaign after");
        let campaign_revision_after = camp_after.revision;
        let chronicle_revision_after = camp_after.chronicle_revision;

        assertions.push(AssertionResult {
            name: "campaign_revision_bumped".into(),
            passed: campaign_revision_after == campaign_revision_before + 1,
            detail: Some(format!(
                "{campaign_revision_before}->{campaign_revision_after}"
            )),
        });
        if summary_code.is_some() {
            assertions.push(AssertionResult {
                name: "chronicle_a_persisted".into(),
                passed: chronicle_revision_after > chronicle_revision_before
                    || !self
                        .campaign_store
                        .list_summaries(&input.campaign_id)
                        .is_empty(),
                detail: summary_code.clone(),
            });
        }

        // variant must be Final
        let final_ok = self
            .conv_store
            .get(&input.conversation_id)
            .and_then(|c| {
                c.nodes
                    .iter()
                    .find(|n| n.id == input.variant_id)
                    .and_then(|n| n.active())
                    .map(|v| {
                        matches!(
                            v.status,
                            storyforge_domain::conversation::VariantStatus::Final
                        )
                    })
            })
            .unwrap_or(false);
        assertions.push(AssertionResult {
            name: "variant_final".into(),
            passed: final_ok,
            detail: None,
        });

        let all_pass = assertions.iter().all(|a| a.passed);
        ProductionAcceptResult {
            ok: all_pass,
            error: if all_pass {
                None
            } else {
                Some("one or more post-commit assertions failed".into())
            },
            force_accept: input.force_accept,
            turn_status: Some(final_status),
            attempt_status: Some(AttemptStatus::Committed),
            campaign_revision_before,
            campaign_revision_after,
            chronicle_revision_before,
            chronicle_revision_after,
            summary_code,
            draft_hash,
            assertions,
        }
    }
}

impl Default for CommitProbeEnv {
    fn default() -> Self {
        Self::new()
    }
}

fn fail_result(
    input: &ProductionAcceptInput,
    campaign_revision_before: u64,
    chronicle_revision_before: u64,
    draft_hash: String,
    mut assertions: Vec<AssertionResult>,
    err: &str,
) -> ProductionAcceptResult {
    assertions.push(AssertionResult {
        name: "accept_ok".into(),
        passed: false,
        detail: Some(err.into()),
    });
    ProductionAcceptResult {
        ok: false,
        error: Some(err.into()),
        force_accept: input.force_accept,
        turn_status: None,
        attempt_status: None,
        campaign_revision_before,
        campaign_revision_after: campaign_revision_before,
        chronicle_revision_before,
        chronicle_revision_after: chronicle_revision_before,
        summary_code: None,
        draft_hash,
        assertions,
    }
}

fn read_variant_content(conv_store: &ConversationStore, conv_id: &Id, node_id: &Id) -> String {
    let conv = match conv_store.get(conv_id) {
        Some(c) => c,
        None => return String::new(),
    };
    conv.nodes
        .iter()
        .find(|n| &n.id == node_id)
        .and_then(|node| node.active())
        .map(|v| v.content.clone())
        .unwrap_or_default()
}

/// 与生产 `next_chronicle_a_seq` 对齐：优先解析 A 级 code，否则用 turn。
pub fn next_chronicle_a_seq(existing: &[RoundSummary]) -> u32 {
    let mut max_seq = 0u32;
    for s in existing {
        if let Some(code) = s.code.as_deref() {
            if let Some(parsed) = ChronicleCode::parse(code)
                && parsed.level() == Some(ChronicleLevel::A)
                && let Ok(n) = code[1..].parse::<u32>()
            {
                max_seq = max_seq.max(n);
            }
        } else {
            max_seq = max_seq.max(s.turn);
        }
    }
    max_seq.saturating_add(1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::turn::{QualitySeverity, QualityWarning, QualityWarningCode};

    #[test]
    fn production_accept_persists_chronicle_a_and_bumps_revision() {
        let env = CommitProbeEnv::new();
        let (campaign_id, conversation_id) = env.bootstrap_campaign("eval-commit");
        let draft = "码头灯火摇曳，角色低声约定银鸦标记。".repeat(3);
        let variant_id = env.append_ai_draft(&conversation_id, &draft);
        let input = ProductionAcceptInput {
            campaign_id: campaign_id.clone(),
            conversation_id: conversation_id.clone(),
            variant_id,
            draft_text: draft,
            summary_text: Some("第1轮：银鸦标记在码头确立。".into()),
            turn_number: 1,
            quality_report: Some(QualityReport { warnings: vec![] }),
            force_accept: false,
        };
        env.prepare_awaiting_accept(&input);
        let result = env.accept_production(&input);
        assert!(result.ok, "accept failed: {:?}", result.error);
        assert_eq!(
            result.campaign_revision_after,
            result.campaign_revision_before + 1
        );
        assert_eq!(result.turn_status, Some(TurnStatus::Committed));
        let summaries = env.campaign_store.list_summaries(&campaign_id);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].code.as_deref(), Some("A0001"));
        assert!(result.chronicle_revision_after > result.chronicle_revision_before);
        env.cleanup();
    }

    #[test]
    fn production_accept_blocks_on_quality_error_without_force() {
        let env = CommitProbeEnv::new();
        let (campaign_id, conversation_id) = env.bootstrap_campaign("eval-block");
        let draft = "足够长的正文用于质量门禁拦截测试——角色在雨夜推进调查。".repeat(2);
        let variant_id = env.append_ai_draft(&conversation_id, &draft);
        let report = QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::PrivateKnowledgeLeak {
                    secret_fingerprint: "deadbeef".into(),
                    owner_id: Some("inst-chen".into()),
                },
                message: "private leak".into(),
                severity: QualitySeverity::Error,
            }],
        };
        let input = ProductionAcceptInput {
            campaign_id,
            conversation_id,
            variant_id,
            draft_text: draft,
            summary_text: Some("should not persist".into()),
            turn_number: 1,
            quality_report: Some(report),
            force_accept: false,
        };
        env.prepare_awaiting_accept(&input);
        let result = env.accept_production(&input);
        assert!(!result.ok);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("quality gate blocked")
        );
        env.cleanup();
    }

    #[test]
    fn production_accept_force_marks_degraded() {
        let env = CommitProbeEnv::new();
        let (campaign_id, conversation_id) = env.bootstrap_campaign("eval-degraded");
        let draft = "强制接受路径：角色带着警告继续推进主线，正文足够长以通过字数下限。".repeat(2);
        let variant_id = env.append_ai_draft(&conversation_id, &draft);
        let report = QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::FormatLeak {
                    snippet: "```".into(),
                },
                message: "format".into(),
                severity: QualitySeverity::Error,
            }],
        };
        let input = ProductionAcceptInput {
            campaign_id,
            conversation_id,
            variant_id,
            draft_text: draft,
            summary_text: Some("degraded commit still writes A".into()),
            turn_number: 1,
            quality_report: Some(report),
            force_accept: true,
        };
        env.prepare_awaiting_accept(&input);
        let result = env.accept_production(&input);
        assert!(result.ok, "{:?}", result.error);
        assert_eq!(result.turn_status, Some(TurnStatus::Degraded));
        env.cleanup();
    }
}
