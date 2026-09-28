//! M-03（SQLite 路径）：类型化 Patch 的 Campaign 锚定语义。
//!
//! 为什么本文件只覆盖这些断言（诚实边界）：
//! - M-03 的修复点「`patch.campaign_id` 必须等于目标 campaign」位于
//!   `commands/meta_typed.rs::patch_campaign_binding_ok`，由
//!   `meta_accept_typed_patch_with_writer` / `meta_preview_typed_patch_with_snapshot`
//!   调用——两者都是 `pub(crate)` 且需要 `AppState`（`AppState::new_for_test` 仅
//!   crate 内可见），集成测试二进制**不可达**；`tauri` 的 `test` feature 未启用，
//!   也无法用 mock app 走真实命令。因此「accept 拒绝 / preview stale / B 未写脏 /
//!   A 仍可接受」这四条**命令层**断言由
//!   `src/lib_tests_meta.rs::test_meta_typed_patch_is_bound_to_its_campaign`
//!   覆盖（命令层代码与后端无关）。
//! - 本文件用**真实 SQLite 后端**钉住 SQLite 写路径自身的保证与风险，正是审查
//!   指出的「旧用例用带 target id 的 action 会假通过」的反证材料：
//!   ① 无 target id 的 action（`UpdateCampaignVariable`）在 SQLite 仓层只认入参
//!   campaign 锚点——证明命令层绑定校验在 SQLite 上是**唯一**归属防线；
//!   ② 事务内 `expected_revision`（提案盖章）是 SQLite 路径的现实防线：盖章与
//!   目标战役 revision 不一致时跨战役 apply 被拒绝且目标未被写脏；
//!   ③ 同一 patch 在它自己的战役 A 上正常落库；
//!   ④ 带 target id 的 action 由 B 作用域内的 target 查找拒绝（旧用例的假通过
//!   路径仍然拒绝，但它覆盖不到 M-03）。
//!
//! `sqlite_runtime::activate` 是进程级单例，因此本二进制只允许**一个** test。

use std::sync::Arc;

use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_lib::{backend_workflows, sqlite_runtime, storage_backend};

fn sqlite_facade(dir: &std::path::Path) -> Arc<storage_backend::StorageFacade> {
    let facade = storage_backend::StorageFacade::new(
        dir.to_path_buf(),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    );
    facade
        .validate_runtime_authority()
        .expect("facade/runtime authority must match");
    Arc::new(facade)
}

