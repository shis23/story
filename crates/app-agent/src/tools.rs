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
use storyforge_domain::preset::{RegexPlacement, RegexScript};
use storyforge_domain::world_info::WorldInfoBook;
use storyforge_infra_regex::{RegexExecutionTarget, apply_regex_scripts_for_target_at_depth};

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

/// 本轮 Chronicle 工具调用预算（规格 `tool_summary_max` / `tool_full_max` / `search_max`）。
///
/// 用 `Arc` 包一层，使 `ToolContext` clone 后仍共享计数；每轮写作开始时换新实例。
#[derive(Debug, Default)]
pub struct ChronicleToolBudget {
    search: std::sync::atomic::AtomicU32,
    summary: std::sync::atomic::AtomicU32,
    full: std::sync::atomic::AtomicU32,
}

impl Clone for ChronicleToolBudget {
    fn clone(&self) -> Self {
        use std::sync::atomic::Ordering::Relaxed;
        Self {
            search: std::sync::atomic::AtomicU32::new(self.search.load(Relaxed)),
            summary: std::sync::atomic::AtomicU32::new(self.summary.load(Relaxed)),
            full: std::sync::atomic::AtomicU32::new(self.full.load(Relaxed)),
        }
    }
}

impl ChronicleToolBudget {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_consume_search(&self) -> Result<u32, ToolError> {
        self.try_consume(
            &self.search,
            storyforge_domain::chronicle::DEFAULT_SEARCH_MAX,
            "search_chronicle",
        )
    }

    pub fn try_consume_summary(&self) -> Result<u32, ToolError> {
        self.try_consume(
            &self.summary,
            storyforge_domain::chronicle::DEFAULT_TOOL_SUMMARY_MAX,
            "get_chronicle(summary)",
        )
    }

    pub fn try_consume_full(&self) -> Result<u32, ToolError> {
        self.try_consume(
            &self.full,
            storyforge_domain::chronicle::DEFAULT_TOOL_FULL_MAX,
            "get_chronicle(full)",
        )
    }

    fn try_consume(
        &self,
        counter: &std::sync::atomic::AtomicU32,
        max: u32,
        label: &str,
    ) -> Result<u32, ToolError> {
        use std::sync::atomic::Ordering::Relaxed;
        loop {
            let cur = counter.load(Relaxed);
            if cur >= max {
                return Err(ToolError::BadArgs(format!(
                    "本轮 {label} 已达上限 {max}，请改用已读结果或缩小查询"
                )));
            }
            if counter
                .compare_exchange(cur, cur + 1, Relaxed, Relaxed)
                .is_ok()
            {
                return Ok(cur + 1);
            }
        }
    }

    pub fn snapshot(&self) -> (u32, u32, u32) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.search.load(Relaxed),
            self.summary.load(Relaxed),
            self.full.load(Relaxed),
        )
    }
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
    /// Chronicle 目录（A/B/C RoundSummary 兼容视图；search/get_chronicle 用）
    pub chronicle_summaries: Vec<storyforge_domain::agent::RoundSummary>,
    /// 本轮 Chronicle 工具预算（共享计数）
    pub chronicle_tool_budget: Arc<ChronicleToolBudget>,
    /// Campaign 运行时快照（阶段 2 新增）。None = 未开 Campaign，走旧路径。
    pub campaign_runtime: Option<Arc<CampaignRuntimeContext>>,
    /// 当前子 Agent 绑定的 instance id（阶段 4 新增）。
    /// 用于子 Agent get_character 工具：只返回自己的 instance 数据，不泄露其他角色。
    /// 导演/编剧/无 Campaign 时为 None。
    pub current_character_instance_id: Option<Id>,
    /// 本轮合并后的 ST regex scripts。工具返回 prompt 内容前可复用。
    pub regex_scripts: Vec<RegexScript>,
}

impl ToolContext {
    /// 测试/占位用最小上下文。
    pub fn empty() -> Self {
        Self {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: Arc::new(ChronicleToolBudget::new()),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        }
    }

