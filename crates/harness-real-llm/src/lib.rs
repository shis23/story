//! 真实 LLM 驱动的端到端测试 harness。
//!
//! 目的（详见 docs/HARNESS-FINDINGS-2026-06-18.md 与 PLAN）：
//! 1. 绕开前端，用真实 LLM 跑通「点遍各按钮」全链路，扫出潜在 bug。
//! 2. 验证各 agent 的知识边界（信息隔离）。
//!
//! 架构：pipeline crate 100% Tauri-free，harness 直接构造
//! `PipelineOrchestrator` + tempdir `CampaignStore`/`ConversationStore`，
//! 不经过 `AppState` / 全局 store / Tauri runtime。campaign-mode 组装复用
//! `storyforge::fill_campaign_runtime_from_store`（线上同一份逻辑）。
//!
//! 凭证：环境变量优先回退——有 `LLM_BASE_URL`/`LLM_API_KEY`/`LLM_MODEL` 用之；
//! 否则读 `data/connections.json` 的 active 连接。真实 LLM 测试一律 `#[ignore]`，
//! 由 `require_real_llm()` 早返保护，`cargo test` 默认零网络。
//!
//! M5 / Phase B 评估扩展：
//! - [`evidence`] 脱敏 JSONL 证据
//! - [`budget`] 真实 LLM 调用预算 + usage 录制
//! - [`commit_probe`] 生产忠实 CommitTurn/Accept
//! - [`long_session`] ≥20 Accept + ContextEpoch membership 校验
//! - [`phase_b_matrix`] Phase B A/B 对照矩阵

pub mod budget;
pub mod commit_probe;
pub mod context_compile_bench;
pub mod endurance;
pub mod evidence;
pub mod long_session;
pub mod observability;
pub mod phase_b_matrix;
pub mod production_evidence;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use storyforge_app_conversation::ConversationStore;
use storyforge_app_pipeline::{PipelineOrchestrator, WritingContext};
use storyforge_domain::Id;
use storyforge_domain::character::Character;
use storyforge_domain::llm::{LlmConnection, LlmProtocol, SamplingParams, ToolMode};
use storyforge_infra_llm::LlmClient;
use storyforge_infra_vector::BruteForceStore;

// 从 tauri-app crate 复用线上 campaign-mode 组装逻辑（零行为复制）。
use storyforge_app_agent::ToolContext;
use storyforge_tauri_app::campaign_store::CampaignStore;
use storyforge_tauri_app::fill_campaign_runtime_from_store;
use storyforge_tauri_app::turn_store::TurnStore;

/// 一次性环境：全部 tempdir，进程隔离，不污染真实 `data/`。
pub struct HarnessEnv {
    pub data_dir: PathBuf,
    pub campaign_store: Arc<CampaignStore>,
    pub conv_store: Arc<ConversationStore>,
    pub turn_store: Arc<TurnStore>,
    pub tool_ctx: Arc<RwLock<ToolContext>>,
    pub vector_store: Arc<BruteForceStore>,
    pub llm: Arc<dyn LlmClient>,
    /// 当前活跃 Campaign（等价线上 `AppState.active_campaign`）。
    pub active_campaign: Mutex<Option<Id>>,
}

impl HarnessEnv {
    /// 新建隔离环境。`llm` 由调用方提供（真实或 mock）。
    pub fn new(llm: Arc<dyn LlmClient>) -> Self {
        let data_dir =
            std::env::temp_dir().join(format!("storyforge_harness_{}", uuid::Uuid::new_v4()));
        Self::open(data_dir, llm)
    }

    /// Open or create a harness environment rooted at an explicit data directory.
    /// Used by endurance resume so accepted turns can continue from disk state.
    pub fn open(data_dir: PathBuf, llm: Arc<dyn LlmClient>) -> Self {
        std::fs::create_dir_all(&data_dir).expect("create data_dir 失败");

        let campaign_store = Arc::new(CampaignStore::new(&data_dir));
        let conv_store = Arc::new(ConversationStore::new(data_dir.join("conversations")));
        let turn_store = Arc::new(TurnStore::new(&data_dir));
        let tool_ctx = Arc::new(RwLock::new(ToolContext {
            characters: vec![],
            world_info: None,
            vector_store: None,
            archived_summaries: vec![],
            chronicle_summaries: vec![],
            chronicle_tool_budget: std::sync::Arc::new(
                storyforge_app_agent::ChronicleToolBudget::new(),
            ),
            campaign_runtime: None,
            current_character_instance_id: None,
            regex_scripts: vec![],
        }));
        let vector_store = Arc::new(BruteForceStore::with_persistence(
            data_dir.join("vectors.json"),
        ));

        Self {
            data_dir,
            campaign_store,
            conv_store,
            turn_store,
            tool_ctx,
            vector_store,
            llm,
            active_campaign: Mutex::new(None),
        }
    }

