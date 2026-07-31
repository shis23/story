//! SQLite-native command lifecycle coverage (Gate 4 五审 P2).
//!
//! 既有“命令级测试”（lib_tests_campaigns.rs）走 `AppState::new_for_test()`，它
//! 固定为 JSON 后端，不能证明 SQLite 命令闭环成立。本二进制以真实 SQLite
//! AppState 驱动真实命令（`storyforge_lib::set_active_campaign` /
//! `delete_character`），验证：
//!
//! 1. `set_active_campaign`：指针提交前先经 backend-neutral facade 准备世界书；
//!    SQLite 下命令成功，指针置位，tool_ctx.world_info 注入模板。
//! 2. `set_active_campaign` 失败原子性（P1）：世界书读取失败（注入坏 payload）
//!    时命令返回错误，活跃指针保持原值、tool_ctx 世界书不被改写。
//! 3. `delete_character`：真实 SQLite 级联删除后，活跃指针清空、会话缓存失效、
//!    tool_ctx 角色移除。
//!
//! `sqlite_runtime::activate` 是进程全局的，因此本二进制独立成文件（与既有
//! sqlite_* 集成测试同模式），且只含一个测试函数以维持单 authority。

use std::sync::Arc;

use storyforge_domain::Id;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::character::{CharacterCard, CharacterExtractionStatus};
use storyforge_domain::world_info::{LoreRoute, WorldInfoBook, WorldInfoEntry};
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_lib::sqlite_runtime;
use storyforge_lib::storage_backend::{
    BackendCapability, CapabilityStatus, CharacterInfo, StorageFacade,
};

/// 种子角色卡（带 embedded world-info 模板，供 set_active_campaign 惰性种子）。
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
        creator: "sqlite-command-lifecycle".into(),
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

