//! 知识边界探针 — 确定性部分（不需 LLM，纯函数 + 工具 dispatch 断言）。
//!
//! 这是用户核心目标「验证各 agent 知识边界」的读侧基线：用合成 2 角色 campaign
//! （各自私有知识）断言隔离的硬保证，与真实 LLM 探针（i1_real_llm.rs）互补——
//! 这里证 wiring 正确，那里证 LLM 在对抗性诱导下也攻不破。
//!
//! 覆盖：
//! - I2 Director 全可见（正对照）：get_character 可取任意 instance。
//! - volatile tail 隔离：build_campaign_subagent_volatile 只注入本 instance 知识/变量。
//! - get_character 越权：子 agent 查别的 instance 名/id → NotFound（已在 app-agent 钉过，
//!   这里在 harness 语境再钉一次，并覆盖 instance_id-as-name 变体）。
//! - 临时 instance 隔离：temp instance 子 agent 看不到常驻 instance。

use std::sync::Arc;

use storyforge_app_agent::ToolContext;
use storyforge_app_agent::ToolError;
use storyforge_app_agent::runtime::build_campaign_subagent_volatile;
use storyforge_app_agent::tools::{ToolRegistry, register_subagent_tools};
use storyforge_domain::Id;
use storyforge_domain::agent::{ContextPackage, SubagentTask};
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::character::{CharacterDefinition, RoleType};
use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
use storyforge_domain::variables::default_character_variables;

/// 构造合成 2 角色 campaign runtime：Lin（主角）+ Chen（主角），各有私有知识。
/// Lin 知道 "Lin 的秘密 A"，Chen 知道 "Chen 的秘密 B"——互不应见。
fn make_two_char_runtime() -> (
    Arc<CampaignRuntimeContext>,
    CharacterInstance,
    CharacterInstance,
) {
    let campaign = Campaign::new(Id::from_str("card-1"), "isolation-campaign");
    let campaign_id = campaign.id.clone();

    let def_lin = CharacterDefinition {
        id: Id::from_str("def-lin"),
        card_id: Id::from_str("card-1"),
        name: "Lin".into(),
        persona_prompt: "冷静的外科医生".into(),
        behavior_rules: "救人优先".into(),
        base_backstory: vec!["是名外科医生".into()],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: default_character_variables(),
    };
    let def_chen = CharacterDefinition {
        id: Id::from_str("def-chen"),
        card_id: Id::from_str("card-1"),
        name: "Chen".into(),
        persona_prompt: "严厉的警察".into(),
        behavior_rules: "遵守规则".into(),
        base_backstory: vec!["是名警察".into()],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: default_character_variables(),
    };

    let inst_lin = CharacterInstance {
        id: Id::from_str("inst-lin"),
        campaign_id: campaign_id.clone(),
        definition_id: Some(def_lin.id.clone()),
        name: "Lin".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    let inst_chen = CharacterInstance {
        id: Id::from_str("inst-chen"),
        campaign_id: campaign_id.clone(),
        definition_id: Some(def_chen.id.clone()),
        name: "Chen".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };

    // 各自私有知识
    let knowledge = vec![
        CharacterKnowledgeEntry {
            id: Id::from_str("k-lin-1"),
            campaign_id: campaign_id.clone(),
            character_id: inst_lin.id.clone(),
            knowledge_text: "Lin 的秘密 A（只有 Lin 知道）".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            source_knowledge_id: None,
            turn_number: 1,
            event_id: None,
            pinned: false,
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        },
        CharacterKnowledgeEntry {
            id: Id::from_str("k-chen-1"),
            campaign_id: campaign_id.clone(),
            character_id: inst_chen.id.clone(),
            knowledge_text: "Chen 的秘密 B（只有 Chen 知道）".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            source_knowledge_id: None,
            turn_number: 1,
            event_id: None,
            pinned: false,
            propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
        },
    ];

    let cr = CampaignRuntimeContext {
        campaign,
        instances: vec![inst_lin.clone(), inst_chen.clone()],
        definitions_by_id: [
            (def_lin.id.clone(), def_lin),
            (def_chen.id.clone(), def_chen),
        ]
        .into_iter()
        .collect(),
        knowledge,
        tasks: vec![],
        turn: 1,
    };
    (Arc::new(cr), inst_lin, inst_chen)
}

/// I2（正对照）：Director 的 get_character 不受 instance_id 绑定，可见所有 instance。
/// 注：Director 用 register_director_tools，这里用子 agent registry 的"无绑定"路径
/// 间接验证——子 agent 在 current_character_instance_id=None 时退回扁平路径，
/// 但 campaign 模式下 Director 是另一套注册器。本测试钉"无绑定 + 有 runtime"时
/// 不会硬失败（与 P0 修复后的"绑定但 unresolvable 才硬失败"对照）。
#[tokio::test]
async fn i2_unbound_subagent_does_not_hard_fail() {
    let (runtime, _inst_lin, _inst_chen) = make_two_char_runtime();
    // 无 instance_id 绑定（Director/Editor 语义）+ 有 runtime → 不应触发 P0 硬失败
    let ctx = Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
            chronicle_summaries: vec![],
        chronicle_tool_budget: std::sync::Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
        campaign_runtime: Some(runtime),
        current_character_instance_id: None,
        regex_scripts: vec![],
    });
    let mut registry = ToolRegistry::new();
    register_subagent_tools(&mut registry);
    // 查 Lin —— 无绑定时走扁平 characters（空）→ NotFound，但不是 P0 的硬失败信息
    let result = registry
        .dispatch("get_character", serde_json::json!({"name": "Lin"}), ctx)
        .await;
    assert!(result.is_err(), "无扁平角色时应 NotFound");
    let err = result.unwrap_err();
    assert!(
        matches!(err, ToolError::NotFound(ref m) if !m.contains("拒绝降级")),
        "无绑定不应触发 P0 硬失败（那是绑定-unresolvable 专属），实际: {err:?}"
    );
}

