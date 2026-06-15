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
}

/// 工具处理器（异步函数 trait）
pub type ToolHandler = dyn Fn(Value, Arc<ToolContext>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, ToolError>> + Send>>
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
        handler: impl Fn(Value, Arc<ToolContext>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, ToolError>> + Send>>
            + Send
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
    registry.register(
        ToolSpec::function(
            "get_character",
            "获取指定角色卡的详细信息（名称、描述、性格、场景等）。",
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
                let top_k = args
                    .get("top_k")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5) as usize;

                let store = ctx.vector_store.as_ref().ok_or_else(|| {
                    ToolError::Internal("向量存储未配置".into())
                })?;

                // 关键词搜索（M2 阶段先用关键词，后续接嵌入向量搜索）
                let keywords: Vec<String> = query
                    .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
                    .filter(|w| w.len() >= 2)
                    .map(String::from)
                    .collect();

                let hits = store.search_by_keywords(&keywords, top_k).map_err(|e| {
                    ToolError::Internal(format!("向量搜索失败: {e}"))
                })?;

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
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3) as usize;

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
pub fn register_subagent_tools(registry: &mut ToolRegistry) {
    // get_character: 子 Agent 只能查自己（上层通过 ContextPackage 控制）
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
        |args, _ctx| {
            Box::pin(async move {
                Ok(args)
            })
        },
    );
}
