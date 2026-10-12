//! harness-real-llm 的集成测试合成一个测试程序（每个文件一个模块），整个 crate 只链接一次。
//!
//! `endurance_sqlite_real_llm.rs` 留在 `tests/` 顶层单独成程序：它会改
//! `STORYFORGE_STORAGE_BACKEND` 环境变量，脚本也按 `--test endurance_sqlite_real_llm` 调它。
//!
//! 只跑其中一个文件：`cargo test -p harness-real-llm --test suite <文件名>::`；
//! 真模型评测照旧加 `-- --ignored`。

mod blind_arm_matrix_real_llm;
mod bronze_deterministic;
mod c_command_layer;
mod cache_context_observability;
mod card_translation_acceptance;
mod endurance_deterministic;
mod endurance_real_llm;
mod endurance_sqlite_deterministic;
mod eval_m5_phaseb_deterministic;
mod eval_m5_phaseb_real_llm;
mod evidence_retention_deterministic;
mod forge_differential;
mod i1_adversarial;
mod isolation_deterministic;
mod knowledge_propagation_real_llm;
mod m5_cache_and_memory;
mod m5_production_evidence;
mod real_llm_smoke_script;
mod t1_first_turn;
mod t2_multi_turn;
mod t3_regenerate;
mod writeback_isolation;
