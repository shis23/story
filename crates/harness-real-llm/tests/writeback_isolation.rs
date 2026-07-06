//! 写回隔离 + 工具白名单钉测（确定性，不需 LLM）。
//!
//! 钉 postprocess 落盘的隔离判定 `is_postprocess_instance_present`（线上同款纯函数），
//! 覆盖审计标的 P3/P4/P6/P8。这些是"清晰泄漏立即修"范围外的设计取舍/边界，
//! 按计划只钉测 + 报告，不改业务行为（P0 已修，B0 在 app-agent 钉过）。

use std::collections::HashSet;

use storyforge_app_agent::ToolContext;
use storyforge_app_agent::ToolError;
use storyforge_app_agent::tools::{ToolRegistry, register_subagent_tools};
use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};
use storyforge_tauri_app::campaign_store;
use storyforge_tauri_app::is_postprocess_instance_present;
use storyforge_tauri_app::normalize_knowledge_update_for_postprocess;

/// P3 钉：is_postprocess_instance_present 空集仍放行（变量路径向后兼容）。
/// P3 分流后，知识路径的空集行为由 normalize_knowledge_update_for_postprocess 按 source 控制。
#[test]
fn b3_empty_present_chars_escape_hatch_current_behavior() {
    let inst = CharacterInstance::temporary(Id::from_str("camp-1"), "缺席角色");
    let raw_id = Id::from_str("inst-absent");
    let empty: HashSet<String> = HashSet::new();

    // 门禁本身：空集 ⇒ 全过（变量路径仍依赖此行为）
    let passes = is_postprocess_instance_present(&inst, &raw_id, &empty, &HashSet::new());
    assert!(
        passes,
        "is_postprocess_instance_present 空集仍放行（变量路径向后兼容）"
    );
}