    /// 清理 tempdir（测试结尾调用，best-effort）。
    pub fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }

    /// 把已导入的 `Character` 注入 tool_ctx（等价线上 `import_character` 的同步逻辑，
    /// 简化版：不处理 world_info 向量库——harness 不依赖 search_vectors）。
    pub fn inject_character(&self, character: Character) {
        let mut ctx = self.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.characters.retain(|c| c.name != character.name);
        if let Some(wi) = character.embedded_world_info.clone() {
            ctx.world_info = Some(Arc::new(wi));
        }
        ctx.characters.push(Arc::new(character));
    }

    /// 设活跃 Campaign（等价线上 `set_active_campaign`，但只动内存——
    /// harness 不依赖磁盘 `active_campaign.json`）。
    pub fn set_active_campaign(&self, id: Id) {
        *self.active_campaign.lock().unwrap() = Some(id);
    }

    /// 组装一个 campaign-mode 的 `WritingContext`（复用线上 `fill_campaign_runtime_from_store`）。
    ///
    /// `base_ctx` 用 `WritingContext::legacy` 起步，本方法在其上填充 campaign 字段 +
    /// 同步 `campaign_runtime` 到 `tool_ctx`。等价线上 `start_writing` 前的上下文组装。
    pub fn fill_campaign_context(&self, mut ctx: WritingContext) -> WritingContext {
        ctx.campaign_runtime = None;
        {
            let mut g = self.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
            g.campaign_runtime = None;
        }
        let active_id = match self.active_campaign.lock().unwrap().clone() {
            Some(id) => id,
            None => return ctx,
        };
        fill_campaign_runtime_from_store(
            &mut ctx,
            &self.tool_ctx,
            &self.campaign_store,
            &active_id,
        );
        ctx
    }

    /// 构造 PipelineOrchestrator（用本环境的 LLM + tool_ctx 快照 + vector_store）。
    pub fn new_pipeline(&self) -> PipelineOrchestrator {
        let llm = self.llm.clone();
        let mut tool_ctx = (*self.tool_ctx.read().unwrap_or_else(|p| p.into_inner())).clone();
        tool_ctx.vector_store = Some(self.vector_store.clone());
        PipelineOrchestrator::new(llm, self.conv_store.clone(), Arc::new(tool_ctx), None)
    }

    /// 当前活跃 Campaign id。
    pub fn active_campaign_id(&self) -> Option<Id> {
        self.active_campaign.lock().unwrap().clone()
    }

    /// 复刻线上 `extract_characters`（tauri-app lib.rs:3970）：跑识别 Agent 建
    /// CharacterCard。失败时降级建单角色卡（与线上同行为）。
    pub async fn extract_characters(
        &self,
        source_character_id: &str,
    ) -> storyforge_domain::character::CharacterCard {
        use storyforge_app_agent::{
            AgentRuntime, attach_definitions_to_card, extract_characters as run_extract,
        };
        use storyforge_domain::character::{
            CharacterCard, CharacterDefinition, CharacterExtractionStatus,
        };
        use storyforge_domain::variables::extract_mvu_schema_from_extensions;

        let character = {
            let ctx = self.tool_ctx.read().unwrap_or_else(|p| p.into_inner());
            ctx.characters
                .iter()
                .find(|c| c.id.as_str() == source_character_id)
                .map(|c| (*c).clone())
                .expect("extract_characters: tool_ctx 中找不到 source 角色")
        };

        // 已存在则复用
        if let Some(existing) = self.campaign_store.get_card_by_source(&character.id) {
            return existing.card;
        }

        let mvu_schema = extract_mvu_schema_from_extensions(&character.extensions);
        let tool_ctx_snapshot =
            Arc::new((*self.tool_ctx.read().unwrap_or_else(|p| p.into_inner())).clone());
        let runtime = AgentRuntime::new(self.llm.clone(), tool_ctx_snapshot);
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        let (definitions, extraction_status, extraction_message) = match run_extract(
            &runtime,
            &character,
            &mvu_schema,
            cancel_rx,
        )
        .await
        {
            Ok(defs) => {
                eprintln!(
                    "[F2 DIAG] extract_characters Ok: {} definitions (正常解析路径)",
                    defs.len()
                );
                (defs, CharacterExtractionStatus::Extracted, None)
            }
            Err(e) => {
                eprintln!(
                    "[F2 DIAG] extract_characters Err: {e} → 降级单角色 (fallback_from_character)"
                );
                (
                    vec![CharacterDefinition::fallback_from_character(
                        &character,
                        &mvu_schema,
                    )],
                    CharacterExtractionStatus::Fallback,
                    Some("识别失败，已按单角色处理，可重新识别。".into()),
                )
            }
        };

        let mut card = CharacterCard::from_character(&character);
        let definitions = attach_definitions_to_card(definitions, &card.id);
        card.character_definitions = definitions;
        card.extraction_status = extraction_status;
        card.extraction_message = extraction_message;
        let stored = self
            .campaign_store
            .save_card(card)
            .expect("save_card failed");
        stored.card
    }

    /// 复刻线上 `create_campaign`（tauri-app lib.rs:4085）：建 Campaign + 实例化
    /// 所有 Protagonist/Supporting 定义。返回 campaign id（已设为活跃）。
    pub fn create_campaign(
        &self,
        card: &storyforge_domain::character::CharacterCard,
        name: &str,
    ) -> Id {
        use storyforge_domain::campaign::{Campaign, CharacterInstance};
        use storyforge_domain::character::RoleType;

        let campaign = Campaign::new(card.id.clone(), name.to_string());
        let campaign_id = campaign.id.clone();
        self.campaign_store
            .save_campaign(campaign)
            .expect("save_campaign failed");

        let mut count = 0;
        for def in &card.character_definitions {
            if matches!(def.role_type, RoleType::Protagonist | RoleType::Supporting) {
                let inst = CharacterInstance::from_definition(campaign_id.clone(), def);
                self.campaign_store
                    .add_instance(inst)
                    .expect("add_instance failed");
                count += 1;
            }
        }
        eprintln!("create_campaign: 实例化 {count} 个角色");
        self.set_active_campaign(campaign_id.clone());
        campaign_id
    }
}

