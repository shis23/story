//! 无活动 Campaign 多角色写回原子性回归（Gate 4 七审 P1）。
//!
//! 旧缺陷：`meta_accept_patch` 无活动 Campaign 时逐角色调用
//! `update_character_world_info_entries_bulk`（SQLite 每次独立 UPDATE、JSON 每次
//! 独立 persist），第二个角色失败时第一个已永久更新——部分提交。修复后所有
//! 角色的新值经 facade `update_character_world_info_entries_bulk_multi` 在**单一
//! 原子操作**内整体写回（SQLite 单 UoW 事务 / JSON 单次 persist），任一步失败
//! 全部角色保持原值。
//!
//! 故障注入：种子两个角色（各自带非全局世界书条目）→ 无活动 Campaign →
//! 注入针对第二个角色的 UPDATE 失败触发器 → `meta_accept_patch` 失败 → 断言
//! 两个角色的世界书条目均保持原值（整体回滚，无部分提交）。
//!
//! `sqlite_runtime::activate` 是进程全局的，因此独立成文件（与既有 sqlite_*
//! 集成测试同模式），且只含一个测试函数。

use std::sync::Arc;

use storyforge_domain::Id;
use storyforge_domain::character::{CharacterCard, CharacterExtractionStatus};
use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_lib::sqlite_runtime;
use storyforge_lib::storage_backend::{CharacterInfo, StorageFacade};

fn sample_character_info(name: &str) -> CharacterInfo {
    CharacterInfo {
        source_character_id: None,
        name: name.to_string(),
        description: format!("{name} description"),
        personality: "calm".into(),
        scenario: "a forest".into(),
        first_mes: format!("Hello, I am {name}."),
        mes_example: String::new(),
        post_history_instructions: String::new(),
        alternate_greetings: vec![],
        system_prompt: String::new(),
        tags: vec!["test".into()],
        creator: "sqlite-meta-multi-role".into(),
        character_version: "1.0".into(),
        spec_version: "3.0".into(),
        extensions: serde_json::json!({}),
        embedded_world_info: None,
        renderable_assets: None,
        raw_card_json: serde_json::json!({ "spec": "3.0" }),
        has_world_info: false,
        has_renderable_assets: false,
        world_info_count: 0,
        world_info_entries: vec![storyforge_lib::storage_backend::WorldInfoEntryInfo {
            keys: vec![format!("{name}_global_key")],
            content: format!("{name} global content"),
            constant: false,
            route: "Selective".into(),
            is_global: false,
            depth: 2,
            order: 100,
        }],
    }
}

fn book_with_entry(key: &str) -> WorldInfoBook {
    WorldInfoBook {
        entries: vec![WorldInfoEntry {
            st_id: None,
            keys: vec![key.to_string()],
            secondary_keys: vec![],
            content: format!("{key} lore content"),
            constant: true, // Constant → is_global 分流回写所有角色
            selective: false,
            selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
            disabled: false,
            position: 0,
            depth: 2,
            order: 100,
            route: LoreRoute::Constant,
            extensions: serde_json::json!({}),
            extra: Default::default(),
        }],
        source: storyforge_domain::Source::Native,
        metadata: Default::default(),
    }
}

fn tauri_state_for_test(
    state: &Arc<storyforge_lib::AppState>,
) -> tauri::State<'_, Arc<storyforge_lib::AppState>> {
    unsafe {
        std::mem::transmute::<
            &Arc<storyforge_lib::AppState>,
            tauri::State<'_, Arc<storyforge_lib::AppState>>,
        >(state)
    }
}

