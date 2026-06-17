/// Agent 工具注册与实现
///
/// 对应设计 §3.2 的 Agent 工具集。M1 实现：
/// - search_world_info: 关键词搜索世界书（非向量，简单匹配）
/// - get_character: 获取角色卡详情
/// - emit_plan: 导演输出 Plan
/// - compose: 编剧输出成文（直接返回内容，不做工具调用）
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use storyforge_domain::Id;
use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::character::Character;
use storyforge_domain::llm::ToolSpec;
use storyforge_domain::world_info::WorldInfoBook;

/// 工具执行错误
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("工具不存在: {0}")]
    NotFound(String),

    #[error("参数错误: {0}")]
    BadArgs(String),

    #[error("内部错误: {0}")]
    Internal(String),
}

/// 工具上下文（提供给工具函数的数据源）
#[derive(Clone)]
pub struct ToolContext {
    /// 可用的角色卡
    pub characters: Vec<Arc<Character>>,
    /// 可用的世界书
    pub world_info: Option<Arc<WorldInfoBook>>,
    /// 向量存储（search_vectors 工具用）
    pub vector_store: Option<Arc<dyn storyforge_infra_vector::VectorStore>>,
    /// 已归档的远记忆摘要（get_recent_summary 工具用）
    pub archived_summaries: Vec<String>,
    /// Campaign 运行时快照（阶段 2 新增）。None = 未开 Campaign，走旧路径。
    pub campaign_runtime: Option<Arc<CampaignRuntimeContext>>,
    /// 当前子 Agent 绑定的 instance id（阶段 4 新增）。
    /// 用于子 Agent get_character 工具：只返回自己的 instance 数据，不泄露其他角色。
    /// 导演/编剧/无 Campaign 时为 None。
    pub current_character_instance_id: Option<Id>,
}

/// 工具处理器（异步函数 trait）
pub type ToolHandler = dyn Fn(
        Value,
        Arc<ToolContext>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, ToolError>> + Send>>
    + Send
    + Sync;

/// 工具注册表
pub struct ToolRegistry {
    tools: HashMap<String, (Arc<ToolHandler>, ToolSpec)>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// 注册工具
    pub fn register(
        &mut self,
        spec: ToolSpec,
        handler: impl Fn(
            Value,
            Arc<ToolContext>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, ToolError>> + Send>,
        > + Send
        + Sync
        + 'static,
    ) {
        let name = spec.function.name.clone();
        self.tools.insert(name, (Arc::new(handler), spec));
    }

    /// 获取所有工具定义（发给 LLM 的 tools 数组）
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|(_, spec)| spec.clone()).collect()
    }

    /// 执行工具调用
    pub async fn dispatch(
        &self,
        name: &str,
        args: Value,
        ctx: Arc<ToolContext>,
    ) -> Result<Value, ToolError> {
        let (handler, _) = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::NotFound(name.to_string()))?;

        handler(args, ctx).await
    }

    /// 检查工具是否存在
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// 按白名单保留已注册的工具。
    ///
    /// 语义（与 `AgentRunConfig::tool_whitelist` 一致）：
    /// - `None`：不改动（使用当前已注册的全部默认工具）。
    /// - `Some([])`：清空所有工具（禁用全部工具）。
    /// - `Some(names)`：只保留 `names` 中存在的工具；`names` 里未注册的名称不 panic，
    ///   由调用方决定是否记录 warning（见 `filter_registry_by_whitelist`）。
    ///
    /// 过滤后 `tool_specs()`（发给 LLM 的 tools 数组）和 `dispatch` 同步收窄，
    /// 被禁用的工具若被调用会返回 `ToolError::NotFound`，不会绕过 whitelist。
    pub fn retain(&mut self, whitelist: Option<&[String]>) {
        match whitelist {
            None => {}
            Some(names) => {
                self.tools.retain(|k, _| names.iter().any(|n| n == k));
            }
        }
    }
}

