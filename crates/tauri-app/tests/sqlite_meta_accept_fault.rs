//! legacy Meta Patch 假成功回归测试（Gate 4 六审 P1）。
//!
//! 旧缺陷：`meta_accept_patch` 先改内存 `tool_ctx.world_info`，SQLite 写盘失败
//! 只记 warning，随后仍把 patch 标为 applied——前端显示采纳成功、权威数据库
//! 未更新。修复后：先在临时副本上执行 patch → 持久化（写盘）成功 → 才提交内存
//! + applied；写盘失败必须返回错误。
//!
//! 故障注入：种子角色/卡/Campaign → 激活（tool_ctx.world_info 注入）→ 用
//! `with_db_raw_write` 删除 Campaign 行（FK 破坏）→ `meta_accept_patch` 写回
//! 世界书时 UPSERT 触发 FK violation → 命令失败，且 `applied` 保持 false、
//! tool_ctx 世界书未被 patch 改写。
//!
//! `sqlite_runtime::activate` 是进程全局的，因此独立成文件（与既有 sqlite_*
//! 集成测试同模式），且只含一个测试函数。

use std::sync::Arc;

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
        creator: "sqlite-meta-fault".into(),
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
fn meta_accept_patch_write_failure_does_not_mark_applied_or_mutate_tool_ctx() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    // ─── 种子：角色（embedded 书）+ 卡 + Campaign（先于 AppState 构造）───
    let source_id = Id::new();
    let card_id = Id::new();
    let campaign_id = Id::new();
    {
        let info = sample_character_info(
            "Meta Hero",
            Some(source_id.as_str().to_string()),
            Some(book_with_entry("meta_lore")),
        );
        sqlite_runtime::save_character(&info).expect("save character");

        let card = CharacterCard {
            id: card_id.clone(),
            name: "Meta Hero".into(),
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
            "Meta Hero",
            Some(source_id.as_str()),
            None,
            &payload,
        )
        .expect("save card payload");

        let mut campaign = Campaign::new(card_id.clone(), "Meta Camp");
        campaign.id = campaign_id.clone();
        sqlite_runtime::save_campaign(&campaign).expect("save campaign");
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

    // 激活 → tool_ctx.world_info 注入 "meta_lore"。
    storyforge_lib::set_active_campaign(
        campaign_id.as_str().to_string(),
        tauri_state_for_test(&state),
    )
    .expect("activate campaign");
    let injected = state
        .tool_ctx
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .world_info
        .as_ref()
        .map(|b| b.entries[0].keys.clone());
    assert_eq!(injected, Some(vec!["meta_lore".to_string()]));

    // 造一个 Update patch，target 命中世界书第 0 条。
    let patch_id = "meta-patch-fault".to_string();
    {
        let mut patches = state
            .meta_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        patches.push(storyforge_app_meta::Patch {
            id: patch_id.clone(),
            description: "fault injection".into(),
            actions: vec![storyforge_app_meta::PatchAction::Update {
                target: "world_info[0]".into(),
                field: "content".into(),
                value: serde_json::json!("patched content"),
            }],
            created_at: chrono::Utc::now(),
            applied: false,
        });
    }

    // ─── 故障注入：触发器使该 Campaign 的世界书 UPSERT 失败 ─────────────
    // 删除 Campaign 会被 FK（子行引用）挡住；改用 BEFORE INSERT / BEFORE UPDATE
    // 两个触发器（SQLite 一个触发器只绑一个事件）对该 campaign 抛 RAISE(ABORT)
    // ——`set_world_info` 的 UPSERT 无论 INSERT 还是 UPDATE 分支命中即失败，
    // 等价于持久化故障。campaign_id 为本测试生成的随机 Id，拼接进 SQL 安全。
    let fault_campaign = campaign_id.as_str().to_string();
    sqlite_runtime::with_db_raw_write(|conn| {
        conn.execute_batch(&format!(
            "DROP TRIGGER IF EXISTS sf_meta_fault_wi_ins; \
             DROP TRIGGER IF EXISTS sf_meta_fault_wi_upd; \
             CREATE TRIGGER sf_meta_fault_wi_ins \
             BEFORE INSERT ON campaign_world_info \
             FOR EACH ROW WHEN NEW.campaign_id = '{fault_campaign}' \
             BEGIN SELECT RAISE(ABORT, 'injected world info write failure'); END; \
             CREATE TRIGGER sf_meta_fault_wi_upd \
             BEFORE UPDATE ON campaign_world_info \
             FOR EACH ROW WHEN NEW.campaign_id = '{fault_campaign}' \
             BEGIN SELECT RAISE(ABORT, 'injected world info write failure'); END;"
        ))
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .expect("inject targeted write fault triggers");

    // ─── meta_accept_patch 必须失败，且不标 applied、不改内存世界书 ─────
    let err = storyforge_lib::meta_accept_patch(patch_id.clone(), tauri_state_for_test(&state))
        .expect_err("write failure must propagate, not report false success");
    assert!(
        !err.to_string().is_empty(),
        "error should carry the persistence failure, got: {err}"
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
        "patch must NOT be marked applied when persistence failed"
    );

    // tool_ctx 世界书必须未被 patch 改写（内容仍是原始 "meta_lore lore content"）。
    let content = state
        .tool_ctx
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .world_info
        .as_ref()
        .map(|b| b.entries[0].content.clone());
    assert_eq!(
        content,
        Some("meta_lore lore content".to_string()),
        "tool_ctx world info must not be mutated when persistence failed"
    );

    // 补成功路径验证（写盘成功 → applied 置 true）：先撤触发器，再恢复
    // Campaign 行（经 sqlite_runtime::save_campaign 保证 payload 完整）。
    sqlite_runtime::with_db_raw_write(|conn| {
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS sf_meta_fault_wi_ins; \
             DROP TRIGGER IF EXISTS sf_meta_fault_wi_upd",
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .expect("drop fault triggers");
    let mut restored = Campaign::new(card_id.clone(), "Meta Camp");
    restored.id = campaign_id.clone();
    sqlite_runtime::save_campaign(&restored).expect("restore campaign row");

    // 重新激活（指针被前面失败命令保留为 campaign——失败原子性五审已保证）。
    storyforge_lib::set_active_campaign(
        campaign_id.as_str().to_string(),
        tauri_state_for_test(&state),
    )
    .expect("re-activate campaign after restore");
    storyforge_lib::meta_accept_patch(patch_id.clone(), tauri_state_for_test(&state))
        .expect("success path: persistence ok");
    let applied_now = state
        .meta_patches
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .find(|p| p.id == patch_id)
        .expect("patch exists")
        .applied;
    assert!(
        applied_now,
        "patch must be applied after successful persistence"
    );
    // 世界书落库内容已被 patch 改写。
    let persisted = state
        .storage()
        .get_world_info(&campaign_id)
        .expect("read world info")
        .entries[0]
        .content
        .clone();
    assert_eq!(
        persisted, "patched content",
        "persisted world info must carry the patch"
    );
}
