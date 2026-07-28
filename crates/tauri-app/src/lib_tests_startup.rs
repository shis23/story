use super::*;

pub(super) fn env_view<'a>(
    pairs: &'a [(&'a str, &'a str)],
) -> impl Fn(&str) -> Option<String> + 'a {
    move |name: &str| {
        pairs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.to_string())
    }
}

#[test]
fn resolve_app_data_dir_returns_os_standard_paths_per_platform() {
    // Windows：%APPDATA%/StoryForge
    let win = resolve_app_data_dir(
        AppDataDirTarget::Windows,
        &env_view(&[("APPDATA", "C:\\Users\\test\\AppData\\Roaming")]),
        None,
        None,
    )
    .expect("Windows path should resolve");
    assert_eq!(
        win,
        PathBuf::from("C:\\Users\\test\\AppData\\Roaming").join("StoryForge")
    );

    // macOS：$HOME/Library/Application Support/StoryForge
    let mac = resolve_app_data_dir(
        AppDataDirTarget::Macos,
        &env_view(&[("HOME", "/Users/test")]),
        None,
        None,
    )
    .expect("macOS path should resolve");
    assert_eq!(
        mac,
        PathBuf::from("/Users/test/Library/Application Support/StoryForge")
    );

    // Linux：$XDG_DATA_HOME 优先
    let lin_xdg = resolve_app_data_dir(
        AppDataDirTarget::Linux,
        &env_view(&[("XDG_DATA_HOME", "/custom/xdg"), ("HOME", "/home/test")]),
        None,
        None,
    )
    .expect("Linux XDG path should resolve");
    assert_eq!(lin_xdg, PathBuf::from("/custom/xdg/storyforge"));

    // Linux：无 XDG 时回退 $HOME/.local/share/storyforge
    let lin_home = resolve_app_data_dir(
        AppDataDirTarget::Linux,
        &env_view(&[("HOME", "/home/test")]),
        None,
        None,
    )
    .expect("Linux HOME path should resolve");
    assert_eq!(
        lin_home,
        PathBuf::from("/home/test/.local/share/storyforge")
    );
}

/// AND-2 Android 路径解析契约：
/// - `STORYFORGE_DATA_DIR` 覆盖优先（真机冒烟/调试）；
/// - 否则精确使用 Tauri/Android Context 返回的数据目录；
/// - 框架目录尚不可用时失败关闭，绝不猜测 `/data/data/...`。

#[test]
fn resolve_app_data_dir_android_prefers_override_then_framework_and_never_hardcodes() {
    let with_override = resolve_app_data_dir(
        AppDataDirTarget::Android,
        &env_view(&[("STORYFORGE_DATA_DIR", "/tmp/custom-sf-data")]),
        None,
        Some(Path::new("/data/user/10/com.storyforge.app")),
    )
    .expect("explicit override should resolve");
    assert_eq!(with_override, PathBuf::from("/tmp/custom-sf-data"));

    let framework = resolve_app_data_dir(
        AppDataDirTarget::Android,
        &env_view(&[]),
        None,
        Some(Path::new("/data/user/10/com.storyforge.app")),
    )
    .expect("framework data directory should resolve");
    assert_eq!(
        framework,
        PathBuf::from("/data/user/10/com.storyforge.app"),
        "必须保留框架返回的多用户数据路径"
    );

    let missing = resolve_app_data_dir(AppDataDirTarget::Android, &env_view(&[]), None, None);
    assert!(missing.is_err(), "框架目录不可用时必须失败关闭");
}

/// AND-2 exe_dir/data 回退契约：所有 OS 标准环境变量缺失时，回退到
/// exe 父目录下的 `data`（旧行为兼容）。用纯函数 + 合成路径验证，
/// 不依赖真实 exe。