    /// 开始新一轮写作：重置 Chronicle 工具预算（保留目录与其它字段）。
    pub fn reset_chronicle_tool_budget(&mut self) {
        self.chronicle_tool_budget = Arc::new(ChronicleToolBudget::new());
    }
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
    ///
    /// A2（缓存稳定性）：按工具名排序，保证同一组工具在每次请求中的 JSON 顺序一致。
    /// HashMap 迭代序不确定，会导致 tools 数组顺序变化，破坏前缀缓存命中。
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> = self.tools.values().map(|(_, spec)| spec.clone()).collect();
        specs.sort_by(|a, b| a.function.name.cmp(&b.function.name));
        specs
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

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
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
                            let content = render_world_info_tool_content(&e.content, &ctx);
                            serde_json::json!({
                                "keys": e.keys,
                                "content": content,
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
                if let Some(runtime) = &ctx.campaign_runtime
                    && let Some(inst) = runtime.find_instance_by_id_or_name(name)
                {
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
            "输出结构化的写作计划。包含场景简述、可选 ScenePlan（冲突/节拍等）和每个子 Agent 的任务（可选欲望/手头动作）。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "scene_brief": {"type": "string", "description": "场景简述"},
                    "scene_plan": {
                        "type": "object",
                        "description": "扩展场景规划（可选）",
                        "properties": {
                            "conflict": {"type": "string"},
                            "opposing_goals": {"type": "array", "items": {"type": "string"}},
                            "stakes": {"type": "string"},
                            "beats": {"type": "array", "items": {"type": "string"}},
                            "complication": {"type": "string"},
                            "must_not_resolve": {"type": "string"},
                            "exit_hook": {"type": "string"}
                        }
                    },
                    "subagent_tasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "character_id": {"type": "string"},
                                "brief": {"type": "string"},
                                "current_desire": {"type": "string", "description": "与用户输入无关的当前欲望"},
                                "ongoing_action": {"type": "string", "description": "进场前正在做的事"},
                                "emotion_stage": {"type": "integer", "minimum": 1, "maximum": 6, "description": "情绪阶段 1-6，禁止正文直说"}
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

    // search_chronicle: 搜 Chronicle A/B/C 目录（不含消息归档块）
    registry.register(
        ToolSpec::function(
            "search_chronicle",
            "搜索剧情纪要目录（code + headline）。含 A/B/C 规范 Chronicle，不含消息归档块。用于点名远楼线索。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "关键词（匹配 code/headline/summary）"},
                    "limit": {"type": "integer", "description": "最多返回条数（默认 3）"},
                    "include_covered": {"type": "boolean", "description": "是否包含已被折叠的条目（默认 false）"},
                    "level": {"type": "string", "enum": ["a", "b", "c", "any"], "description": "限定层级，默认 any"}
                },
                "required": ["query"]
            }),
        ),
        |args, ctx| {
            Box::pin(async move {
                let used = ctx.chronicle_tool_budget.try_consume_search()?;
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::BadArgs("缺少 query".into()))?
                    .to_lowercase();
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3)
                    .clamp(1, 10) as usize;
                let include_covered = args
                    .get("include_covered")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let level_filter = args
                    .get("level")
                    .and_then(|v| v.as_str())
                    .unwrap_or("any")
                    .to_ascii_lowercase();

                // 新→旧：优先较近条目；B/C 与 A 同池按 turn_end 排
                let mut catalog: Vec<&storyforge_domain::agent::RoundSummary> =
                    ctx.chronicle_summaries.iter().collect();
                catalog.sort_by_key(|s| std::cmp::Reverse(s.effective_turn_end()));

                let mut hits = Vec::new();
                for s in catalog {
                    if !include_covered && s.covered_by.is_some() {
                        continue;
                    }
                    if level_filter != "any" {
                        let want = match level_filter.as_str() {
                            "a" => 0,
                            "b" => 1,
                            "c" => 2,
                            _ => return Err(ToolError::BadArgs(
                                "level 须为 a/b/c/any".into(),
                            )),
                        };
                        if s.level != want {
                            continue;
                        }
                    }
                    let code = s.code.clone().unwrap_or_default();
                    let headline = s.overview_headline(40);
                    let hay = format!(
                        "{} {} {}",
                        code.to_lowercase(),
                        headline.to_lowercase(),
                        s.content.to_lowercase()
                    );
                    if query.is_empty() || hay.contains(&query) {
                        hits.push(serde_json::json!({
                            "code": code,
                            "level": s.level,
                            "level_name": chronicle_level_name(s.level),
                            "headline": headline,
                            "turn_span": [s.turn, s.effective_turn_end()],
                            "covers_count": s.covers.len(),
                            "chronicle_entry_id": s.id.to_string(),
                            "covered_by": s.covered_by.as_ref().map(|id| id.to_string()),
                        }));
                    }
                    if hits.len() >= limit {
                        break;
                    }
                }
                Ok(serde_json::json!({
                    "query": args.get("query").and_then(|v| v.as_str()).unwrap_or(""),
                    "results_count": hits.len(),
                    "results": hits,
                    "budget_used_search": used,
                    "budget_max_search": storyforge_domain::chronicle::DEFAULT_SEARCH_MAX,
                }))
            })
        },
    );

    // get_chronicle: 默认 summary；full 可选（含 B/C）
    registry.register(
        ToolSpec::function(
            "get_chronicle",
            "按 code 或 id 读取一条剧情纪要（A/B/C）。默认 detail=summary；full 返回更长正文。",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "code": {"type": "string", "description": "如 A0012 / B0003 / C0001"},
                    "id": {"type": "string", "description": "chronicle_entry_id / RoundSummary id"},
                    "detail": {"type": "string", "enum": ["summary", "full"], "description": "默认 summary"}
                }
            }),
        ),
        |args, ctx| {
            Box::pin(async move {
                let code = args.get("code").and_then(|v| v.as_str());
                let id = args.get("id").and_then(|v| v.as_str());
                if code.is_none() && id.is_none() {
                    return Err(ToolError::BadArgs("需要 code 或 id".into()));
                }
                let detail = args
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("summary");
                let used = if detail == "full" {
                    ctx.chronicle_tool_budget.try_consume_full()?
                } else {
                    ctx.chronicle_tool_budget.try_consume_summary()?
                };
                let found = ctx.chronicle_summaries.iter().find(|s| {
                    if let Some(id) = id
                        && s.id.as_str() == id
                    {
                        return true;
                    }
                    if let Some(code) = code
                        && s.code.as_deref() == Some(code)
                    {
                        return true;
                    }
                    false
                });
                let Some(s) = found else {
                    return Ok(serde_json::json!({
                        "found": false,
                        "message": "未找到对应纪要",
                    }));
                };
                let body = if detail == "full" {
                    s.content.clone()
                } else if let Some(h) = s.headline.as_ref().filter(|h| !h.trim().is_empty()) {
                    // summary：headline + 短截断正文，便于 B/C 导航
                    let max = 240usize;
                    let snippet = if s.content.chars().count() <= max {
                        s.content.clone()
                    } else {
                        format!("{}…", s.content.chars().take(max).collect::<String>())
                    };
                    if snippet.trim().is_empty() || snippet == *h {
                        h.clone()
                    } else {
                        format!("{h}\n{snippet}")
                    }
                } else {
                    let max = 240usize;
                    if s.content.chars().count() <= max {
                        s.content.clone()
                    } else {
                        format!("{}…", s.content.chars().take(max).collect::<String>())
                    }
                };
                let source_kind = match s.level {
                    1 => "chronicle_b",
                    2 => "chronicle_c",
                    _ => "chronicle_a",
                };
                // B/C：递归展开 covers 到 leaf A 的 turn 集合；A：单 turn
                let source_turn_ids = expand_source_turn_ids(s, &ctx.chronicle_summaries);
                Ok(serde_json::json!({
                    "found": true,
                    "code": s.code,
                    "level": s.level,
                    "level_name": chronicle_level_name(s.level),
                    "headline": s.overview_headline(40),
                    "detail": detail,
                    "body": body,
                    "turn_span": [s.turn, s.effective_turn_end()],
                    "covers": s.covers.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                    "chronicle_entry_id": s.id.to_string(),
                    "source_turn_ids": source_turn_ids,
                    "source_kind": source_kind,
                    "covered_by": s.covered_by.as_ref().map(|id| id.to_string()),
                    "budget_used": used,
                    "budget_max": if detail == "full" {
                        storyforge_domain::chronicle::DEFAULT_TOOL_FULL_MAX
                    } else {
                        storyforge_domain::chronicle::DEFAULT_TOOL_SUMMARY_MAX
                    },
                }))
            })
        },
    );
}