#[test]
fn multi_role_world_info_write_failure_rolls_back_all_characters() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    // ─── 种子：两个角色（各自带非全局条目）+ 一张卡 + 无 Campaign ───────
    // 无活跃 Campaign → meta_accept_patch 走角色库维护路径。
    let card_id = Id::new();
    {
        sqlite_runtime::save_character(&sample_character_info("Alpha")).expect("save character A");
        sqlite_runtime::save_character(&sample_character_info("Beta")).expect("save character B");

        let card = CharacterCard {
            id: card_id.clone(),
            name: "Multi Role".into(),
            source_character_id: Id::new(),
            character_definitions: vec![],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({}),
            extraction_status: CharacterExtractionStatus::Unknown,
            extraction_message: None,
        };
        let stored_card = storyforge_lib::campaign_store::StoredCard {
            card,
            imported_at: "2026-07-31 00:00:00".to_string(),
        };
        let payload = serde_json::to_value(stored_card).expect("serialize stored card");
        // 卡需存在（角色库维护路径遍历 list_characters，卡本身无需 FK 引用）。
        // 但为完整性仍种一张卡。
        let _ = sqlite_runtime::save_card_payload(&card_id, "Multi Role", None, None, &payload);
    }

    let data_dir = temp.path().to_path_buf();
    let storage = Arc::new(StorageFacade::new(
        data_dir.clone(),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    ));
    storage
        .validate_runtime_authority()
        .expect("facade/runtime authority must match");
    let state = Arc::new(
        storyforge_lib::AppState::new_with_backend(data_dir.clone(), storage)
            .expect("SQLite AppState must construct"),
    );
    // 无活跃 Campaign：active_campaign 必须为 None。
    assert!(
        state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none()
    );

    // 排序控制：`list_characters` 按 (imported_at, character_id) 排序，批量写回
    // 遍历该顺序。为确保 Alpha（成功者）先于 Beta（注入故障者）更新——旧实现
    // 逐角色独立提交时，Alpha 会先被永久改写、Beta 后失败 → 部分提交必现——
    // 把 Alpha 的 imported_at 置早、Beta 置晚。
    let alpha_id = state
        .storage()
        .list_characters()
        .expect("list characters")
        .into_iter()
        .find(|c| c.info.name == "Alpha")
        .expect("Alpha exists")
        .id;
    let beta_id = state
        .storage()
        .list_characters()
        .expect("list characters")
        .into_iter()
        .find(|c| c.info.name == "Beta")
        .expect("Beta exists")
        .id;
    sqlite_runtime::with_db_raw_write(|conn| {
        conn.execute(
            "UPDATE characters SET imported_at = '2020-01-01 00:00:00' WHERE character_id = ?1",
            [&alpha_id],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .expect("backdate Alpha");
    sqlite_runtime::with_db_raw_write(|conn| {
        conn.execute(
            "UPDATE characters SET imported_at = '2020-01-01 00:00:01' WHERE character_id = ?1",
            [&beta_id],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .expect("stamp Beta later");

    // 造 patch（此时 tool_ctx.world_info 为空——无活跃 Campaign 且未 seed 书）。
    let patch_id = "meta-multi-role-fault".to_string();
    {
        let mut patches = state
            .meta_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        patches.push(storyforge_app_meta::Patch {
            id: patch_id.clone(),
            description: "multi-role fault".into(),
            actions: vec![storyforge_app_meta::PatchAction::Update {
                target: "world_info[0]".into(),
                field: "content".into(),
                value: serde_json::json!("patched global content"),
            }],
            created_at: chrono::Utc::now(),
            applied: false,
        });
    }

    // ─── Gate 4 七审 P1：无可修改的世界书 → 必须失败，不静默标 applied ──
    // 注意：SQLite 启动恢复（六审 P1）会从角色库注入 world_info，因此需显式
    // 清空以构造「无世界书」场景。
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.world_info = None;
    }
    assert!(
        state
            .tool_ctx
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .world_info
            .is_none(),
        "precondition: tool_ctx.world_info must be empty before seeding"
    );
    let empty_ctx_err =
        storyforge_lib::meta_accept_patch(patch_id.clone(), tauri_state_for_test(&state))
            .expect_err("no modifiable world info must fail, not silently succeed");
    assert!(
        !empty_ctx_err.to_string().is_empty(),
        "error must carry the missing-context reason, got: {empty_ctx_err}"
    );
    let applied_empty = state
        .meta_patches
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .find(|p| p.id == patch_id)
        .expect("patch exists")
        .applied;
    assert!(
        !applied_empty,
        "patch must NOT be marked applied when there is no modifiable world info"
    );

    // 注入世界书后继续（无活跃 Campaign → 角色库维护路径）。
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.world_info = Some(Arc::new(book_with_entry("global_lore")));
    }

    // ─── 故障注入：对第二个角色（Beta）的 UPDATE 抛错 ───────────────────
    // 批量方法按 list_characters 顺序遍历（imported_at, character_id 排序，
    // 已控制 Alpha 在前、Beta 在后）。触发器按 NEW.character_id = Beta 判定，
    // Alpha 的更新成功后、Beta 的更新抛 RAISE(ABORT)。
    sqlite_runtime::with_db_raw_write(|conn| {
        conn.execute_batch(&format!(
            "DROP TRIGGER IF EXISTS sf_multi_role_fault; \
             CREATE TRIGGER sf_multi_role_fault \
             BEFORE UPDATE OF info_json ON characters \
             FOR EACH ROW WHEN NEW.character_id = '{beta_id}' \
             BEGIN SELECT RAISE(ABORT, 'injected second character update failure'); END;"
        ))
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .expect("inject second-character update fault trigger");

    // ─── meta_accept_patch 必须失败，且两个角色均保持原值 ───────────────
    let err = storyforge_lib::meta_accept_patch(patch_id.clone(), tauri_state_for_test(&state))
        .expect_err("second-character write failure must propagate");
    assert!(
        !err.to_string().is_empty(),
        "error must carry failure, got: {err}"
    );

    // applied 必须保持 false。
    let applied = state
        .meta_patches
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .find(|p| p.id == patch_id)
        .expect("patch exists")
        .applied;
    assert!(
        !applied,
        "patch must NOT be applied when multi-role write failed"
    );

    // 两个角色的世界书条目都必须保持原值（整体回滚，无部分提交）。
    let all = state
        .storage()
        .list_characters()
        .expect("list characters after failure");
    for stored in &all {
        assert!(
            stored
                .info
                .world_info_entries
                .iter()
                .any(|e| e.content == format!("{} global content", stored.info.name)),
            "character {} must keep its original world info entry (rollback), got {:?}",
            stored.info.name,
            stored.info.world_info_entries
        );
        assert!(
            !stored
                .info
                .world_info_entries
                .iter()
                .any(|e| e.content == "patched global content"),
            "character {} must NOT receive the patched entry after rollback",
            stored.info.name
        );
    }
    // 确认 Beta 的触发器确实曾被命中（batch 顺序里 Beta 非首个即会触发）。
    // 至少验证字符 A（Alpha）未收到 patch——上面已覆盖。

    // 撤触发器后，成功路径：两个角色都收到 patch 后的全局条目。
    sqlite_runtime::with_db_raw_write(|conn| {
        conn.execute_batch("DROP TRIGGER IF EXISTS sf_multi_role_fault")
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .expect("drop fault trigger");
    storyforge_lib::meta_accept_patch(patch_id.clone(), tauri_state_for_test(&state))
        .expect("success path after trigger removed");
    let all_after = state
        .storage()
        .list_characters()
        .expect("list characters after success");
    for stored in &all_after {
        assert!(
            stored
                .info
                .world_info_entries
                .iter()
                .any(|e| e.content == "patched global content"),
            "character {} must receive the patched global entry on success",
            stored.info.name
        );
    }
}