#[test]
fn sqlite_meta_patch_campaign_binding_and_anchor_scoping() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");
    let facade = sqlite_facade(temp.path());

    // ── ① 无 target id 的 action：归属防线只在命令层，SQLite 层面靠盖章 ──
    let card_a = Id::from_str("binding-card-a");
    let mut campaign_a = Campaign::new(card_a.clone(), "Run A");
    campaign_a.id = Id::from_str("binding-camp-a");
    let mut campaign_b = Campaign::new(card_a.clone(), "Run B");
    campaign_b.id = Id::from_str("binding-camp-b");
    // B 的 revision 已被推进（写作中很常见）；A 的提案盖章指向 A 的 revision。
    campaign_b.revision = 7;
    sqlite_runtime::save_campaign(&campaign_a).expect("save campaign A");
    sqlite_runtime::save_campaign(&campaign_b).expect("save campaign B");

    // M-03 的关键形态：action **没有 target id**。旧用例都用带 target id 的
    // action（会被 target 查找先拒绝）→ 覆盖不到「A 的提案写进 B」。
    let no_target_actions = vec![
        storyforge_app_meta::TypedPatchAction::UpdateCampaignVariable {
            key: "weather".into(),
            value: serde_json::json!("storm"),
        },
    ];

    // 用 A 的盖章 revision 打 B：必须被 SQLite 事务内 revision 校验拒绝，
    // 且 B 一个变量都没写。
    // （注意：`Campaign::new` 会带默认变量如 weather=晴，所以判定「未写脏」要与
    //  调用前的快照比较，而不是断言 None。）
    let b_before = sqlite_runtime::get_campaign(&campaign_b.id)
        .unwrap()
        .expect("B 存在");
    let b_weather_before = b_before.get_variable("weather").cloned();
    let err = backend_workflows::apply_typed_patch_actions_for_backend(
        &facade,
        &campaign_b.id,
        &no_target_actions,
        Some(campaign_a.revision),
    )
    .expect_err("A 战役的提案盖章不得在 B 战役上落库");
    assert!(
        err.contains("revision mismatch"),
        "SQLite 路径的跨战役防线是事务内 revision 校验，got: {err}"
    );
    let b_after = sqlite_runtime::get_campaign(&campaign_b.id)
        .unwrap()
        .expect("B 仍存在");
    assert_eq!(
        b_after.get_variable("weather"),
        b_weather_before.as_ref(),
        "B 战役不得被 A 战役的提案写脏（weather 必须保持调用前的值）"
    );
    assert_ne!(
        b_after.get_variable("weather"),
        Some(&serde_json::json!("storm")),
        "B 战役不得收到 A 战役提案里的值"
    );

    // 同一份 actions 在它自己的战役 A 上正常落库（盖章 revision 匹配）。
    backend_workflows::apply_typed_patch_actions_for_backend(
        &facade,
        &campaign_a.id,
        &no_target_actions,
        Some(campaign_a.revision),
    )
    .expect("A 自己战役的补丁必须可以接受");
    let a_after = sqlite_runtime::get_campaign(&campaign_a.id)
        .unwrap()
        .expect("A 仍存在");
    assert_eq!(
        a_after.get_variable("weather"),
        Some(&serde_json::json!("storm")),
        "变量必须写在 A 战役"
    );
    let b_untouched = sqlite_runtime::get_campaign(&campaign_b.id)
        .unwrap()
        .expect("B 仍存在");
    assert_eq!(
        b_untouched.get_variable("weather"),
        b_weather_before.as_ref(),
        "B 始终未被写入"
    );

    // ── ② 带 target id 的 action 仍然被跨作用域 target 查找拒绝 ──
    let card_c = Id::from_str("binding-card-c");
    let mut campaign_c = Campaign::new(card_c.clone(), "Run C");
    campaign_c.id = Id::from_str("binding-camp-c");
    let mut campaign_d = Campaign::new(card_c.clone(), "Run D");
    campaign_d.id = Id::from_str("binding-camp-d");
    sqlite_runtime::save_campaign(&campaign_c).expect("save campaign C");
    sqlite_runtime::save_campaign(&campaign_d).expect("save campaign D");

    let instance = CharacterInstance {
        id: Id::from_str("binding-instance-c"),
        campaign_id: campaign_c.id.clone(),
        definition_id: None,
        name: "Hero".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    sqlite_runtime::save_instance(&instance).expect("save instance");

    let target_actions = vec![
        storyforge_app_meta::TypedPatchAction::UpdateInstanceVariable {
            instance_id: instance.id.clone(),
            key: "hp".into(),
            value: serde_json::json!(1),
        },
    ];
    let err = backend_workflows::apply_typed_patch_actions_for_backend(
        &facade,
        &campaign_d.id,
        &target_actions,
        None,
    )
    .expect_err("属于 C 的 target 不得在 D 上被改写");
    assert!(
        err.contains("binding-instance-c")
            || err.contains("not found")
            || err.contains("不存在")
            || err.contains("RecordNotFound"),
        "target 查找必须拒绝跨战役 target，got: {err}"
    );
    let instances_c = sqlite_runtime::list_instances(&campaign_c.id).unwrap();
    assert_eq!(instances_c.len(), 1);
    assert_eq!(
        instances_c[0].get_variable("hp"),
        None,
        "C 的实例不得被作用于 D 的 apply 改动"
    );
}
