//! 三审9：单一公共启动恢复入口。
//!
//! 生产 `lib.rs` setup hook 与子进程重启等价测试（`backend_parity_suite.rs`）
//! 共同调用 `run_startup_recovery`，消除原先两份近乎重复的 turn-recovery 包装器。
//!
//! 恢复内容（按 is_sqlite 分派）：
//! - SQLite：`recover_turns_on_startup`（内部已 `fail_incomplete_preaccept` +
//!   `fail_incomplete_turns` 覆盖未完成 preaccept/turn）+
//!   `recover_compress_jobs_on_startup`（Running→Pending 后 spawn worker）。
//! - JSON：TurnLifecycleService 重放 Committing 态 Turn + 标记非 terminal 活动 Turn
//!   Failed + `recover_compress_jobs_on_startup`。
//!
//! 幂等：内部捕获错误记日志不 panic（与既有 wrapper 行为一致），可安全重复调用。

use std::sync::Arc;

use crate::AppState;
use crate::backend_workflows::recover_compress_jobs_on_startup;
use crate::index_round_summaries_to_vector;
use crate::sqlite_runtime;
use crate::storage_backend;
use crate::turn_lifecycle::TurnLifecycleService;

/// 单一公共启动恢复入口：生产与子进程重启测试共同调用。
///
/// 幂等——二轮调用结果不变（Running→Pending 已迁移、Committing 已重放）。
pub fn run_startup_recovery(app_state: &Arc<AppState>) {
    if app_state.storage.is_sqlite() {
        // SQLite accept 是原子的：恢复 = 标记未完成 pipeline turn 为 Failed
        // （内部 fail_incomplete_preaccept + fail_incomplete_turns 已覆盖未完成
        // preaccept/turn，与既有 lib.rs 行为一致）。
        match sqlite_runtime::recover_turns_on_startup() {
            Ok(n) if n > 0 => {
                tracing::warn!(count = n, "sqlite recovery failed incomplete turns")
            }
            Ok(_) => {}
            Err(e) => tracing::error!("sqlite turn recovery failed: {e}"),
        }
    } else {
        // JSON：经 facade 拥有的 store 构造 TurnLifecycleService 重放。
        let campaign_store = app_state
            .storage()
            .json_campaign_store(
                storage_backend::BackendCapability::TurnLifecycle,
                "recover JSON turns",
            )
            .expect("JSON recovery requires the facade-owned CampaignStore");
        let turn_store = app_state
            .storage()
            .json_turn_store("recover JSON turns")
            .expect("JSON recovery requires the facade-owned TurnStore");
        let service = TurnLifecycleService::new(campaign_store, turn_store, &app_state.conv_store);
        service.recover_turns_on_startup(|batch| {
            // 启动恢复路径只做同步关键词索引，避免阻塞启动。
            index_round_summaries_to_vector(app_state.vector_store.as_ref(), batch);
        });
    }
    // 双后端：重放未完成 ChronicleCompressor 任务（Running→Pending 后 spawn worker）。
    recover_compress_jobs_on_startup(app_state.clone());
}