fn expand_source_turn_ids(
    entry: &storyforge_domain::agent::RoundSummary,
    catalog: &[storyforge_domain::agent::RoundSummary],
) -> Vec<u32> {
    use std::collections::{HashSet, VecDeque};
    if entry.covers.is_empty() {
        // leaf A 或无 covers 的 stage：返回闭区间内全部 turn（span 连续假设）
        let start = entry.turn;
        let end = entry.effective_turn_end();
        return (start..=end).collect();
    }
    let by_id: std::collections::HashMap<_, _> =
        catalog.iter().map(|s| (s.id.to_string(), s)).collect();
    let mut turns = HashSet::new();
    let mut q: VecDeque<String> = entry.covers.iter().map(|id| id.to_string()).collect();
    let mut seen = HashSet::new();
    while let Some(id) = q.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(child) = by_id.get(&id) {
            if child.covers.is_empty() || child.is_leaf_a() {
                let start = child.turn;
                let end = child.effective_turn_end();
                for t in start..=end {
                    turns.insert(t);
                }
            } else {
                for c in &child.covers {
                    q.push_back(c.to_string());
                }
                // 若子 stage 无进一步 covers 命中，至少纳入其 span
                if child.covers.is_empty() {
                    for t in child.turn..=child.effective_turn_end() {
                        turns.insert(t);
                    }
                }
            }
        }
    }
    if turns.is_empty() {
        return (entry.turn..=entry.effective_turn_end()).collect();
    }
    let mut out: Vec<u32> = turns.into_iter().collect();
    out.sort_unstable();
    out
}