#[test]
fn resolve_app_data_dir_falls_back_to_exe_data_when_env_missing() {
    let tmp =
        std::env::temp_dir().join(format!("sf-resolve-exe-fallback-{}", uuid::Uuid::new_v4()));
    let exe_parent = tmp.join("install");
    let resolved = resolve_app_data_dir(
        AppDataDirTarget::Linux,
        &env_view(&[]), // 无 HOME/XDG
        Some(&exe_parent),
        None,
    )
    .expect("desktop fallback should resolve");
    assert_eq!(resolved, exe_parent.join("data"));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// 迁移契约：旧 exe_dir/data 有数据、新目录为空时，迁移过去；
/// 新目录已有数据时不迁移（不覆盖用户数据）。用临时目录验证，
/// 不依赖真实 exe 或真实数据目录。

#[test]
fn migrate_from_exe_dir_copies_when_new_dir_empty_and_skips_when_populated() {
    let root = std::env::temp_dir().join(format!("sf-migrate-contract-{}", uuid::Uuid::new_v4()));
    let exe_parent = root.join("install");
    let old_dir = exe_parent.join("data");
    let new_dir = root.join("newdata");
    std::fs::create_dir_all(&old_dir).expect("create old dir");
    std::fs::create_dir_all(&new_dir).expect("create new dir");
    std::fs::write(old_dir.join("characters.json"), b"{}").expect("seed old data");

    // 新目录为空 → 迁移
    migrate_from_exe_dir_if_needed(&new_dir, Some(&exe_parent));
    assert!(
        new_dir.join("characters.json").exists(),
        "新目录为空时应迁移旧数据"
    );

    // 再次迁移：新目录已有数据 → 不动（幂等，不覆盖）
    std::fs::write(old_dir.join("characters.json"), b"\"stale\"").expect("rewrite old data");
    migrate_from_exe_dir_if_needed(&new_dir, Some(&exe_parent));
    let migrated = std::fs::read_to_string(new_dir.join("characters.json")).unwrap();
    assert_eq!(migrated, "{}", "新目录已有数据时不得覆盖（迁移必须跳过）");

    let _ = std::fs::remove_dir_all(&root);
}

/// 迁移契约：旧目录不存在时不报错（fresh install 无旧数据）。

#[test]
fn migrate_from_exe_dir_is_noop_when_old_dir_absent() {
    let root = std::env::temp_dir().join(format!("sf-migrate-noop-{}", uuid::Uuid::new_v4()));
    let exe_parent = root.join("install"); // 无 data 子目录
    let new_dir = root.join("newdata");
    std::fs::create_dir_all(&new_dir).expect("create new dir");
    // 不应 panic
    migrate_from_exe_dir_if_needed(&new_dir, Some(&exe_parent));
    assert!(new_dir.exists());
    let _ = std::fs::remove_dir_all(&root);
}

use serde::ser::Error as _;
use std::sync::{Arc, Mutex};

pub(super) struct RecordingMockLlm {
    responses: Mutex<std::collections::VecDeque<storyforge_domain::llm::ChatResponse>>,
    requests: Mutex<Vec<storyforge_domain::llm::ChatRequest>>,
}

impl RecordingMockLlm {
    pub(super) fn new(responses: Vec<storyforge_domain::llm::ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<storyforge_domain::llm::ChatRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub(super) fn clear_requests(&self) {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    pub(super) fn next_response(&self) -> storyforge_domain::llm::ChatResponse {
        self.responses
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front()
            .unwrap_or_else(|| storyforge_domain::llm::ChatResponse {
                content: "recording mock fallback response".into(),
                reasoning_content: Some("recording mock fallback reasoning".into()),
                tool_calls: vec![],
                finish_reason: Some("stop".into()),
                usage: None,
            })
    }
}

#[async_trait::async_trait]
impl LlmClient for RecordingMockLlm {
    async fn chat(
        &self,
        req: &storyforge_domain::llm::ChatRequest,
    ) -> Result<storyforge_domain::llm::ChatResponse, storyforge_domain::llm::LlmError> {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(req.clone());
        Ok(self.next_response())
    }

    async fn chat_stream(
        &self,
        req: &storyforge_domain::llm::ChatRequest,
        tx: tokio::sync::mpsc::UnboundedSender<storyforge_domain::llm::StreamChunk>,
        _cancel: watch::Receiver<bool>,
    ) -> Result<storyforge_domain::llm::ChatResponse, storyforge_domain::llm::LlmError> {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(req.clone());
        let response = self.next_response();
        let _ = tx.send(storyforge_domain::llm::StreamChunk {
            delta_content: Some(response.content.clone()),
            delta_reasoning_content: response.reasoning_content.clone(),
            delta_tool_calls: if response.tool_calls.is_empty() {
                None
            } else {
                Some(response.tool_calls.clone())
            },
            finish_reason: Some("stop".into()),
        });
        Ok(response)
    }
}

pub(super) fn mock_chat_response(
    content: impl Into<String>,
) -> storyforge_domain::llm::ChatResponse {
    storyforge_domain::llm::ChatResponse {
        content: content.into(),
        reasoning_content: Some("recording mock reasoning".into()),
        tool_calls: vec![],
        finish_reason: Some("stop".into()),
        usage: None,
    }
}

pub(super) fn command_prompt_hook_channel(
    state: Arc<AppState>,
    marker: &'static str,
) -> tauri::ipc::Channel<WritingEvent> {
    tauri::ipc::Channel::new(move |body| {
        let event = body.deserialize::<WritingEvent>()?;
        if event.event_type == "prompt_hook_request" {
            let request_id = event
                .data
                .get("request_id")
                .and_then(|value| value.as_str())
                .expect("prompt hook request should include request_id");
            let mut messages: Vec<ChatMessage> =
                serde_json::from_value(event.data["messages"].clone())
                    .expect("prompt hook request should include messages");
            messages.push(ChatMessage::user(marker));
            assert!(
                resolve_prompt_hook_pending(
                    &state.prompt_hook_pending,
                    request_id,
                    Some(messages),
                    None,
                ),
                "pending prompt hook sender should be registered"
            );
        }
        Ok(())
    })
}

pub(super) fn state_with_recording_llm(llm: Arc<RecordingMockLlm>) -> Arc<AppState> {
    let state = AppState::new_for_test();
    *state.active_llm.lock().unwrap_or_else(|p| p.into_inner()) = Some(llm as Arc<dyn LlmClient>);
    let state = Arc::new(state);
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.characters
            .push(Arc::new(make_test_character("Seraphina")));
    }
    state
}

pub(super) fn tauri_state_for_test(state: &Arc<AppState>) -> tauri::State<'_, Arc<AppState>> {
    // Tauri State has no public constructor; command-level tests need the
    // same wrapper type that invoke would provide around managed Arc state.
    unsafe { std::mem::transmute::<&Arc<AppState>, tauri::State<'_, Arc<AppState>>>(state) }
}

pub(super) fn plan_response_json() -> String {
    serde_json::json!({
        "scene_brief": "A compact command prompt hook test scene.",
        "subagent_tasks": [
            {
                "character_id": "Seraphina",
                "brief": "Perform a short beat.",
                "context_package": {
                    "character_brief": "Seraphina, concise test character.",
                    "scene_brief": "A compact command prompt hook test scene.",
                    "relevant_lore": [],
                    "constant_lore": [],
                    "recent_window": [],
                    "task": "Perform a short beat."
                }
            }
        ]
    })
    .to_string()
}

pub(super) fn any_recorded_request_contains_marker(llm: &RecordingMockLlm, marker: &str) -> bool {
    llm.requests()
        .iter()
        .any(|req| req.messages.iter().any(|message| message.content == marker))
}

pub(super) struct FailingSerialize;

impl Serialize for FailingSerialize {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(S::Error::custom("forced serialization failure"))
    }
}
#[test]
fn test_to_json_value_returns_structured_error_on_serialization_failure() {
    let err = to_json_value(&FailingSerialize, "failing dto").unwrap_err();

    match err {
        TauriCommandError::Internal { message } => {
            assert!(message.contains("failing dto"));
            assert!(message.contains("forced serialization failure"));
        }
        other => panic!("expected internal serialization error, got {other:?}"),
    }
}
#[test]
fn test_missing_active_llm_fails_closed() {
    let state = AppState::new_for_test();
    state.clear_active_connection();
    assert!(state.active_conn_id().is_none());

    match state.require_active_llm() {
        Err(TauriCommandError::Llm { message, retryable }) => {
            assert!(message.contains("未配置"));
            assert!(!retryable);
        }
        Ok(_) => panic!("missing connection must not return a fallback LLM client"),
        Err(other) => panic!("expected LLM configuration error, got {other:?}"),
    }
}

/// 无连接时角色识别应安全降级为源卡自身，不能写入 Mock 的示例人物。
#[test]
fn test_no_connection_extraction_uses_source_character_only() {
    let character = make_test_character("Seraphina");

    let (definitions, status, message) = fallback_character_extraction(&character, &[]);

    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].name, "Seraphina");
    assert!(
        definitions
            .iter()
            .all(|definition| { definition.name != "林医生" && definition.name != "陈警官" })
    );
    assert_eq!(
        status,
        storyforge_domain::character::CharacterExtractionStatus::Fallback
    );
    assert_eq!(message.as_deref(), Some(FALLBACK_EXTRACTION_MESSAGE));
}