/// 按 `AgentRunConfig::tool_whitelist` 过滤 registry。
///
/// - `None`：不动（默认工具集）。
/// - `Some([])`：清空全部工具。
/// - `Some(list)`：只保留 list 中已注册的工具；list 里未注册的名称记 warning 后忽略，
///   不 panic（满足「未知工具名不能 panic」的要求）。
///
/// `role_label` 仅用于日志，便于排查是哪个角色配了不存在的工具。
pub fn filter_registry_by_whitelist(
    registry: &mut ToolRegistry,
    whitelist: Option<&[String]>,
    role_label: &str,
) {
    if let Some(names) = whitelist {
        // 先校验：列出白名单里不存在于 registry 的名称，记 warning
        for n in names {
            if !registry.has(n) {
                tracing::warn!(
                    target: "app-agent",
                    "{role_label}: tool_whitelist 引用了未注册的工具 '{n}'，忽略"
                );
            }
        }
        registry.retain(Some(names));
    }
}

// ─── 预置工具实现 ──────────────────────────────────────────────────────────

/// 注册导演 Agent 的工具
pub fn register_director_tools(registry: &mut ToolRegistry) {
    // search_world_info: 关键词搜索世界书
    registry.register(
        ToolSpec::function(
            "search_world_info",
            "按关键词搜索世界书条目。返回匹配的条目内容。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"}
                },
                "required": ["query"]
            }),
        ),
        |args, ctx| {
            Box::pin(async move {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::BadArgs("缺少 query 参数".into()))?;

                let results = if let Some(book) = &ctx.world_info {
                    book.search_by_keywords(query)
                        .into_iter()
                        .map(|e| {
                            serde_json::json!({
                                "keys": e.keys,
                                "content": e.content,
                                "route": format!("{:?}", e.route),
                            })
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                };

                Ok(serde_json::json!({
                    "query": query,
                    "results_count": results.len(),
                    "results": results,
                }))
            })
        },
    );

    // get_character: 获取角色卡详情
    // 阶段 3：有 campaign_runtime 时优先查 Campaign 实例（含 definition/persona/behavior）
    // 无 campaign_runtime 或查不到实例时，退回旧的扁平 Character 逻辑
    registry.register(
        ToolSpec::function(
            "get_character",
            "获取指定角色的详细信息。可传角色名或 instance_id。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "角色名称或 instance_id"}
                },
                "required": ["name"]
            }),
        ),
        |args, ctx| {
            Box::pin(async move {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::BadArgs("缺少 name 参数".into()))?;

                // 阶段 3：优先从 campaign_runtime 查实例
                if let Some(runtime) = &ctx.campaign_runtime {
                    if let Some(inst) = runtime.find_instance_by_id_or_name(name) {
                        let def = runtime.definition_for_instance(inst);
                        let persona = runtime.resolved_persona_for(inst);
                        let behavior = runtime.resolved_behavior_for(inst);
                        let role_type = def.map(|d| format!("{:?}", d.role_type));
                        let backstory = def.map(|d| d.base_backstory.clone());

                        return Ok(serde_json::json!({
                            "id": inst.id.as_str(),
                            "instance_id": inst.id.as_str(),
                            "name": inst.name,
                            "definition_id": inst.definition_id.as_ref().map(|id| id.as_str()),
                            "role_type": role_type,
                            "persona": persona,
                            "behavior": behavior,
                            "backstory": backstory,
                            "variables": inst.variables,
                            "is_temporary": inst.is_temporary,
                            "source": "campaign_instance",
                        }));
                    }
                }

                // fallback：旧的扁平 Character 逻辑
                let character = ctx
                    .characters
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(name))
                    .ok_or_else(|| ToolError::NotFound(format!("角色 '{name}' 不存在")))?;

                Ok(serde_json::json!({
                    "name": character.name,
                    "description": character.description,
                    "personality": character.personality,
                    "scenario": character.scenario,
                    "first_mes": character.first_mes,
                    "system_prompt": character.system_prompt,
                    "source": "flat_character",
                }))
            })
        },
    );

    // emit_plan: 导演输出 Plan（解析 JSON 参数为 Plan 结构）
    registry.register(
        ToolSpec::function(
            "emit_plan",
            "输出结构化的写作计划。包含场景简述和每个子 Agent 的任务。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "scene_brief": {"type": "string", "description": "场景简述"},
                    "subagent_tasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "character_id": {"type": "string"},
                                "brief": {"type": "string"},
                            }
                        }
                    }
                },
                "required": ["scene_brief", "subagent_tasks"]
            }),
        ),
        |args, _ctx| {
            Box::pin(async move {
                // 直接返回 args 作为 Plan 的 JSON（上层解析）
                Ok(args)
            })
        },
    );

    // search_vectors: 向量记忆搜索（M2 新增）
    registry.register(
        ToolSpec::function(
            "search_vectors",
            "按语义相似度搜索远记忆、世界书和角色设定。返回最相关的结果。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索查询（自然语言）"},
                    "top_k": {"type": "integer", "description": "返回结果数量（默认 5）"}
                },
                "required": ["query"]
            }),
        ),
        |args, ctx| {
            Box::pin(async move {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::BadArgs("缺少 query 参数".into()))?;
                let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

                let store = ctx
                    .vector_store
                    .as_ref()
                    .ok_or_else(|| ToolError::Internal("向量存储未配置".into()))?;

                // 关键词搜索（M2 阶段先用关键词，后续接嵌入向量搜索）
                let keywords: Vec<String> = query
                    .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
                    .filter(|w| w.len() >= 2)
                    .map(String::from)
                    .collect();

                let hits = store
                    .search_by_keywords(&keywords, top_k)
                    .map_err(|e| ToolError::Internal(format!("向量搜索失败: {e}")))?;

                let results: Vec<serde_json::Value> = hits
                    .into_iter()
                    .map(|h| {
                        serde_json::json!({
                            "content": h.content,
                            "score": h.score,
                            "kind": h.kind,
                            "keywords": h.keywords,
                        })
                    })
                    .collect();

                Ok(serde_json::json!({
                    "query": query,
                    "results_count": results.len(),
                    "results": results,
                }))
            })
        },
    );

    // get_recent_summary: 获取远记忆摘要（M2 新增）
    registry.register(
        ToolSpec::function(
            "get_recent_summary",
            "获取已归档的远记忆摘要列表。用于了解之前发生的重要事件。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "返回摘要数量（默认 3）"}
                }
            }),
        ),
        |args, ctx| {
            Box::pin(async move {
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

                let summaries: Vec<&str> = ctx
                    .archived_summaries
                    .iter()
                    .rev()
                    .take(limit)
                    .map(|s| s.as_str())
                    .collect();

                Ok(serde_json::json!({
                    "summaries_count": summaries.len(),
                    "summaries": summaries,
                }))
            })
        },
    );
}

