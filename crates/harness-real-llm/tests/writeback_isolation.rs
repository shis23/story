//! 写回隔离 + 工具白名单钉测（确定性，不需 LLM）。
//!
//! 钉 postprocess 落盘的隔离判定 `is_postprocess_instance_present`（线上同款纯函数），
//! 覆盖审计标的 P3/P4/P6/P8。这些是"清晰泄漏立即修"范围外的设计取舍/边界，
//! 按计划只钉测 + 报告，不改业务行为（P0 已修，B0 在 app-agent 钉过）。

use std::collections::HashSet;

use storyforge_app_agent::tools::{register_subagent_tools, ToolRegistry};
use storyforge_app_agent::ToolContext;
use storyforge_app_agent::ToolError;
use storyforge_domain::campaign::CharacterInstance;
use storyforge_domain::Id;
use storyforge_tauri_app::is_postprocess_instance_present;

/// P3 钉当前行为：present_chars 空集时，所有 instance 的写回都通过（向后兼容逃生口）。
/// 这意味着 postprocess 在无 present_chars 约束时，可写任意角色知识/变量。
/// 报告为"待定收紧"——若要拒绝，需改 is_postprocess_instance_present 的空集分支。
#[test]
fn b3_empty_present_chars_escape_hatch_current_behavior() {
    let inst = CharacterInstance::temporary(Id::from_str("camp-1"), "缺席角色");
    let raw_id = Id::from_str("inst-absent");
    let empty: HashSet<String> = HashSet::new();

    // 当前行为：空集 ⇒ 全过（逃生口）
    let passes = is_postprocess_instance_present(&inst, &raw_id, &empty);
    assert!(
        passes,
        "当前行为：present_chars 空集时所有 instance 写回通过（向后兼容逃生口）"
    );
    // 记录：这是审计标的 P3，待用户拍板是否收紧为"空集也拒绝"
}

/// P3 期望行为（#[ignore]，待收紧后翻转）：空集时应拒绝非显式在场的角色。
/// 当前 fail（因为逃生口放行）——收紧 is_postprocess_instance_present 后应改为 pass。
#[test]
#[ignore = "P3 待定收紧：空集逃生口当前放行，收紧后翻转此断言"]
fn b3_empty_present_chars_should_reject_when_tightened() {
    let inst = CharacterInstance::temporary(Id::from_str("camp-1"), "缺席角色");
    let raw_id = Id::from_str("inst-absent");
    let empty: HashSet<String> = HashSet::new();
    let passes = is_postprocess_instance_present(&inst, &raw_id, &empty);
    assert!(
        !passes,
        "期望（收紧后）：空集时不应放行未显式列出的角色写回"
    );
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
        !is_postprocess_instance_present(&inst_chen, &Id::from_str("inst-chen"), &present),
        "Chen 不在场，写回应被拒"
    );
    // Lin 按 name 在场 ⇒ 通过
    assert!(
        is_postprocess_instance_present(&inst_lin, &Id::from_str("inst-lin"), &present),
        "Lin 按 name 在场，写回应通过"
    );
    // present 改为 inst.id 形式 ⇒ 也通过（id 路匹配）
    let mut present_by_id: HashSet<String> = HashSet::new();
    present_by_id.insert("inst-lin".into());
    assert!(
        is_postprocess_instance_present(&inst_lin, &Id::from_str("inst-lin"), &present_by_id),
        "present 含 inst.id 时，id 路匹配应通过"
    );
}

/// P4 别名风险钉（记录行为）：若两个 instance 同名，present 含该 name 时两者都通过。
/// 这是 name 匹配的固有歧义——记录为已知行为，收紧建议见 findings。
#[test]
fn b4_name_collision_both_pass() {
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
    // 两个同名 instance 都因 name 匹配通过——歧义行为
    assert!(
        is_postprocess_instance_present(&inst_a, &Id::from_str("inst-a"), &present),
        "同名 instance A 因 name 匹配通过（已知歧义）"
    );
    assert!(
        is_postprocess_instance_present(&inst_b, &Id::from_str("inst-b"), &present),
        "同名 instance B 也通过——name 匹配无法区分（记录为待收紧）"
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
    let whitelist: Option<&[String]> = Some(&[
        "search_world_info".to_string(),
        "get_character".to_string(),
    ]);
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