/// volatile tail 隔离：Lin 的 volatile 不含 Chen 的秘密，反之亦然。
#[test]
fn volatile_tail_knowledge_isolation_between_instances() {
    let (cr, inst_lin, inst_chen) = make_two_char_runtime();
    let task = SubagentTask {
        character_id: "Lin".into(),
        brief: "演一场".into(),
        context_package: ContextPackage {
            character_brief: String::new(),
            scene_brief: "医院走廊".into(),
            relevant_lore: vec![],
            constant_lore: vec![],
            recent_window: vec![],
            task: "出场".into(),
        },
    };

    let lin_tail = build_campaign_subagent_volatile(&task, &cr, &inst_lin);
    let chen_task = SubagentTask {
        character_id: "Chen".into(),
        ..task.clone()
    };
    let chen_tail = build_campaign_subagent_volatile(&chen_task, &cr, &inst_chen);

    // Lin 只见自己的秘密
    assert!(
        lin_tail.contains("Lin 的秘密 A"),
        "Lin 的 volatile 应含自己的知识"
    );
    assert!(
        !lin_tail.contains("Chen 的秘密 B"),
        "Lin 的 volatile 不应含 Chen 的知识（信息泄漏）"
    );
    // Chen 只见自己的秘密
    assert!(
        chen_tail.contains("Chen 的秘密 B"),
        "Chen 的 volatile 应含自己的知识"
    );
    assert!(
        !chen_tail.contains("Lin 的秘密 A"),
        "Chen 的 volatile 不应含 Lin 的知识（信息泄漏）"
    );
    // 场景是共享的，两者都应见
    assert!(lin_tail.contains("医院走廊") && chen_tail.contains("医院走廊"));
}

