//! SQLite 并发回归：set_active_campaign 与 delete_character 的竞态（Gate 4 六审 P1）。
//!
//! 旧缺陷：激活命令写指针后释放锁、随后才更新 tool_ctx 世界书；角色删除则先删
//! 数据库、之后才取得同一把锁。并发时可能“指针已清空、旧 Campaign 世界书又被
//! 写回 tool_ctx”。修复后世界书写入作为 `after_commit` 钩子在 `active_campaign_
//! update` 锁内执行，与指针提交原子。
//!
//! **判别力设计**：`set_active_campaign_in_state` 的 `validate` 闭包在锁内执行。
//! 测试用它在锁内作可控屏障：
//! 1. 删除线程先删数据库（级联删 Campaign A），随后调 `delete_character` 阻塞在
//!    `active_campaign_update` 锁上；
//! 2. 激活线程调 `set_active_campaign_in_state`（目标 B），其 `validate` 在锁内
//!    `wait()` 暂停——此时锁被激活持有，删除线程不可能完成清理；
//! 3. 放行 validate → 激活写指针 + `after_commit`（置 `world_committed`）→ 释放锁；
//! 4. 删除线程拿到锁完成清理（置 `delete_done`）。
//!
//! 断言：`delete_done` 必须晚于 `world_committed`。若 `after_commit` 在锁外（旧
//! 实现），激活释放锁后删除线程先完成清理、after_commit 后执行 → 时序颠倒 →
//! 测试变红。因此本测试能区分修复前后。
//!
//! `sqlite_runtime::activate` 是进程全局的，因此独立成文件（与既有 sqlite_*
//! 集成测试同模式）。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
fn activation_world_commit_holds_lock_before_delete_proceeds() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    // ─── 种子：角色 X（embedded 书）+ 卡 X + Campaign A + Campaign B ────
    // 两张卡各自独立 Campaign：删除角色 X 只级联删 A；B 由激活线程切过去。
    let source_id = Id::new();
    let card_id = Id::new();
    let card_b_id = Id::new();
    let campaign_a = Id::new();
    let campaign_b = Id::new();
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

        // Campaign B 用独立卡（不随角色 X 级联删除）。
        let mut camp_b = Campaign::new(card_b_id.clone(), "Camp B");
        camp_b.id = campaign_b.clone();
        sqlite_runtime::save_campaign(&camp_b).expect("save campaign B");
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

    // ─── 判别力编排 ─────────────────────────────────────────────────────
    // 判别手段：after_commit 闭包内检查 `active_campaign_update` 锁是否仍被
    // 本线程持有。Rust 的 std::sync::Mutex **非重入**——若命令在锁内调用
    // after_commit（修复后），`try_lock()` 返回 Err(WouldBlock)；若命令在锁外
    // 调用（旧实现 drop(_update) 后），`try_lock()` 返回 Ok。这是确定性判别。
    let lock_held_during_commit = Arc::new(AtomicBool::new(false));
    let delete_done = Arc::new(AtomicBool::new(false));
    // validate 闭包在锁内执行；屏障钉住「激活持有锁、删除等锁」。
    let validate_entered = Arc::new(std::sync::Barrier::new(2));
    let release_validate = Arc::new(std::sync::Barrier::new(2));

    // 删除线程：先删库（级联删 A），随后 delete_character 尝试拿锁——若
    // after_commit 在锁外（旧实现），删除能在激活写世界书前完成清理。
    let delete_thread = {
        let state = Arc::clone(&state);
        let delete_done = Arc::clone(&delete_done);
        let source = source_id.as_str().to_string();
        std::thread::spawn(move || {
            storyforge_lib::delete_character(source, tauri_state_for_test(&state))
                .expect("delete character");
            delete_done.store(true, Ordering::SeqCst);
        })
    };

    // 激活线程：set_active_campaign_in_state(B)。validate 在锁内暂停；after_commit
    // 闭包检查锁是否仍被持有（锁内调用 → try_lock 失败；锁外调用 → try_lock 成功）。
    let activate_thread = {
        let state = Arc::clone(&state);
        let validate_entered = Arc::clone(&validate_entered);
        let release_validate = Arc::clone(&release_validate);
        let lock_held_during_commit = Arc::clone(&lock_held_during_commit);
        let id = campaign_b.clone();
        std::thread::spawn(move || {
            storyforge_lib::set_active_campaign_in_state(
                state.as_ref(),
                id,
                move || {
                    validate_entered.wait();
                    release_validate.wait();
                    Ok(())
                },
                move |state, _id| {
                    // 非重入 Mutex：若锁仍被本线程持有，try_lock 失败。
                    let lock_held = state.active_campaign_update.try_lock().is_err();
                    lock_held_during_commit.store(lock_held, Ordering::SeqCst);
                },
            )
            .expect("activate campaign B in state");
        })
    };

    // 等激活线程进入 validate（锁内）。
    validate_entered.wait();
    // 此刻激活持有锁、暂停在 validate；删除线程删库后阻塞在锁上。
    // 给删除线程时间删库（级联删 A）。
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if state
            .storage()
            .get_campaign(&campaign_a)
            .expect("read campaign A")
            .is_none()
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "delete thread must delete campaign A within timeout"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // 删除线程已删库、正阻塞在锁上（世界书尚未提交）。
    assert!(
        !delete_done.load(Ordering::SeqCst),
        "delete must be blocked while activation holds the lock"
    );

    // 放行 validate → 激活写指针 + after_commit（锁内 try_lock 检查）→ 释放锁。
    release_validate.wait();
    activate_thread.join().expect("activate thread join");
    delete_thread.join().expect("delete thread join");

    // ─── 判别力断言 ─────────────────────────────────────────────────────
    // 修复后 after_commit 在锁内执行 → try_lock 失败 → lock_held=true。
    // 旧实现（after_commit 在锁外）→ try_lock 成功 → lock_held=false → 断言红。
    assert!(
        lock_held_during_commit.load(Ordering::SeqCst),
        "after_commit (world-info write) must run while active_campaign_update \
         is still held: proves the lock wraps the commit"
    );
    assert!(
        delete_done.load(Ordering::SeqCst),
        "delete must complete after activation commits"
    );

    // 最终一致性：指针不指向已删 A；世界书不残留 A 的 "active" 书。
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
    // Campaign A 行已删。
    assert!(
        state
            .storage()
            .get_campaign(&campaign_a)
            .expect("read campaign A after delete")
            .is_none(),
        "campaign A must be deleted"
    );
}