/// 注册子 Agent 的工具（只读，受限）
///
/// 阶段 4 改造：有 `current_character_instance_id` 时，get_character 只返回
/// 当前子 Agent 绑定的 instance 数据（信息隔离），不泄露其他角色。
pub fn register_subagent_tools(registry: &mut ToolRegistry) {
    // get_character: 子 Agent 只能查自己绑定的 instance
    registry.register(
        ToolSpec::function(
            "get_character",
            "获取当前角色的详细信息。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "角色名称"}
                },
                "required": ["name"]
            }),
        ),
        |args, ctx| {
            Box::pin(async move {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::BadArgs("缺少 name 参数".into()))?;

                // 阶段 4：如果有 current_character_instance_id，只返回该 instance 的数据
                if let Some(ref instance_id) = ctx.current_character_instance_id {
                    if let Some(ref runtime) = ctx.campaign_runtime {
                        // 只允许查自己的 instance
                        if let Some(inst) = runtime.instances.iter().find(|i| &i.id == instance_id) {
                            let def = runtime.definition_for_instance(inst);
                            let persona = runtime.resolved_persona_for(inst);
                            let behavior = runtime.resolved_behavior_for(inst);
                            let role_type = def.map(|d| format!("{:?}", d.role_type));
                            let backstory = def.map(|d| d.base_backstory.clone());

                            // 验证 name 参数匹配（允许传自己的名字或 id）
                            if inst.name.eq_ignore_ascii_case(name) || inst.id.as_str() == name {
                                return Ok(serde_json::json!({
                                    "id": inst.id.as_str(),
                                    "instance_id": inst.id.as_str(),
                                    "name": inst.name,
                                    "definition_id": inst.definition_id.as_ref().map(|id| id.as_str()),
                                    "role_type": role_type,
                                    "persona": persona,
                                    "behavior": behavior,
                                    "backstory": backstory,
                                    "variables": inst.variables,
                                    "is_temporary": inst.is_temporary,
                                    "source": "campaign_instance",
                                }));
                            } else {
                                return Err(ToolError::NotFound(
                                    format!("子 Agent 只能查询自己的角色信息，不能查询 '{name}'")
                                ));
                            }
                        }
                    }
                }

                // fallback：旧的扁平 Character 逻辑（无 Campaign 时）
                let character = ctx
                    .characters
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(name))
                    .ok_or_else(|| ToolError::NotFound(format!("角色 '{name}' 不存在")))?;

                Ok(serde_json::json!({
                    "name": character.name,
                    "description": character.description,
                    "personality": character.personality,
                }))
            })
        },
    );
}