/// get_character 越权（harness 语境）：子 agent 绑定 Lin，查 Chen 的名字/instance_id → NotFound。
/// 覆盖 instance_id-as-name 变体（攻击者可能传对方的 instance_id 字符串）。
#[tokio::test]
async fn get_character_cross_instance_denied() {
    let (runtime, _inst_lin, _inst_chen) = make_two_char_runtime();
    let ctx = Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
            chronicle_summaries: vec![],
        chronicle_tool_budget: std::sync::Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
        campaign_runtime: Some(runtime),
        current_character_instance_id: Some(Id::from_str("inst-lin")),
        regex_scripts: vec![],
    });
    let mut registry = ToolRegistry::new();
    register_subagent_tools(&mut registry);

    // 查 Chen 的名字
    let r1 = registry
        .dispatch(
            "get_character",
            serde_json::json!({"name": "Chen"}),
            ctx.clone(),
        )
        .await;
    assert!(r1.is_err(), "Lin 子 agent 不应能查 Chen（按名）");

    // 查 Chen 的 instance_id 字符串
    let r2 = registry
        .dispatch(
            "get_character",
            serde_json::json!({"name": "inst-chen"}),
            ctx.clone(),
        )
        .await;
    assert!(r2.is_err(), "Lin 子 agent 不应能查 Chen（按 instance_id）");

    // 大小写变体
    let r3 = registry
        .dispatch("get_character", serde_json::json!({"name": "chen"}), ctx)
        .await;
    assert!(r3.is_err(), "Lin 子 agent 不应能查 chen（大小写变体）");

    // 正面：查自己
    let r_ok = registry
        .dispatch(
            "get_character",
            serde_json::json!({"name": "Lin"}),
            Arc::new(ToolContext {
                characters: vec![],
                world_info: None,
                vector_store: None,
                archived_summaries: vec![],
            chronicle_summaries: vec![],
                chronicle_tool_budget: std::sync::Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
                campaign_runtime: Some(make_two_char_runtime().0),
                current_character_instance_id: Some(Id::from_str("inst-lin")),
                regex_scripts: vec![],
            }),
        )
        .await
        .unwrap();
    assert_eq!(r_ok["source"], "campaign_instance");
    assert_eq!(r_ok["name"], "Lin");
}

/// 临时 instance 隔离：temp instance 绑定的子 agent 看不到常驻 instance 的知识。
#[tokio::test]
async fn temporary_instance_isolation() {
    let (cr, _inst_lin, _inst_chen) = make_two_char_runtime();
    let mut cr2 = (*cr).clone();
    // 加一个 temp instance
    let temp_inst = CharacterInstance::temporary(cr2.campaign.id.clone(), "路人甲");
    let temp_id = temp_inst.id.clone();
    cr2.instances.push(temp_inst.clone());
    let cr2 = Arc::new(cr2);

    // temp instance 绑定的子 agent
    let ctx = Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
            chronicle_summaries: vec![],
        chronicle_tool_budget: std::sync::Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
        campaign_runtime: Some(cr2.clone()),
        current_character_instance_id: Some(temp_id),
        regex_scripts: vec![],
    });
    let mut registry = ToolRegistry::new();
    register_subagent_tools(&mut registry);

    // temp 子 agent 查常驻 Lin → NotFound
    let r = registry
        .dispatch("get_character", serde_json::json!({"name": "Lin"}), ctx)
        .await;
    assert!(r.is_err(), "临时 instance 子 agent 不应能查常驻 Lin");

    // volatile tail：temp instance 无知识（空），不应含 Lin 的秘密
    let task = SubagentTask {
        character_id: "路人甲".into(),
        brief: "路过".into(),
        context_package: ContextPackage {
            character_brief: String::new(),
            scene_brief: "街角".into(),
            relevant_lore: vec![],
            constant_lore: vec![],
            recent_window: vec![],
            task: "出现".into(),
        },
    };
    let tail = build_campaign_subagent_volatile(&task, &cr2, &temp_inst);
    assert!(
        !tail.contains("Lin 的秘密 A"),
        "临时 instance 的 volatile 不应含常驻角色的私有知识"
    );
    assert!(
        !tail.contains("Chen 的秘密 B"),
        "临时 instance 的 volatile 不应含其他常驻角色的私有知识"
    );
}