/// P3 核心修复：空集 + Witnessed → normalize 拒绝（不再全放行）。
#[test]
fn b3_empty_witnessed_is_rejected() {
    let dir = std::env::temp_dir().join(format!("sf_b3_empty_witnessed_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();
    let mut inst = CharacterInstance::temporary(campaign.id.clone(), "缺席角色");
    inst.id = Id::from_str("inst-absent");
    store.add_instance(inst).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("缺席角色"),
        knowledge_text: "test".into(),
        source: KnowledgeSource::Witnessed,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    let empty: HashSet<String> = HashSet::new();

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        1,
        &empty,
        &HashSet::new(),
    );
    assert!(entries.is_empty(), "空集 + Witnessed 应被 P3 分流拒绝");

    let _ = std::fs::remove_dir_all(&dir);
}

/// P3：空集 + ToldByOther → 放行（跨在场告知不受在场约束）。
#[test]
fn b3_empty_told_by_other_passes() {
    let dir = std::env::temp_dir().join(format!("sf_b3_empty_tbo_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();
    let mut inst = CharacterInstance::temporary(campaign.id.clone(), "缺席角色");
    inst.id = Id::from_str("inst-absent");
    store.add_instance(inst).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("缺席角色"),
        knowledge_text: "told by someone".into(),
        source: KnowledgeSource::ToldByOther,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    let empty: HashSet<String> = HashSet::new();

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        1,
        &empty,
        &HashSet::new(),
    );
    assert_eq!(
        entries.len(),
        1,
        "空集 + ToldByOther 应放行（P3 分流：不受在场约束）"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// P3：非空 present_ids（不含目标）+ ToldByOther → 放行（跨在场告知）。
#[test]
fn b3_told_by_other_bypasses_presence() {
    let dir = std::env::temp_dir().join(format!("sf_b3_tbo_bypass_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();
    let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    lin.id = Id::from_str("inst-lin");
    store.add_instance(lin).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Lin"),
        knowledge_text: "Chen told Lin".into(),
        source: KnowledgeSource::ToldByOther,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    // present 含 "Chen"，不含 Lin
    let present = HashSet::from([String::from("Chen")]);

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        1,
        &present,
        &HashSet::new(),
    );
    assert_eq!(entries.len(), 1, "ToldByOther 应绕过在场检查（跨在场告知）");

    let _ = std::fs::remove_dir_all(&dir);
}

/// P3：非空 present_ids（不含目标）+ Backstory → 放行（开局已有）。
#[test]
fn b3_backstory_bypasses_presence() {
    let dir = std::env::temp_dir().join(format!("sf_b3_backstory_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();
    let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    lin.id = Id::from_str("inst-lin");
    store.add_instance(lin).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Lin"),
        knowledge_text: "Lin's backstory".into(),
        source: KnowledgeSource::Backstory,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    let present = HashSet::from([String::from("Chen")]);

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        1,
        &present,
        &HashSet::new(),
    );
    assert_eq!(entries.len(), 1, "Backstory 应绕过在场检查（开局已有）");

    let _ = std::fs::remove_dir_all(&dir);
}

/// P3：非空 present_ids（不含目标）+ Witnessed → 拒绝（不在场不可能亲眼见）。
#[test]
fn b3_witnessed_respects_presence() {
    let dir = std::env::temp_dir().join(format!("sf_b3_witnessed_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();
    let mut chen = CharacterInstance::temporary(campaign.id.clone(), "Chen");
    chen.id = Id::from_str("inst-chen");
    store.add_instance(chen).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Chen"),
        knowledge_text: "Chen saw something".into(),
        source: KnowledgeSource::Witnessed,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    // present 只含 Lin，不含 Chen
    let present = HashSet::from([String::from("Lin")]);

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        1,
        &present,
        &HashSet::new(),
    );
    assert!(entries.is_empty(), "Witnessed 且不在场应被拒绝（P3 分流）");

    let _ = std::fs::remove_dir_all(&dir);
}

/// P4 钉：present_chars 含 "Lin"，给缺席角色 "Chen"/inst-chen 写回 ⇒ 拒。
/// 同时钉 name/id 三路匹配的合法情况：present 含 inst.id 或 name 都算在场。
#[test]
fn b4_present_chars_name_id_matching() {
    let inst_lin = CharacterInstance {
        id: Id::from_str("inst-lin"),
        campaign_id: Id::from_str("camp-1"),
        definition_id: None,
        name: "Lin".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    let inst_chen = CharacterInstance {
        id: Id::from_str("inst-chen"),
        campaign_id: Id::from_str("camp-1"),
        definition_id: None,
        name: "Chen".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    let mut present: HashSet<String> = HashSet::new();
    present.insert("Lin".into()); // 只 Lin 在场

    // Chen（缺席）写回 ⇒ 拒
    assert!(
        !is_postprocess_instance_present(
            &inst_chen,
            &Id::from_str("inst-chen"),
            &present,
            &HashSet::new()
        ),
        "Chen 不在场，写回应被拒"
    );
    // Lin 按 name 在场 ⇒ 通过（无同名冲突时 name 路有效）
    assert!(
        is_postprocess_instance_present(
            &inst_lin,
            &Id::from_str("inst-lin"),
            &present,
            &HashSet::new()
        ),
        "Lin 按 name 在场，写回应通过"
    );
    // present 改为 inst.id 形式 ⇒ 也通过（id 路匹配）
    let mut present_by_id: HashSet<String> = HashSet::new();
    present_by_id.insert("inst-lin".into());
    assert!(
        is_postprocess_instance_present(
            &inst_lin,
            &Id::from_str("inst-lin"),
            &present_by_id,
            &HashSet::new()
        ),
        "present 含 inst.id 时，id 路匹配应通过"
    );
}

/// P4 同名收紧：若两个 instance 同名且 name 在 name_collisions 中，name 路失效。
/// 只有 id 在 present 中的那个 instance 通过。
#[test]
fn b4_name_collision_only_id_path_works() {
    let inst_a = CharacterInstance {
        id: Id::from_str("inst-a"),
        campaign_id: Id::from_str("camp-1"),
        definition_id: None,
        name: "Dup".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    let inst_b = CharacterInstance {
        id: Id::from_str("inst-b"),
        campaign_id: Id::from_str("camp-1"),
        definition_id: None,
        name: "Dup".into(), // 同名
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    let mut present: HashSet<String> = HashSet::new();
    present.insert("Dup".into());
    // name_collisions 包含 "Dup"——同名时 name 路失效
    let name_collisions = HashSet::from([String::from("Dup")]);

    // 两个同名 instance 都因 name 路失效被拒（id 不在 present 中）
    assert!(
        !is_postprocess_instance_present(
            &inst_a,
            &Id::from_str("inst-a"),
            &present,
            &name_collisions
        ),
        "同名时 name 路失效，inst-a 的 id 不在 present 中应被拒"
    );
    assert!(
        !is_postprocess_instance_present(
            &inst_b,
            &Id::from_str("inst-b"),
            &present,
            &name_collisions
        ),
        "同名时 name 路失效，inst-b 的 id 不在 present 中应被拒"
    );
}

/// P4 同名场景：present 含 inst_a 的 id → inst_a 过、inst_b 拒。
#[test]
fn b4_name_collision_id_path_still_works() {
    let inst_a = CharacterInstance {
        id: Id::from_str("inst-a"),
        campaign_id: Id::from_str("camp-1"),
        definition_id: None,
        name: "Dup".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    let inst_b = CharacterInstance {
        id: Id::from_str("inst-b"),
        campaign_id: Id::from_str("camp-1"),
        definition_id: None,
        name: "Dup".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    // present 含 inst-a 的 id（不是 name）
    let present = HashSet::from([String::from("inst-a")]);
    let name_collisions = HashSet::from([String::from("Dup")]);

    // inst-a 的 id 在 present 中 → 通过（id 路不受 name_collisions 影响）
    assert!(
        is_postprocess_instance_present(
            &inst_a,
            &Id::from_str("inst-a"),
            &present,
            &name_collisions
        ),
        "inst-a 的 id 在 present 中，id 路应通过"
    );
    // inst-b 的 id 不在 present 中 → 拒（name 路因 name_collisions 失效）
    assert!(
        !is_postprocess_instance_present(
            &inst_b,
            &Id::from_str("inst-b"),
            &present,
            &name_collisions
        ),
        "inst-b 的 id 不在 present 中，同名时 name 路失效应被拒"
    );
}

/// P8 钉：subagent 的 tool_whitelist 加未注册工具名（如 search_world_info），
/// 不应生效——subagent registry 只注册了 get_character，whitelist 无法添加新工具。
/// dispatch 未注册/被移除的工具返回 NotFound，不可绕过。
#[tokio::test]
async fn b8_subagent_whitelist_cannot_add_unregistered_tool() {
    use storyforge_app_agent::filter_registry_by_whitelist;

    let mut registry = ToolRegistry::new();
    register_subagent_tools(&mut registry);
    // 基线：subagent 只有 get_character
    assert_eq!(
        registry.tool_specs().len(),
        1,
        "subagent 应只注册 get_character"
    );

    // whitelist 尝试加 search_world_info（Director 工具，subagent 未注册）
    let whitelist: Option<&[String]> =
        Some(&["search_world_info".to_string(), "get_character".to_string()]);
    filter_registry_by_whitelist(&mut registry, whitelist, "subagent");

    // 结果：get_character 保留，search_world_info 因未注册被忽略
    assert_eq!(
        registry.tool_specs().len(),
        1,
        "whitelist 不能添加未注册工具，应仍只有 get_character"
    );

    // dispatch 未注册的 search_world_info → NotFound
    let ctx = Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
        campaign_runtime: None,
        current_character_instance_id: None,
        regex_scripts: vec![],
    });
    let r = registry
        .dispatch("search_world_info", serde_json::json!({}), ctx)
        .await;
    assert!(
        matches!(r, Err(ToolError::NotFound(_))),
        "未注册工具 dispatch 应 NotFound，不可绕过 whitelist"
    );
}

use std::sync::Arc;