/// 验证协议字符串解析
#[test]
fn test_parse_protocol() {
    assert_eq!(parse_protocol("openai").unwrap(), LlmProtocol::OpenAi);
    assert_eq!(parse_protocol("anthropic").unwrap(), LlmProtocol::Anthropic);
    assert_eq!(parse_protocol("gemini").unwrap(), LlmProtocol::Gemini);
    match parse_protocol("custom:myapi").unwrap() {
        LlmProtocol::Custom(s) => assert_eq!(s, "myapi"),
        other => panic!("应是 Custom，实际: {other:?}"),
    }
    assert!(parse_protocol("unknown").is_err());
}

/// 验证 tool_mode 字符串解析
#[test]
fn test_parse_tool_mode() {
    assert_eq!(parse_tool_mode("native").unwrap(), ToolMode::Native);
    assert_eq!(
        parse_tool_mode("text_fallback").unwrap(),
        ToolMode::TextFallback
    );
    assert!(parse_tool_mode("unknown").is_err());
}

/// 验证 AppState 的 vector_store 字段初始化正常（可读写）
#[test]
fn test_vector_store_initialized() {
    let state = AppState::new_for_test();
    let initial = state.vector_store.count();

    // 写入一条测试记录，验证可检索
    let _ = state.vector_store.upsert(VectorRecord {
        id: Id::from_str("test-init-vs"),
        content: "测试内容".into(),
        vector: vec![],
        keywords: vec!["测试".into()],
        kind: VectorKind::WorldInfo,
        metadata: std::collections::HashMap::new(),
    });
    assert_eq!(state.vector_store.count(), initial + 1);
    let hits = state
        .vector_store
        .search_by_keywords(&["测试".into()], 10)
        .unwrap();
    assert!(hits.iter().any(|h| h.content == "测试内容"));

    // 清理测试记录
    let _ = state.vector_store.delete(&Id::from_str("test-init-vs"));
}

/// 验证 vector_store 持久化：写入后重载能查到
#[test]
fn test_vector_store_persistence() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_vs_persist_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vectors.json");

    {
        let store = BruteForceStore::with_persistence(path.clone());
        let _ = store.upsert(VectorRecord {
            id: Id::from_str("v1"),
            content: "持久化测试".into(),
            vector: vec![],
            keywords: vec!["持久".into()],
            kind: VectorKind::WorldInfo,
            metadata: std::collections::HashMap::new(),
        });
    }

    // 重载
    {
        let store = BruteForceStore::with_persistence(path.clone());
        assert_eq!(store.count(), 1);
        let hits = store.search_by_keywords(&["持久".into()], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].content, "持久化测试");
    }

    let _ = std::fs::remove_dir_all(&dir);
}