fn chronicle_level_name(level: u8) -> &'static str {
    match level {
        1 => "B",
        2 => "C",
        _ => "A",
    }
}

fn render_world_info_tool_content(content: &str, ctx: &ToolContext) -> String {
    if ctx.regex_scripts.is_empty() {
        return content.to_string();
    }

    apply_regex_scripts_for_target_at_depth(
        content,
        &ctx.regex_scripts,
        RegexPlacement::WorldInfo,
        RegexExecutionTarget::Prompt,
        0,
    )
    .unwrap_or_else(|e| {
        tracing::warn!("世界书工具正则执行失败，使用原始世界书内容: {e}");
        content.to_string()
    })
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
                if let Some(ref instance_id) = ctx.current_character_instance_id
                    && let Some(ref runtime) = ctx.campaign_runtime
                {
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
                        } else {
                            // P0 修复：已绑定 instance_id + 有 campaign_runtime，但绑定 id 在
                            // runtime.instances 中找不到（stale / mismatched id）。旧实现会 fallthrough
                            // 到下方未隔离的扁平 Character 搜索，导致信息泄漏。正常流程不可达
                            // （spawn_subagents 用同一批 runtime 做匹配与 tool ctx），属
                            // defense-in-depth 洞，此处硬失败以符合隔离意图。
                            return Err(ToolError::NotFound(
                                format!(
                                    "子 Agent 绑定的 instance_id '{instance_id}' 在当前 Campaign runtime 中找不到，拒绝降级到扁平角色查询以防信息泄漏"
                                )
                            ));
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
            tasks: vec![],
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
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: Some(runtime),
            current_character_instance_id: None,
            regex_scripts: vec![],
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
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: Some(runtime),
            current_character_instance_id: None,
            regex_scripts: vec![],
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
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
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
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: Some(runtime),
            current_character_instance_id: None,
            regex_scripts: vec![],
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
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: Some(cr),
            current_character_instance_id: Some(Id::from_str("inst-lin")),
            regex_scripts: vec![],
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
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
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

    /// P0 回归：子 Agent 已绑定 current_character_instance_id 且有 campaign_runtime，
    /// 但绑定 id 在 runtime.instances 中找不到（stale/mismatched）时，必须硬失败 NotFound，
    /// 不得 fallthrough 到未隔离的扁平 Character 搜索。
    ///
    /// 正常流程不可达（spawn_subagents 用同一批 runtime 做匹配与 tool ctx），本测试钉
    /// defense-in-depth 不变量。修复前会泄漏扁平 ctx.characters 的角色数据。
    #[tokio::test]
    async fn test_subagent_get_character_bound_but_unresolvable_does_not_leak() {
        let runtime = make_campaign_runtime_with_lin(); // 含 inst-lin
        // 故意构造一个不在 runtime 中的 instance_id，并把一个扁平 Character 塞进 ctx.characters
        let ctx = Arc::new(ToolContext {
            characters: vec![make_flat_character("Seraphina")],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: Some(runtime),
            current_character_instance_id: Some(Id::from_str("inst-nonexistent")),
            regex_scripts: vec![],
        });

        let mut registry = ToolRegistry::new();
        register_subagent_tools(&mut registry);

        // 查扁平 Character 的名字 —— 修复前会泄漏 Seraphina 数据，修复后必须 NotFound
        let result = registry
            .dispatch(
                "get_character",
                serde_json::json!({"name": "Seraphina"}),
                ctx,
            )
            .await;
        assert!(
            result.is_err(),
            "绑定但 unresolvable 的 instance_id 不应降级到扁平角色查询（信息泄漏）"
        );
        let err = result.unwrap_err();
        match err {
            ToolError::NotFound(_) => {}
            other => panic!("期望 NotFound，实际: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_search_world_info_returns_only_selective_matches() {
        use storyforge_domain::world_info::{
            LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry,
        };

        fn entry(content: &str, route: LoreRoute, keys: &[&str]) -> WorldInfoEntry {
            WorldInfoEntry {
                st_id: None,
                keys: keys.iter().map(|s| (*s).to_string()).collect(),
                secondary_keys: vec![],
                content: content.to_string(),
                constant: matches!(route, LoreRoute::Constant | LoreRoute::Both),
                selective: matches!(route, LoreRoute::Selective | LoreRoute::Both),
                selective_logic: SelectiveLogic::And,
                disabled: false,
                position: 0,
                depth: 2,
                order: 100,
                route,
                extensions: serde_json::json!({}),
                extra: Default::default(),
            }
        }

        let ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: Some(Arc::new(WorldInfoBook {
                source: storyforge_domain::Source::Native,
                entries: vec![
                    entry("constant lore", LoreRoute::Constant, &["castle"]),
                    entry("selective lore", LoreRoute::Selective, &["forest"]),
                    entry("both lore", LoreRoute::Both, &["harbor"]),
                    entry("disabled route lore", LoreRoute::Disabled, &["dungeon"]),
                ],
                metadata: Default::default(),
            })),
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });

        let mut registry = ToolRegistry::new();
        register_director_tools(&mut registry);

        let result = registry
            .dispatch(
                "search_world_info",
                serde_json::json!({"query": "castle forest harbor dungeon"}),
                ctx,
            )
            .await
            .unwrap();

        assert_eq!(result["results_count"], 2);
        let contents: Vec<&str> = result["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["content"].as_str().unwrap())
            .collect();
        assert_eq!(contents, vec!["selective lore", "both lore"]);
    }

    #[tokio::test]
    async fn test_search_world_info_applies_world_info_regex() {
        use storyforge_domain::preset::{
            RegexPlacement, RegexScript, RegexScriptSource, ST_REGEX_PLACEMENT_WORLD_INFO,
        };
        use storyforge_domain::world_info::{
            LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry,
        };

        let mut registry = ToolRegistry::new();
        register_director_tools(&mut registry);
        let ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: Some(Arc::new(WorldInfoBook {
                source: storyforge_domain::Source::Native,
                entries: vec![WorldInfoEntry {
                    st_id: None,
                    keys: vec!["moon".into()],
                    secondary_keys: vec![],
                    content: "Tool lore: {{MOON}}".into(),
                    constant: false,
                    selective: true,
                    selective_logic: SelectiveLogic::And,
                    disabled: false,
                    position: 0,
                    depth: 2,
                    order: 100,
                    route: LoreRoute::Selective,
                    extensions: serde_json::json!({}),
                    extra: Default::default(),
                }],
                metadata: Default::default(),
            })),
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(crate::tools::ChronicleToolBudget::new()),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![RegexScript {
                id: "world-info-format".into(),
                script_name: "world-info-format".into(),
                find_regex: r"\{\{MOON\}\}".into(),
                replace_string: "Lunar Vault".into(),
                placement: RegexPlacement::WorldInfo,
                placement_codes: vec![ST_REGEX_PLACEMENT_WORLD_INFO],
                source: RegexScriptSource::Preset,
                disabled: false,
                flags: "gm".into(),
                only_format_formatting: None,
                markdown_only: None,
                prompt_only: None,
                run_on_edit: None,
                substitute_regex: None,
                trim_strings: vec![],
                min_depth: None,
                max_depth: None,
            }],
        });

        let result = registry
            .dispatch(
                "search_world_info",
                serde_json::json!({"query":"moon"}),
                ctx,
            )
            .await
            .unwrap();

        assert_eq!(result["results_count"], 1);
        assert_eq!(result["results"][0]["content"], "Tool lore: Lunar Vault");
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
                    chronicle_summaries: vec![],
                    chronicle_tool_budget: std::sync::Arc::new(
                        crate::tools::ChronicleToolBudget::new(),
                    ),
                    campaign_runtime: None,
                    current_character_instance_id: None,
                    regex_scripts: vec![],
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

    /// A2：tool_specs() 返回按工具名排序（确定性顺序，稳定前缀缓存）
    #[test]
    fn tool_specs_sorted_by_name() {
        let registry = registry_with_director_tools();
        let specs = registry.tool_specs();
        let names: Vec<String> = specs.iter().map(|s| s.function.name.clone()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            names, sorted,
            "tool_specs() 应按名排序，实际顺序: {names:?}"
        );
    }

    /// A2：多次调用 tool_specs() 返回相同顺序
    #[test]
    fn tool_specs_consistent_order_across_calls() {
        let registry = registry_with_director_tools();
        let first: Vec<String> = registry
            .tool_specs()
            .iter()
            .map(|s| s.function.name.clone())
            .collect();
        let second: Vec<String> = registry
            .tool_specs()
            .iter()
            .map(|s| s.function.name.clone())
            .collect();
        assert_eq!(first, second);
    }

    fn sample_chronicle_catalog() -> Vec<storyforge_domain::agent::RoundSummary> {
        use storyforge_domain::Id;
        use storyforge_domain::agent::RoundSummary;
        let camp = Id::from_str("c1");
        let conv = Id::from_str("v1");
        let a = RoundSummary::new(camp.clone(), conv.clone(), 1, "远楼A：旧日密约".into())
            .with_code("A0001")
            .with_headline("旧日密约");
        let mut b = RoundSummary::new(camp.clone(), conv.clone(), 1, "中期折叠：密约与背叛".into())
            .with_code("B0001")
            .with_headline("密约与背叛");
        b.level = 1;
        b.turn_end = 4;
        b.covers = vec![a.id.clone()];
        let a3 = RoundSummary::new(camp, conv, 6, "近轮A：对峙".into())
            .with_code("A0006")
            .with_headline("对峙");
        vec![a, b, a3]
    }

    #[tokio::test]
    async fn search_chronicle_finds_level_b() {
        let mut catalog = sample_chronicle_catalog();
        // clear covered_by on a3 path; ensure B is searchable
        catalog[1].covered_by = None;
        let budget = Arc::new(ChronicleToolBudget::new());
        let ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: catalog,
            chronicle_tool_budget: budget.clone(),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let mut registry = ToolRegistry::new();
        register_director_tools(&mut registry);
        let result = registry
            .dispatch(
                "search_chronicle",
                serde_json::json!({"query": "密约", "level": "b"}),
                ctx,
            )
            .await
            .unwrap();
        assert_eq!(result["results_count"], 1);
        assert_eq!(result["results"][0]["code"], "B0001");
        assert_eq!(result["results"][0]["level"], 1);
        assert_eq!(result["results"][0]["level_name"], "B");
        assert_eq!(result["results"][0]["turn_span"][1], 4);
        assert_eq!(budget.snapshot().0, 1);
    }

    #[tokio::test]
    async fn get_chronicle_full_budget_enforced() {
        let catalog = sample_chronicle_catalog();
        let budget = Arc::new(ChronicleToolBudget::new());
        let ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: catalog,
            chronicle_tool_budget: budget.clone(),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let mut registry = ToolRegistry::new();
        register_director_tools(&mut registry);
        for _ in 0..storyforge_domain::chronicle::DEFAULT_TOOL_FULL_MAX {
            let ok = registry
                .dispatch(
                    "get_chronicle",
                    serde_json::json!({"code": "B0001", "detail": "full"}),
                    ctx.clone(),
                )
                .await
                .unwrap();
            assert_eq!(ok["found"], true);
            assert_eq!(ok["source_kind"], "chronicle_b");
        }
        let err = registry
            .dispatch(
                "get_chronicle",
                serde_json::json!({"code": "B0001", "detail": "full"}),
                ctx,
            )
            .await;
        assert!(
            matches!(err, Err(ToolError::BadArgs(_))),
            "over full budget: {err:?}"
        );
        assert_eq!(
            budget.snapshot().2,
            storyforge_domain::chronicle::DEFAULT_TOOL_FULL_MAX
        );
    }

    #[tokio::test]
    async fn search_chronicle_budget_enforced() {
        let catalog = sample_chronicle_catalog();
        let budget = Arc::new(ChronicleToolBudget::new());
        let ctx = Arc::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: catalog,
            chronicle_tool_budget: budget,
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        });
        let mut registry = ToolRegistry::new();
        register_director_tools(&mut registry);
        for _ in 0..storyforge_domain::chronicle::DEFAULT_SEARCH_MAX {
            registry
                .dispatch(
                    "search_chronicle",
                    serde_json::json!({"query": "对峙"}),
                    ctx.clone(),
                )
                .await
                .unwrap();
        }
        let err = registry
            .dispatch(
                "search_chronicle",
                serde_json::json!({"query": "对峙"}),
                ctx,
            )
            .await;
        assert!(matches!(err, Err(ToolError::BadArgs(_))), "{err:?}");
    }

    #[test]
    fn expand_source_turn_ids_recurses_c_to_a() {
        use storyforge_domain::Id;
        use storyforge_domain::agent::RoundSummary;
        let camp = Id::from_str("c1");
        let conv = Id::from_str("v1");
        let a1 = RoundSummary::new(camp.clone(), conv.clone(), 1, "a1".into()).with_code("A0001");
        let a2 = RoundSummary::new(camp.clone(), conv.clone(), 2, "a2".into()).with_code("A0002");
        let a3 = RoundSummary::new(camp.clone(), conv.clone(), 3, "a3".into()).with_code("A0003");
        let a4 = RoundSummary::new(camp.clone(), conv.clone(), 4, "a4".into()).with_code("A0004");
        let mut b = RoundSummary::new(camp.clone(), conv.clone(), 1, "b".into())
            .with_code("B0001")
            .with_headline("b");
        b.level = 1;
        b.turn_end = 4;
        b.covers = vec![a1.id.clone(), a2.id.clone(), a3.id.clone(), a4.id.clone()];
        let mut c = RoundSummary::new(camp, conv, 1, "c".into())
            .with_code("C0001")
            .with_headline("c");
        c.level = 2;
        c.turn_end = 4;
        c.covers = vec![b.id.clone()];
        let catalog = vec![a1, a2, a3, a4, b, c.clone()];
        assert_eq!(expand_source_turn_ids(&c, &catalog), vec![1, 2, 3, 4]);
    }
}
