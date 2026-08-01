//! SQLite 并发回归：set_active_campaign 与 delete_character 的竞态（Gate 4 六审 P1）。
//!
//! 旧缺陷：激活命令写指针后释放锁、随后才更新 tool_ctx 世界书；角色删除则先删
//! 数据库、之后才取得同一把锁。并发时可能“指针已清空、旧 Campaign 世界书又被
//! 写回 tool_ctx”。修复后世界书写入作为 `after_commit` 钩子在 `active_campaign_
//! update` 锁内执行，与指针提交原子。
//!
//! 本测试用 `active_campaign_update` 锁作**可控屏障**，确定性复现危险窗口：
//! 1. 主线程持有锁；
//! 2. 删除线程先删数据库（角色级联删掉 Campaign A），随后阻塞在锁上——这正是
//!    旧实现里“数据库已删、指针尚在、世界书待写”的窗口；
//! 3. 激活线程对已删除的 A 重新激活，同样阻塞在锁上；
//! 4. 释放锁后两个线程串行完成。
//!
//! 断言最终状态一致：指针绝不指向已删除的 A，tool_ctx.world_info 绝不残留 A
//! 的书，A 的 Campaign 行已删除。
//!
//! `sqlite_runtime::activate` 是进程全局的，因此独立成文件（与既有 sqlite_*
//! 集成测试同模式）。

use std::sync::Arc;
use std::time::Duration;

use storyforge_domain::Id;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::character::{CharacterCard, CharacterExtractionStatus};
use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_lib::sqlite_runtime;
use storyforge_lib::storage_backend::{CharacterInfo, StorageFacade};

fn sample_character_info(
    name: &str,
    source_character_id: Option<String>,
    book: Option<WorldInfoBook>,
) -> CharacterInfo {
    CharacterInfo {
        source_character_id,
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
        creator: "sqlite-command-concurrency".into(),
        character_version: "1.0".into(),
        spec_version: "3.0".into(),
        extensions: serde_json::json!({}),
        embedded_world_info: book,
        renderable_assets: None,
        raw_card_json: serde_json::json!({ "spec": "3.0" }),
        has_world_info: false,
        has_renderable_assets: false,
        world_info_count: 0,
        world_info_entries: vec![],
    }
}

fn book_with_entry(key: &str) -> WorldInfoBook {
    WorldInfoBook {
        entries: vec![WorldInfoEntry {
            st_id: None,
            keys: vec![key.to_string()],
            secondary_keys: vec![],
            content: format!("{key} lore content"),
            constant: false,
            selective: true,
            selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
            disabled: false,
            position: 0,
            depth: 2,
            order: 100,
            route: LoreRoute::Selective,
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
fn concurrent_delete_and_reactivate_never_leave_stale_world_info() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    // ─── 种子：角色 X（含 embedded 书 "active"）+ 卡 X + Campaign A ───────
    // 只有一套卡/Campaign：delete_character 的级联只影响 A，删掉即没有其它
    // Campaign 可切——这让“对已删 A 重新激活”必然失败（not_found），最终指针
    // 只能为 None，世界书只能为空，从而唯一确定地暴露旧窗口。
    let source_id = Id::new();
    let card_id = Id::new();
    let campaign_a = Id::new();
    {
        let info = sample_character_info(
            "Race Hero",
            Some(source_id.as_str().to_string()),
            Some(book_with_entry("active")),
        );
        sqlite_runtime::save_character(&info).expect("save character");

        let card = CharacterCard {
            id: card_id.clone(),
            name: "Race Hero".into(),
            source_character_id: source_id.clone(),
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
        sqlite_runtime::save_card_payload(
            &card_id,
            "Race Hero",
            Some(source_id.as_str()),
            None,
            &payload,
        )
        .expect("save card payload");

        let mut camp_a = Campaign::new(card_id.clone(), "Camp A");
        camp_a.id = campaign_a.clone();
        sqlite_runtime::save_campaign(&camp_a).expect("save campaign A");
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

    // 先激活 A（tool_ctx.world_info = "active"）。
    storyforge_lib::set_active_campaign(
        campaign_a.as_str().to_string(),
        tauri_state_for_test(&state),
    )
    .expect("activate campaign A");
    assert!(
        state
            .tool_ctx
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .world_info
            .as_ref()
            .is_some(),
        "world_info must be injected after activating A"
    );

    // ─── 可控屏障：主线程持有 update 锁，编排删除→激活 ─────────────────
    let lock_guard = state
        .active_campaign_update
        .lock()
        .unwrap_or_else(|p| p.into_inner());

    // 删除线程：先删数据库（级联删 A），然后阻塞在 update 锁上。
    let delete_thread = {
        let state = Arc::clone(&state);
        let source = source_id.as_str().to_string();
        std::thread::spawn(move || {
            // 先删库——此时锁仍被主线程持有，删除在库已删、指针待清的状态阻塞。
            storyforge_lib::delete_character(source, tauri_state_for_test(&state))
        })
    };

    // 激活线程：对已删除的 A 重新激活（会 not_found），同样阻塞在锁上。
    let activate_thread = {
        let state = Arc::clone(&state);
        let id = campaign_a.as_str().to_string();
        std::thread::spawn(move || {
            storyforge_lib::set_active_campaign(id, tauri_state_for_test(&state))
        })
    };

    // 给删除线程足够时间完成数据库级联（删 A）并到达锁。
    // 轮询 DB 直到 A 行消失（证明删除线程已过了“删库”阶段、正阻塞在锁上）。
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let a_gone = state
            .storage()
            .get_campaign(&campaign_a)
            .expect("read campaign A")
            .is_none();
        if a_gone {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "delete thread must delete campaign A from the DB within timeout"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // 危险窗口此刻被钉死：DB 已删 A，指针仍指向 A，世界书仍是 A 的书。
    // 释放锁——删除与激活串行完成。修复后二者都在锁内原子提交，绝不出现
    // “指针清空、世界书残留 A 的书”。
    drop(lock_guard);

    let delete_result = delete_thread.join().expect("delete thread join");
    let activate_result = activate_thread.join().expect("activate thread join");

    // ─── 最终一致性断言 ─────────────────────────────────────────────────
    delete_result.expect("delete must succeed");
    // 对已删 A 重新激活必须失败（campaign 不存在）。
    assert!(
        activate_result.is_err(),
        "re-activating a deleted campaign must fail"
    );

    // 指针绝不能指向已删除的 A。
    let active = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    assert_ne!(
        active.as_ref(),
        Some(&campaign_a),
        "active pointer must never point at the deleted campaign A"
    );

    // tool_ctx.world_info 绝不残留 A 的 "active" 书（删除已重建/清空世界书）。
    let world_keys: Vec<String> = state
        .tool_ctx
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .world_info
        .as_ref()
        .map(|book| {
            book.entries
                .iter()
                .map(|e| e.keys.join(","))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        !world_keys.iter().any(|k| k == "active"),
        "tool_ctx.world_info must not carry the deleted campaign A's book, got {world_keys:?}"
    );

    // Campaign 行已删除。
    assert!(
        state
            .storage()
            .get_campaign(&campaign_a)
            .expect("read campaign A after delete")
            .is_none(),
        "campaign A must be deleted"
    );
}