// ─── LLM 凭证解析（环境变量优先，回退 data/connections.json）──────────────

/// 解析真实 LLM 凭证。
///
/// 优先级：`LLM_BASE_URL`+`LLM_API_KEY`+`LLM_MODEL` 环境变量 > `data/connections.json`
/// active 连接（线上 `AppState::new` lib.rs:210-231 同款逻辑）。
///
/// 返回 `Ok(conn)` 表示可构造真实 client；`Err(reason)` 表示无可用凭证。
pub fn resolve_llm_connection() -> Result<LlmConnection, String> {
    let base_url = std::env::var("LLM_BASE_URL").ok().filter(|s| !s.is_empty());
    let api_key = std::env::var("LLM_API_KEY").ok().filter(|s| !s.is_empty());
    let model = std::env::var("LLM_MODEL").ok().filter(|s| !s.is_empty());

    if let Some(conn) = resolve_env_llm_connection(
        base_url,
        api_key,
        model,
        std::env::var("LLM_TOOL_MODE").ok().as_deref(),
    )? {
        return Ok(conn);
    }

    // 回退：读 data/connections.json 的 active 连接
    let data_dir = exe_data_dir();
    let path = data_dir.join("connections.json");
    if !path.exists() {
        return Err(format!(
            "无 LLM 凭证：env 未设全 LLM_BASE_URL/API_KEY/MODEL，且 {} 不存在",
            path.display()
        ));
    }
    let raw =
        std::fs::read_to_string(&path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("解析 connections.json 失败: {e}"))?;
    let active_id = v
        .get("active_id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "connections.json 无 active_id".to_string())?;
    let conn = v
        .get("connections")
        .and_then(|x| x.as_array())
        .and_then(|arr| {
            arr.iter().find(|c| {
                c.get("id").and_then(|x| x.as_str()) == Some(active_id)
                    || c.get("id")
                        .and_then(|x| x.get("id"))
                        .and_then(|x| x.as_str())
                        == Some(active_id)
            })
        })
        .ok_or_else(|| format!("connections.json 找不到 active_id={active_id}"))?;
    // LlmConnection 直接反序列化（字段齐全）
    let conn_obj = conn
        .get("connection")
        .or(Some(conn))
        .ok_or_else(|| "connection 字段缺失".to_string())?;
    let mut conn = serde_json::from_value::<LlmConnection>(conn_obj.clone())
        .map_err(|e| format!("反序列化 LlmConnection 失败: {e}"))?;
    let secret_store = storyforge_infra_util::secret_store::SystemSecretStore::default();
    conn.api_key =
        storyforge_infra_util::secret_store::resolve_secret_value(&conn.api_key, &secret_store)
            .map_err(|e| format!("解析 connections.json SecretRef 失败: {e}"))?;
    Ok(conn)
}

fn resolve_env_llm_connection(
    base_url: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    tool_mode_value: Option<&str>,
) -> Result<Option<LlmConnection>, String> {
    let (Some(base_url), Some(api_key), Some(model)) = (base_url, api_key, model) else {
        return Ok(None);
    };

    let tool_mode = parse_env_tool_mode(tool_mode_value)?;
    Ok(Some(LlmConnection {
        id: Id::new(),
        name: "harness-env".into(),
        base_url,
        api_key,
        model,
        protocol: LlmProtocol::OpenAi,
        params: SamplingParams::default(),
        tool_mode,
    }))
}

fn parse_env_tool_mode(value: Option<&str>) -> Result<ToolMode, String> {
    let Some(value) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(ToolMode::Native);
    };

    match value.to_ascii_lowercase().replace('-', "_").as_str() {
        "native" | "openai" | "tools" | "function_calling" => Ok(ToolMode::Native),
        "text" | "fallback" | "textfallback" | "text_fallback" => Ok(ToolMode::TextFallback),
        other => Err(format!(
            "LLM_TOOL_MODE must be native or text_fallback, got {other}"
        )),
    }
}