/// 注册编剧 Agent 的工具
#[allow(dead_code)]
pub fn register_editor_tools(registry: &mut ToolRegistry) {
    // compose: 编剧输出成文（实际上编剧直接输出文本，不需要真正调工具）
    // 这个工具是为了让编剧可以声明"我完成了"
    registry.register(
        ToolSpec::function(
            "compose",
            "输出最终成文。调用此工具表示合并完成。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "最终成文（Markdown）"}
                },
                "required": ["text"]
            }),
        ),
        |args, _ctx| Box::pin(async move { Ok(args) }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::Id;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::variables::default_character_variables;

    fn make_flat_character(name: &str) -> Arc<Character> {
        Arc::new(Character {
            id: Id::from_str(name),
            name: name.into(),
            description: format!("{name} desc"),
            personality: format!("{name} personality"),
            scenario: String::new(),
            first_mes: String::new(),
            mes_example: String::new(),
            system_prompt: String::new(),
            post_history_instructions: String::new(),
            tags: vec![],
            creator: "test".into(),
            character_version: "1.0".into(),
            alternate_greetings: vec![],
            embedded_world_info: None,
            extensions: serde_json::json!({}),
            renderable_assets: None,
            source: storyforge_domain::Source::Native,
            spec_version: "3.0".into(),
            raw_card_json: serde_json::json!({}),
        })
    }

    fn make_campaign_runtime_with_lin() -> Arc<CampaignRuntimeContext> {
        let campaign = Campaign::new(Id::from_str("card-1"), "test-campaign");
        let def = CharacterDefinition {
            id: Id::from_str("def-lin"),
            card_id: Id::from_str("card-1"),
            name: "Lin".into(),
            persona_prompt: "calm surgeon".into(),
            behavior_rules: "save first".into(),
            base_backstory: vec!["is a surgeon".into()],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        let instance = CharacterInstance {
            id: Id::from_str("inst-lin"),
            campaign_id: campaign.id.clone(),
            definition_id: Some(def.id.clone()),
            name: "Lin".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        };
        let mut definitions_by_id = std::collections::HashMap::new();
        definitions_by_id.insert(def.id.clone(), def);

        Arc::new(CampaignRuntimeContext {
            campaign,
            instances: vec![instance],
            definitions_by_id,
            knowledge: vec![],
            turn: 1,
        })
    }

    /// 阶段 3：有 campaign_runtime 时，get_character 返回 instance 数据
    #[tokio::test]
    async fn test_get_character_returns_campaign_instance() {
        let runtime = make_campaign_runtime_with_lin();
        let ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: Some(runtime),
            current_character_instance_id: None,
        });

        let mut registry = ToolRegistry::new();
        register_director_tools(&mut registry);

        let result = registry
            .dispatch("get_character", serde_json::json!({"name": "Lin"}), ctx)
            .await
            .unwrap();

        assert_eq!(result["source"], "campaign_instance");
        assert_eq!(result["name"], "Lin");
        assert_eq!(result["instance_id"], "inst-lin");
        assert_eq!(result["definition_id"], "def-lin");
        assert_eq!(result["persona"], "calm surgeon");
        assert_eq!(result["behavior"], "save first");
        assert_eq!(result["is_temporary"], false);
        assert!(
            result["role_type"]
                .as_str()
                .unwrap()
                .contains("Protagonist")
        );
    }

    /// 阶段 3：有 campaign_runtime 但查不到实例时，fallback 到扁平 Character
    #[tokio::test]
    async fn test_get_character_fallback_to_flat_when_not_in_campaign() {
        let runtime = make_campaign_runtime_with_lin();
        let ctx = Arc::new(ToolContext {
            characters: vec![make_flat_character("Seraphina")],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: Some(runtime),
            current_character_instance_id: None,
        });

        let mut registry = ToolRegistry::new();
        register_director_tools(&mut registry);

        let result = registry
            .dispatch(
                "get_character",
                serde_json::json!({"name": "Seraphina"}),
                ctx,
            )
            .await
            .unwrap();

        assert_eq!(result["source"], "flat_character");
        assert_eq!(result["name"], "Seraphina");
    }

    /// 阶段 3：无 campaign_runtime 时，get_character 走旧的扁平 Character 逻辑
    #[tokio::test]
    async fn test_get_character_uses_flat_character_when_no_runtime() {
        let ctx = Arc::new(ToolContext {
            characters: vec![make_flat_character("Seraphina")],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
        });

        let mut registry = ToolRegistry::new();
        register_director_tools(&mut registry);

        let result = registry
            .dispatch(
                "get_character",
                serde_json::json!({"name": "Seraphina"}),
                ctx,
            )
            .await
            .unwrap();

        assert_eq!(result["source"], "flat_character");
        assert_eq!(result["name"], "Seraphina");
        assert_eq!(result["personality"], "Seraphina personality");
    }

    /// 阶段 3：campaign_runtime 存在但实例和扁平角色都没有 → NotFound
    #[tokio::test]
    async fn test_get_character_not_found_when_no_match() {
        let runtime = make_campaign_runtime_with_lin();
        let ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: Some(runtime),
            current_character_instance_id: None,
        });

        let mut registry = ToolRegistry::new();
        register_director_tools(&mut registry);

        let result = registry
            .dispatch("get_character", serde_json::json!({"name": "Ghost"}), ctx)
            .await;

        assert!(result.is_err(), "不存在的角色应返回错误");
    }

    // ── 阶段 4：子 Agent get_character 信息隔离测试 ──

    /// 阶段 4：子 Agent 有 current_character_instance_id 时，只能查自己的 instance
    #[tokio::test]
    async fn test_subagent_get_character_only_returns_own_instance() {
        let runtime = make_campaign_runtime_with_lin();
        // 添加第二个 instance
        let mut cr = (*runtime).clone();
        let campaign = Campaign::new(Id::from_str("card-1"), "test-campaign");
        let def_chen = CharacterDefinition {
            id: Id::from_str("def-chen"),
            card_id: Id::from_str("card-1"),
            name: "Chen".into(),
            persona_prompt: "strict cop".into(),
            behavior_rules: "follow rules".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        let inst_chen = CharacterInstance {
            id: Id::from_str("inst-chen"),
            campaign_id: campaign.id.clone(),
            definition_id: Some(def_chen.id.clone()),
            name: "Chen".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        };
        cr.instances.push(inst_chen);
        cr.definitions_by_id.insert(def_chen.id.clone(), def_chen);
        let cr = Arc::new(cr);

        // 子 Agent 绑定到 inst-lin
        let ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: Some(cr),
            current_character_instance_id: Some(Id::from_str("inst-lin")),
        });

        let mut registry = ToolRegistry::new();
        register_subagent_tools(&mut registry);

        // 查自己的名字 → 成功
        let result = registry
            .dispatch(
                "get_character",
                serde_json::json!({"name": "Lin"}),
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(result["source"], "campaign_instance");
        assert_eq!(result["name"], "Lin");

        // 查别人的名字 → 错误（信息隔离）
        let result = registry
            .dispatch("get_character", serde_json::json!({"name": "Chen"}), ctx)
            .await;
        assert!(result.is_err(), "子 Agent 不应能查其他角色");
    }

    /// 阶段 4：子 Agent 无 current_character_instance_id 时退回旧路径
    #[tokio::test]
    async fn test_subagent_get_character_fallback_when_no_instance_id() {
        let ctx = Arc::new(ToolContext {
            characters: vec![make_flat_character("Seraphina")],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            campaign_runtime: None,
            current_character_instance_id: None,
        });

        let mut registry = ToolRegistry::new();
        register_subagent_tools(&mut registry);

        let result = registry
            .dispatch(
                "get_character",
                serde_json::json!({"name": "Seraphina"}),
                ctx,
            )
            .await
            .unwrap();
        assert_eq!(result["name"], "Seraphina");
    }

    // ── tool_whitelist：ToolRegistry::retain / filter_registry_by_whitelist ──

    fn registry_with_director_tools() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        register_director_tools(&mut registry);
        registry
    }

    /// None = 不动，保留所有默认工具
    #[test]
    fn retain_none_keeps_all_tools() {
        let registry = registry_with_director_tools();
        let before = registry.tool_specs().len();
        let mut r = registry;
        r.retain(None);
        assert_eq!(r.tool_specs().len(), before, "None 不应改动工具集");
    }

    /// Some([]) = 清空全部工具
    #[tokio::test]
    async fn retain_empty_vec_clears_all_tools() {
        let mut registry = registry_with_director_tools();
        registry.retain(Some(&[]));
        assert!(registry.tool_specs().is_empty(), "Some([]) 应清空所有工具");
        // dispatch 被禁用的工具应返回 NotFound，不绕过 whitelist
        let res = registry
            .dispatch(
                "get_character",
                serde_json::json!({}),
                Arc::new(ToolContext {
                    characters: vec![],
                    world_info: None,
                    vector_store: None,
                    archived_summaries: vec![],
                    campaign_runtime: None,
                    current_character_instance_id: None,
                }),
            )
            .await;
        assert!(
            matches!(res, Err(ToolError::NotFound(_))),
            "被禁用的工具 dispatch 应返回 NotFound，实际: {res:?}"
        );
    }

    /// Some(list) = 只保留列表中的工具
    #[test]
    fn retain_partial_list_keeps_only_listed() {
        let mut registry = registry_with_director_tools();
        let whitelist = vec!["get_character".to_string(), "emit_plan".to_string()];
        registry.retain(Some(&whitelist));
        let names: Vec<String> = registry
            .tool_specs()
            .iter()
            .map(|s| s.function.name.clone())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"get_character".to_string()));
        assert!(names.contains(&"emit_plan".to_string()));
        // 不在白名单的工具已被移除
        assert!(!registry.has("search_world_info"));
    }

    /// 白名单里的未知工具名不能 panic，被忽略
    #[test]
    fn filter_unknown_tool_name_does_not_panic() {
        let mut registry = registry_with_director_tools();
        let whitelist = vec![
            "get_character".to_string(),
            "this_tool_does_not_exist".to_string(),
        ];
        // 不应 panic
        filter_registry_by_whitelist(&mut registry, Some(&whitelist), "test-role");
        // 已注册的保留，未注册的被忽略
        assert!(registry.has("get_character"));
        assert_eq!(registry.tool_specs().len(), 1);
    }

    /// filter_registry_by_whitelist(None) 不改动
    #[test]
    fn filter_none_is_noop() {
        let registry = registry_with_director_tools();
        let before = registry.tool_specs().len();
        let mut r = registry;
        filter_registry_by_whitelist(&mut r, None, "test-role");
        assert_eq!(r.tool_specs().len(), before);
    }
}
