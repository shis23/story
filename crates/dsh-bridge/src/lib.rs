//! DSH 插件版 PoC 的 sidecar 核心（库）。
//!
//! - [`rpc`]：双向 JSON-RPC 对端（传输无关）
//! - [`llm`]：`DshLlmClient`，模型调用反向交给 DSH 插件
//! - [`service`]：`BridgeService`，调用 `storyforge-app-service` 的命令本体跑生产写作路径
//! - 本文件：AppState 装配、请求分发与 stdio 外壳 [`run_stdio`]
//!
//! 协议（每行一个 JSON 对象，格式见 [`rpc`]）：
//!
//! 插件 → sidecar 请求：
//! - `bridge.status {}` → `BridgeStatus`（`busy` 写作锁占用；`writing` 前台成文的存档；
//!   `queued` 排队中的存档。busy 且 writing 为空 = 后台收尾）
//! - `bridge.configure {tool_mode?: "native" | "text_fallback"}` → `{tool_mode}`
//! - `campaign.create {card_path, name?}` → `CampaignCreated`
//! - `campaign.fork {campaign_id, history_turns, name?}` → `CampaignForked`（只从最新一轮）
//! - `turn.run {campaign_id, intent, generation_mode?, history_turns?}` → `TurnOutput`
//!   （成文即返回；`generation_mode` 取 `continuation` / `duet` / `big_scene` /
//!   `sequential_crew`，省略走自动路由；`history_turns` 与存档不一致时报 `out_of_sync`）
//! - `turn.cancel {campaign_id?}` → `{cancelled}`（只取消前台：排队中的写作请求放弃，
//!   正在成文的一轮中止；收尾照常完成。省略 `campaign_id` 时不区分存档）
//! - `conversation.display {refs: [{key, conversation_id?, node_id?, node_index?}],
//!   campaign_id?}` → `{items: {key: NodeDisplay | null}}`（`campaign_id` 是 DSH 会话当前
//!   对应的存档；只读，不占写作锁；解析规则见 `BridgeService::display_nodes`，一次最多
//!   `MAX_DISPLAY_REFS` 条）
//! - `card_shell.manifest {campaign_id}` → `CardShellInfo`（卡的壳清单与可代取主机；只读）
//! - `card_shell.variables_set {campaign_id, variables}` → `{ok}`（整体替换存档的卡壳
//!   选择器桶 `__storyforge_card_shell_variables`；形状与大小不对整份拒绝）
//! - `mvu.status {campaign_id}` → `MvuStatus`（卡的 MVU 翻译、角色定义、实例与 Campaign
//!   变量、待应用的变量定义；只读，不占写作锁）
//! - `mvu.analyze {campaign_id}` → `MvuAnalyzed`（手动分析，调用一次底层模型）
//! - `mvu.apply {campaign_id}` → `MvuStatus`（把翻译出的变量定义应用到卡内角色）
//! - `variables.set {campaign_id, writes: [{scope: "campaign" | "instance", instance_id?,
//!   key, value}]}` → `{results, status}`（逐条写，单条失败不影响其它条；一次最多
//!   `MAX_VARIABLE_WRITES` 条）。后三个与 `card_shell.variables_set` 都先等上一轮收尾结束
//! - `bridge.shutdown {}` → `{}` 后退出
//!
//! sidecar → 插件：
//! - 请求 `llm.stream {...}`（插件回若干 `llm.chunk` 通知 + 响应），通知 `llm.cancel {id}`
//! - 通知 `bridge.ready` / `bridge.fatal`、`turn.event {campaign_id, event}`、
//!   `turn.settled {report}`

pub mod llm;
pub mod rpc;
pub mod service;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use storyforge_app_service::AppState;
use storyforge_app_service::storage_backend::StorageFacade;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::generation::GenerationMode;
use storyforge_domain::llm::RetryConfig;
use storyforge_infra_llm::{LlmClient, RetryingClient};
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};

use crate::llm::{DshLlmClient, LlmBridgeState};
use crate::rpc::{Incoming, Peer, RpcError};
use crate::service::mvu::VariableWrite;
use crate::service::{BridgeError, BridgeService, DisplayRef, SettleReport, TurnObserver};