/// 在 `#[ignore]` 测试里调用：若拿不到真实凭证则早返（不 fail），否则构造真实 client。
///
/// F1 修好后，`HttpLlmClient.chat()`/`chat_stream()` 已正确回退到连接配置的 model，
/// 不再需要 `ModelPinningLlmClient` wrapper。
pub fn require_real_llm() -> Arc<dyn LlmClient> {
    let mut conn = resolve_llm_connection().expect(
        "未配置真实 LLM 凭证：设 LLM_BASE_URL/LLM_API_KEY/LLM_MODEL 或 data/connections.json",
    );
    // Allow harness-specific max_tokens override without touching production defaults.
    // dsv4f supports up to 384K output; default SamplingParams.max_tokens=4096 is too
    // restrictive for creative writing. This only affects the harness client.
    if let Ok(v) = std::env::var("STORYFORGE_EVAL_MAX_TOKENS")
        && let Ok(n) = v.trim().parse::<u32>()
    {
        conn.params.max_tokens = Some(n);
    }
    let client = storyforge_infra_llm::create_client(&conn)
        .map_err(|e| format!("构造 LLM client 失败: {e}"))
        .expect("LLM client 构造失败");
    Arc::from(client)
}

/// 与线上 `get_app_data_dir` 同语义（exe 同级 data/），仅回退路径用。
fn exe_data_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe_dir.join("data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_tool_mode_defaults_to_native() {
        assert_eq!(parse_env_tool_mode(None).unwrap(), ToolMode::Native);
        assert_eq!(parse_env_tool_mode(Some("")).unwrap(), ToolMode::Native);
        assert_eq!(parse_env_tool_mode(Some("  ")).unwrap(), ToolMode::Native);
    }

    #[test]
    fn env_tool_mode_accepts_native_aliases() {
        assert_eq!(
            parse_env_tool_mode(Some("native")).unwrap(),
            ToolMode::Native
        );
        assert_eq!(
            parse_env_tool_mode(Some("OpenAI")).unwrap(),
            ToolMode::Native
        );
        assert_eq!(
            parse_env_tool_mode(Some("function-calling")).unwrap(),
            ToolMode::Native
        );
    }

    #[test]
    fn env_tool_mode_accepts_text_fallback_aliases() {
        assert_eq!(
            parse_env_tool_mode(Some("text_fallback")).unwrap(),
            ToolMode::TextFallback
        );
        assert_eq!(
            parse_env_tool_mode(Some("text-fallback")).unwrap(),
            ToolMode::TextFallback
        );
        assert_eq!(
            parse_env_tool_mode(Some("fallback")).unwrap(),
            ToolMode::TextFallback
        );
    }

    #[test]
    fn env_tool_mode_rejects_unknown_values() {
        let err = parse_env_tool_mode(Some("json_schema")).unwrap_err();
        assert!(err.contains("LLM_TOOL_MODE"));
    }

    #[test]
    fn env_tool_mode_is_ignored_when_env_credentials_are_incomplete() {
        let conn = resolve_env_llm_connection(
            Some("https://example.invalid/v1/chat/completions".into()),
            None,
            Some("model".into()),
            Some("json_schema"),
        )
        .expect("incomplete env credentials should fall back without parsing tool mode");

        assert!(conn.is_none());
    }
}
