//! #22：SQLite 后端 MVU 翻译权威 + 收集链路证明。
//!
//! 独立集成测试二进制：`sqlite_runtime::activate` 是进程级 OnceLock，
//! 必须在自己的进程里激活，不能混进 lib 单测（会翻转其他测试的后端分支）。
//!
//! 覆盖：save/get/list/delete 往返、后处理规则收集（按 source 卡去重）、
//! fallback 片段收集、meta 读命令数据链同源。

use std::sync::Arc;

use storyforge_app_pipeline::WritingContext;
use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
use storyforge_domain::mvu_translation::{FallbackFragment, MvuTranslation};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_lib::sqlite_runtime;
use storyforge_lib::{
    collect_mvu_fallback_fragments_for_backend, collect_mvu_update_rules_for_backend,
    load_sqlite_campaign_context_snapshot, storage_backend::StorageFacade,
};

#[test]
fn sqlite_mvu_translation_authority_and_collectors() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate sqlite runtime");
    assert!(sqlite_runtime::is_sqlite_active());
    let storage = StorageFacade::new(
        temp.path().to_path_buf(),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    );

    // ── 卡 payload（StoredCard 形态）+ 两个 definition 供 def→source 反查 ──
    let card_id = Id::from_str("card-mvu");
    let source_id = Id::from_str("source-mvu");
    let def_a = CharacterDefinition {
        id: Id::from_str("def-a"),
        card_id: card_id.clone(),
        name: "Alice".into(),
        persona_prompt: "工程师".into(),
        behavior_rules: String::new(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: storyforge_domain::variables::default_character_variables(),
    };
    let def_b = CharacterDefinition {
        id: Id::from_str("def-b"),
        card_id: card_id.clone(),
        name: "Bob".into(),
        persona_prompt: "医生".into(),
        behavior_rules: String::new(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Supporting,
        variable_schema: storyforge_domain::variables::default_character_variables(),
    };
    let card = CharacterCard {
        id: card_id.clone(),
        name: "MVU 测试卡".into(),
        source_character_id: source_id.clone(),
        character_definitions: vec![def_a.clone(), def_b.clone()],
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::Value::Null,
        extraction_status: Default::default(),
        extraction_message: None,
    };
    let stored_card = serde_json::json!({
        "card": serde_json::to_value(&card).unwrap(),
        "imported_at": "2026-07-27T00:00:00Z",
    });
    sqlite_runtime::save_card_payload(
        &card_id,
        "MVU 测试卡",
        Some(source_id.as_str()),
        Some("2026-07-27T00:00:00Z"),
        &stored_card,
    )
    .expect("card payload persists");

    // ── MVU 翻译 save/get/list 往返 ──
    let mut translation = MvuTranslation::pure_data_fallback(vec![]);
    translation.update_rules = vec![
        "受到伤害时降低 hp".into(),
        "  ".into(), // 空白规则应被收集端过滤
    ];
    translation.fallback_fragments = vec![
        FallbackFragment {
            description: "探针".into(),
            js_snippet: "variables.probe = true;".into(),
            reason: "JS 专用".into(),
        },
        FallbackFragment {
            description: "空片段应被过滤".into(),
            js_snippet: String::new(),
            reason: String::new(),
        },
    ];
    let stored_mvu = storyforge_lib::campaign_store::StoredMvuTranslation {
        source_character_id: source_id.clone(),
        character_name: "Alice".into(),
        translation,
        analyzed_at: "2026-07-27T00:00:00Z".into(),
    };
    sqlite_runtime::save_mvu(&stored_mvu).expect("mvu translation persists");

    let loaded = sqlite_runtime::get_mvu(&source_id)
        .expect("get_mvu ok")
        .expect("translation exists");
    assert_eq!(loaded.character_name, "Alice");
    assert_eq!(loaded.translation.update_rules.len(), 2);
    assert_eq!(sqlite_runtime::list_mvu().expect("list ok").len(), 1);

    // ── 收集链路：campaign runtime 两个实例同卡 → 规则去重、片段过滤 ──
    let campaign = Campaign::new(card_id.clone(), "MVU campaign".to_string());
    sqlite_runtime::save_campaign(&campaign).expect("campaign persists for context compilation");
    let instance_a = CharacterInstance {
        id: Id::from_str("inst-a"),
        campaign_id: campaign.id.clone(),
        definition_id: Some(def_a.id.clone()),
        name: "Alice".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    let instance_b = CharacterInstance {
        id: Id::from_str("inst-b"),
        campaign_id: campaign.id.clone(),
        definition_id: Some(def_b.id.clone()),
        name: "Bob".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    let runtime = CampaignRuntimeContext {
        campaign: campaign.clone(),
        instances: vec![instance_a, instance_b],
        definitions_by_id: Default::default(),
        knowledge: vec![],
        tasks: vec![],
        turn: 1,
    };
    let mut ctx = WritingContext::legacy(vec![], None, Id::from_str("conv-mvu"));
    ctx.campaign_runtime = Some(Arc::new(runtime));

    // 同卡两实例在场：规则只贡献一次（source 去重），空白规则被过滤
    let rules = collect_mvu_update_rules_for_backend(
        &storage,
        &ctx,
        &["Alice".to_string(), "Bob".to_string()],
    )
    .expect("SQLite MVU rule collection succeeds");
    assert_eq!(rules, vec!["受到伤害时降低 hp".to_string()]);

    // fallback 片段：空 js_snippet 被过滤（无去重语义，与 JSON 路径一致）
    let fragments =
        collect_mvu_fallback_fragments_for_backend(&storage, &ctx, &["Alice".to_string()])
            .expect("SQLite MVU fragment collection succeeds");
    assert_eq!(fragments.len(), 1);
    assert_eq!(fragments[0].js_snippet, "variables.probe = true;");

    // 未知角色 / 无 definition 命中 → 空集不报错
    assert!(
        collect_mvu_update_rules_for_backend(&storage, &ctx, &["无名氏".to_string()])
            .expect("an unknown character is a successful empty match")
            .is_empty()
    );

    let card_corruptor = Database::open(&db_path).expect("open card corruption probe");
    card_corruptor
        .connection()
        .execute(
            "UPDATE character_cards SET payload_json = '{' WHERE card_id = ?1",
            [card_id.as_str()],
        )
        .expect("corrupt the authoritative card payload");
    drop(card_corruptor);
    let context_error = match load_sqlite_campaign_context_snapshot(&storage, &campaign.id) {
        Err(error) => error,
        Ok(_) => panic!("SQLite card decode failures must not produce an empty Campaign context"),
    };
    assert!(
        !context_error.trim().is_empty(),
        "SQLite card corruption must produce a diagnostic"
    );
    sqlite_runtime::save_card_payload(
        &card_id,
        "MVU 测试卡",
        Some(source_id.as_str()),
        Some("2026-07-27T00:00:00Z"),
        &stored_card,
    )
    .expect("restore card payload after the error propagation probe");

    // ── 删除级联往返 ──
    assert!(sqlite_runtime::delete_mvu(&source_id).expect("delete ok"));
    assert!(!sqlite_runtime::delete_mvu(&source_id).expect("second delete ok"));
    assert!(
        sqlite_runtime::get_mvu(&source_id)
            .expect("get ok")
            .is_none()
    );
    assert!(
        collect_mvu_update_rules_for_backend(&storage, &ctx, &["Alice".to_string()])
            .expect("a missing translation is a successful empty match")
            .is_empty()
    );

    sqlite_runtime::save_mvu(&stored_mvu).expect("restore translation for error propagation");
    let corruptor = Database::open(&db_path).expect("open an independent corruption probe");
    corruptor
        .connection()
        .execute(
            "UPDATE mvu_translations SET payload_json = '{' WHERE source_character_id = ?1",
            [source_id.as_str()],
        )
        .expect("corrupt the authoritative MVU payload");
    drop(corruptor);
    let error = collect_mvu_update_rules_for_backend(&storage, &ctx, &["Alice".to_string()])
        .expect_err("SQLite MVU decode failures must not be disguised as an empty rule set");
    let normalized_error = error.to_ascii_lowercase();
    assert!(
        normalized_error.contains("mvu")
            || normalized_error.contains("json")
            || normalized_error.contains("deserialize"),
        "unexpected SQLite MVU error: {error}"
    );
}