/// 在 sidecar 自己的数据目录上构造 JSON 后端的 AppState，并注入 DSH 模型客户端。
pub fn build_app(
    data_dir: &Path,
    peer: Arc<Peer>,
    llm: Arc<LlmBridgeState>,
) -> Result<Arc<AppState>, String> {
    // 必须先于任何存储构造：否则全局连接 / 预设存储会解析到用户真实的数据目录。
    let data_dir = storyforge_app_service::pin_app_data_dir(data_dir)?;
    let storage = StorageFacade::new(
        data_dir.clone(),
        PinnedBackend::new(StorageBackend::Json, BackendSource::Config),
    );
    let app = Arc::new(AppState::new_with_backend(data_dir, Arc::new(storage))?);
    // 与生产连接同样的包装：重试（429/5xx/超时）+ 调用日志。
    let retrying = RetryingClient::new(
        Arc::new(DshLlmClient::new(peer, llm)) as Arc<dyn LlmClient>,
        RetryConfig::default(),
    );
    let logged = storyforge_app_logging::interceptor::LlmInterceptor::new(
        Arc::new(retrying),
        app.log_store.clone(),
        "dsh".into(),
    );
    app.install_llm_client(Arc::new(logged));
    // 与 Tauri setup 相同：重放 Committing 态 Turn、把中断的活动 Turn 标记失败
    // （sidecar 崩溃或被杀后，下一轮才不会被活动 Turn 屏障挡住）。
    storyforge_app_service::startup_recovery::run_startup_recovery(&app);
    // 派生向量索引对账只写本地索引、不调嵌入 API，放后台线程。
    {
        let app = app.clone();
        std::thread::spawn(move || {
            storyforge_app_service::vector_index::reconcile_on_startup(&app)
        });
    }
    Ok(app)
}

/// 把本层事件变成发给插件的通知。
pub struct PeerObserver {
    peer: Arc<Peer>,
}

impl PeerObserver {
    pub fn new(peer: Arc<Peer>) -> Self {
        Self { peer }
    }
}

impl TurnObserver for PeerObserver {
    fn pipeline_event(&self, campaign_id: &str, event: &PipelineEvent) {
        self.peer.notify(
            "turn.event",
            json!({ "campaign_id": campaign_id, "event": event }),
        );
    }

    fn settled(&self, report: &SettleReport) {
        self.peer
            .notify("turn.settled", json!({ "report": report }));
    }
}

fn to_rpc(error: BridgeError) -> RpcError {
    RpcError::new(error.code, error.message)
}

fn string_param(params: &Value, key: &str) -> Result<String, RpcError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| RpcError::new("invalid_params", format!("缺少参数 {key}")))
}

fn usize_param(params: &Value, key: &str) -> Result<Option<usize>, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(|n| Some(n as usize))
            .ok_or_else(|| RpcError::new("invalid_params", format!("参数 {key} 应为非负整数"))),
    }
}