#[test]
fn sqlite_command_lifecycle_active_set_and_character_delete() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    // ─── 0. 真实 SQLite AppState ─────────────────────────────────────────
    // SQLite 已全局激活：AppState::new_with_backend 走 SQLite 分支
    // （ConversationStore 挂 SqliteConversationPersistence，storage 分派到
    // sqlite_runtime）。不触碰任何 JSON store。
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

    // ─── 1. 种子：角色库角色（含 embedded 世界书模板）+ 卡 + Campaign ──
    // 角色 source 与卡 source 一致：set_active_campaign 经 resolve_character
    // _world_info_template 用 source 反查角色模板；delete_character 级联用
    // source 反查卡 → Campaign。
    let source_id = Id::new();
    let card_id = Id::new();
    let campaign_id = Id::new();
    {
        let book = book_with_entry("village");
        let info = sample_character_info("Elena", Some(source_id.as_str().to_string()), Some(book));
        state
            .storage()
            .save_character(info)
            .expect("save character");

        // 卡：payload 走 save_card_payload（V001 character_cards 表）。payload
        // 形状 = Tauri StoredCard（{ card, imported_at }），与生产加载器一致。
        let card = CharacterCard {
            id: card_id.clone(),
            name: "Elena".into(),
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
            "Elena",
            Some(source_id.as_str()),
            None,
            &payload,
        )
        .expect("save card payload");

        // Campaign（V001 campaigns 表，FK → character_cards）。
        let mut campaign = Campaign::new(card_id.clone(), "Command Lifecycle Camp");
        campaign.id = campaign_id.clone();
        sqlite_runtime::save_campaign(&campaign).expect("save campaign");

        // 会话（conversations 表无 FK，独立种子；campaign 绑定 conv_id）。
        let conv = state
            .conv_store
            .create_persisted(None, Some(campaign_id.clone()))
            .expect("create conversation");
        let mut loaded = sqlite_runtime::get_campaign(&campaign_id)
            .expect("read campaign")
            .expect("campaign exists");
        loaded.conversation_id = Some(conv.id.clone());
        sqlite_runtime::save_campaign(&loaded).expect("bind conversation id");
    }

    // ─── 2. set_active_campaign：指针前准备世界书 → 成功 + 注入 ──────────
    assert_eq!(
        state.storage().capability(BackendCapability::WorldInfo),
        CapabilityStatus::Supported,
        "WorldInfo must be Supported under SQLite"
    );
    storyforge_lib::set_active_campaign(
        campaign_id.as_str().to_string(),
        tauri_state_for_test(&state),
    )
    .expect("set_active_campaign must succeed over SQLite");

    let active = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    assert_eq!(
        active.as_ref(),
        Some(&campaign_id),
        "active pointer must be set"
    );
    // tool_ctx.world_info 被注入（模板：本局世界书为空 → ensure_world_info
    // _from_book 从角色库 embedded 世界书种子 → 应用层注入）。
    let injected = state
        .tool_ctx
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .world_info
        .as_ref()
        .map(|book| {
            book.entries
                .iter()
                .map(|e| e.keys.clone())
                .collect::<Vec<_>>()
        });
    assert_eq!(
        injected,
        Some(vec![vec!["village".to_string()]]),
        "tool_ctx.world_info must carry the seeded template"
    );
    // 世界书已落库（ensure_world_info_from_book 写入 V006 campaign_world_info）。
    let persisted_book = state
        .storage()
        .get_world_info(&campaign_id)
        .expect("read world info");
    assert_eq!(
        persisted_book.entries.len(),
        1,
        "world info persisted after seed"
    );
    assert_eq!(persisted_book.entries[0].keys, vec!["village".to_string()]);

    // ─── 3. 失败原子性（P1）：世界书读取失败 → 命令失败但指针不变 ────────
    // 先把指针切到 campaign_b；对 campaign_b 的世界书行注入坏 payload（经
    // sqlite_runtime 测试写钩子），使 set_active_campaign 读取世界书失败。
    let campaign_b_id = Id::new();
    {
        let mut campaign_b = Campaign::new(card_id.clone(), "Broken World Info Camp");
        campaign_b.id = campaign_b_id.clone();
        sqlite_runtime::save_campaign(&campaign_b).expect("save campaign b");
        let bad_payload = "{ this is not valid world info json }";
        sqlite_runtime::with_db_raw_write(|conn| {
            conn.execute(
                "INSERT INTO campaign_world_info (campaign_id, payload_json, updated_at) \
                 VALUES (?1, ?2, '')",
                rusqlite::params![campaign_b_id.as_str(), bad_payload],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .expect("inject corrupted world info row");
    }
    // 当前指针是 campaign_a；目标 campaign_b 的世界书反序列化失败 → 命令必须
    // 失败且指针保持 campaign_a，绝不能切到 campaign_b。
    let err = storyforge_lib::set_active_campaign(
        campaign_b_id.as_str().to_string(),
        tauri_state_for_test(&state),
    )
    .expect_err("set_active_campaign must fail on corrupted world info");
    // 注入的坏 payload（非法 JSON）→ 仓库层 get_world_info_payload 解析失败
    // 直接上抛。错误内容无需精确匹配（serde 报错文案随注入文本变化），关键
    // 断言是命令失败 + 指针不变 + tool_ctx 未被改写（下方三条）。
    assert!(!err.to_string().is_empty(), "command must fail, got: {err}");
    let active_after = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    assert_eq!(
        active_after.as_ref(),
        Some(&campaign_id),
        "failed set_active_campaign must not change the active pointer"
    );
    // tool_ctx 世界书仍是被注入的 campaign_a 模板，未被失败命令改写。
    let injected_after = state
        .tool_ctx
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .world_info
        .as_ref()
        .map(|book| {
            book.entries
                .iter()
                .map(|e| e.keys.clone())
                .collect::<Vec<_>>()
        });
    assert_eq!(
        injected_after,
        Some(vec![vec!["village".to_string()]]),
        "failed set_active_campaign must not rewrite tool_ctx.world_info"
    );

    // ─── 4. delete_character：真实 SQLite 级联 → 指针/缓存/tool_ctx 清理 ─
    // 删除角色（source 反查卡 → Campaign → 级联删 conversations）。
    storyforge_lib::delete_character(source_id.as_str().to_string(), tauri_state_for_test(&state))
        .expect("delete_character must succeed over SQLite");

    // 活跃 Campaign 已被级联删除 → 指针必须清空。
    assert!(
        state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none(),
        "active pointer must clear after deleting the active character's campaign"
    );
    // 会话缓存必须失效（级联删行 + conv_store.delete 同步缓存）。
    assert!(
        state
            .conv_store
            .list()
            .iter()
            .all(|c| c.campaign_id.as_ref() != Some(&campaign_id)),
        "conversation cache must drop the deleted campaign's conversation"
    );
    // tool_ctx 角色移除 + 世界书重建（角色库已删该角色）。
    let ctx = state.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
    assert!(
        ctx.characters.iter().all(|c| c.name != "Elena"),
        "tool_ctx characters must drop the deleted character"
    );
    // 库内行已删（角色 + 卡 + campaign + conversations）。
    assert!(
        state
            .storage()
            .get_character("Elena")
            .expect("read")
            .is_none()
    );
    assert!(
        state
            .storage()
            .get_campaign(&campaign_id)
            .expect("read campaign after delete")
            .is_none()
    );
    // 注：campaign_b 复用同一 card_id，删除角色级联会连同该卡下所有 campaign
    // 一起删除——这是卡级级联的正确语义（作用域逐行计数已由
    // sqlite_character_lifecycle.rs 覆盖），此处不再断言。
}

/// 集成测试无法访问 `crate::lib_tests_startup::tauri_state_for_test`（pub(super)），
/// 复刻同一 tauri::State 构造：Tauri State 无公开构造器，命令级测试需要与
/// invoke 相同的包装类型。
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