/// 分发一个插件请求（`bridge.shutdown` 由外壳处理）。
pub async fn handle_request(
    service: &BridgeService,
    observer: Arc<dyn TurnObserver>,
    method: &str,
    params: &Value,
) -> Result<Value, RpcError> {
    let encode = |value: serde_json::Result<Value>| {
        value.map_err(|error| RpcError::new("internal", error.to_string()))
    };
    match method {
        "bridge.status" => encode(serde_json::to_value(service.status())),
        "bridge.configure" => {
            let tool_mode = params.get("tool_mode").and_then(Value::as_str);
            let tool_mode = service.configure(tool_mode).map_err(to_rpc)?;
            Ok(json!({ "tool_mode": tool_mode }))
        }
        "campaign.fork" => {
            let campaign_id = string_param(params, "campaign_id")?;
            let history_turns = usize_param(params, "history_turns")?
                .ok_or_else(|| RpcError::new("invalid_params", "缺少参数 history_turns"))?;
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let forked = service
                .fork_campaign(&campaign_id, history_turns, name)
                .await
                .map_err(to_rpc)?;
            encode(serde_json::to_value(forked))
        }
        "campaign.create" => {
            let card_path = PathBuf::from(string_param(params, "card_path")?);
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let created = service
                .create_campaign(card_path, name)
                .await
                .map_err(to_rpc)?;
            encode(serde_json::to_value(created))
        }
        "turn.run" => {
            let campaign_id = string_param(params, "campaign_id")?;
            let intent = string_param(params, "intent")?;
            let generation_mode = params
                .get("generation_mode")
                .filter(|value| !value.is_null())
                .map(|value| serde_json::from_value::<GenerationMode>(value.clone()))
                .transpose()
                .map_err(|error| {
                    RpcError::new("invalid_params", format!("generation_mode 无效: {error}"))
                })?;
            let history_turns = usize_param(params, "history_turns")?;
            let output = service
                .run_turn(
                    &campaign_id,
                    intent,
                    generation_mode,
                    history_turns,
                    observer,
                )
                .await
                .map_err(to_rpc)?;
            encode(serde_json::to_value(output))
        }
        "turn.cancel" => {
            let campaign_id = params.get("campaign_id").and_then(Value::as_str);
            Ok(json!({ "cancelled": service.cancel(campaign_id) }))
        }
        "conversation.display" => {
            let refs: Vec<DisplayRef> = match params.get("refs") {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
                    RpcError::new("invalid_params", format!("refs 无效: {error}"))
                })?,
            };
            let campaign_id = params.get("campaign_id").and_then(Value::as_str);
            let items = service.display_nodes(&refs, campaign_id).map_err(to_rpc)?;
            encode(serde_json::to_value(items).map(|items| json!({ "items": items })))
        }
        "card_shell.manifest" => {
            let campaign_id = string_param(params, "campaign_id")?;
            encode(serde_json::to_value(
                service.card_shell_manifest(&campaign_id).map_err(to_rpc)?,
            ))
        }
        "card_shell.variables_set" => {
            let campaign_id = string_param(params, "campaign_id")?;
            let variables = params
                .get("variables")
                .cloned()
                .ok_or_else(|| RpcError::new("invalid_params", "缺少参数 variables"))?;
            service
                .set_card_shell_variables(&campaign_id, variables)
                .await
                .map_err(to_rpc)?;
            Ok(json!({ "ok": true }))
        }
        "mvu.status" => {
            let campaign_id = string_param(params, "campaign_id")?;
            encode(serde_json::to_value(
                service.mvu_status(&campaign_id).map_err(to_rpc)?,
            ))
        }
        "mvu.analyze" => {
            let campaign_id = string_param(params, "campaign_id")?;
            let analyzed = service.analyze_mvu(&campaign_id).await.map_err(to_rpc)?;
            encode(serde_json::to_value(analyzed))
        }
        "mvu.apply" => {
            let campaign_id = string_param(params, "campaign_id")?;
            let status = service.apply_mvu(&campaign_id).await.map_err(to_rpc)?;
            encode(serde_json::to_value(status))
        }
        "variables.set" => {
            let campaign_id = string_param(params, "campaign_id")?;
            let writes: Vec<VariableWrite> = match params.get("writes") {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
                    RpcError::new("invalid_params", format!("writes 无效: {error}"))
                })?,
            };
            let written = service
                .write_variables(&campaign_id, &writes)
                .await
                .map_err(to_rpc)?;
            encode(serde_json::to_value(written))
        }
        other => Err(RpcError::new("unknown_method", format!("未知方法 {other}"))),
    }
}

/// stdio 外壳：stdin 读、stdout 写，直到插件断开或要求退出。
pub async fn run_stdio(data_dir: PathBuf) -> Result<(), String> {
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(line) = out_rx.recv().await {
            if stdout.write_all(line.as_bytes()).await.is_err()
                || stdout.write_all(b"\n").await.is_err()
                || stdout.flush().await.is_err()
            {
                break;
            }
        }
    });
    let peer = Arc::new(Peer::new(out_tx));
    let llm = Arc::new(LlmBridgeState::default());

    let app = match build_app(&data_dir, peer.clone(), llm.clone()) {
        Ok(app) => app,
        Err(message) => {
            peer.notify("bridge.fatal", json!({ "message": message }));
            drop(peer);
            let _ = writer.await;
            return Err(message);
        }
    };
    let service = Arc::new(BridgeService::new(app, data_dir.clone(), llm));
    let observer: Arc<dyn TurnObserver> = Arc::new(PeerObserver::new(peer.clone()));
    peer.notify(
        "bridge.ready",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "pid": std::process::id(),
            "data_dir": data_dir.display().to_string(),
        }),
    );
    tracing::info!("bridge ready, data_dir={}", data_dir.display());

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                tracing::error!("读取 stdin 失败: {error}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match peer.accept_line(&line) {
            Incoming::Request { id, method, params } => {
                if method == "bridge.shutdown" {
                    peer.respond(id, Ok(json!({})));
                    break;
                }
                let service = service.clone();
                let observer = observer.clone();
                let peer = peer.clone();
                tokio::spawn(async move {
                    let result = handle_request(&service, observer, &method, &params).await;
                    peer.respond(id, result);
                });
            }
            Incoming::Notification { method, params } => {
                tracing::debug!("忽略通知 {method}: {params}");
            }
            Incoming::Routed => {}
            Incoming::Invalid(reason) => tracing::warn!("无效消息: {reason}"),
        }
    }

    // 插件断开或要求退出：中止在跑的写作，挂起的模型调用一律失败。
    tracing::info!("bridge shutting down");
    service.cancel_all();
    peer.fail_all();
    drop(observer);
    drop(service);
    drop(peer);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), writer).await;
    Ok(())
}
