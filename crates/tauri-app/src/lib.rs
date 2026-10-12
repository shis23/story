mod card_shell_cache;
pub mod card_studio_api;
mod commands;
mod mvu_webview_runtime;
mod shell_doc_protocol;

// 宿主无关的应用服务层（AppState、存储、工作流、命令辅助）已抽到
// `storyforge-app-service`；根级 re-export 让 `crate::Xxx` 与外部
// `storyforge_lib::Xxx` 路径保持不变。
pub use storyforge_app_service::*;

use commands::{
    campaigns::*, card_shell::*, characters::*, connections::*, conversations::*, diagnostics::*,
    import_export::*, memory::*, meta::*, meta_typed::*, mvu::*, plugins::*, presets::*,
    profiles::*, turns::*, variables::*, world_info::*, writing::*,
};
// Stable crate-root surface used by deterministic/real-LLM harnesses. Keep the
// command module private while exposing only the shared postprocess helpers.
/// Gate 4: backend-neutral Meta snapshot DTO (typed patch propose/preview).
pub use commands::meta_typed::MetaSnapshot;
pub use commands::writing::{
    is_postprocess_instance_present, normalize_knowledge_update_for_postprocess,
};
// Gate 4 五审 P2：独立 SQLite 命令测试二进制（tests/sqlite_command_lifecycle.rs）
// 需要以真实 SQLite AppState 驱动真实命令。命令函数本体已提升为 `pub`（签名不变），
// 这里仅向 crate 外暴露这两个命令名——无任何 backend 分派逻辑（gate3 源码级测试
// 仍锚定 tauri command 属性标记的原函数，分派全部经 AppState::storage() facade）。
pub use commands::campaigns::set_active_campaign;
pub use commands::campaigns::set_active_campaign_in_state;
pub use commands::characters::delete_character;
pub use commands::import_export::{CampaignBundle, export_campaign_st_cards};
pub use commands::meta::meta_accept_patch;
pub use commands::meta_typed::{meta_accept_typed_patch, meta_propose_campaign_repairs};
// Gate 5：后端等价套件（tests/backend_parity_suite.rs）以真实命令驱动 JSON 与
// SQLite 两套 AppState。命令函数本体提升为 `pub`（签名不变），此处仅暴露命令
// 名——无任何 backend 分派逻辑。
pub use commands::campaigns::{
    add_campaign_instance, apply_campaign_opening, create_campaign, delete_campaign,
    extract_characters, fork_campaign,
};
pub use commands::cards::{delete_card, get_card, list_cards};
pub use commands::memory::{abandon_task, complete_task, create_task};
pub use commands::variables::{
    add_campaign_variable, promote_temporary_instance, set_campaign_variable,
    set_character_variable, sync_campaign_variable_schema,
};
#[cfg(test)]
use commands::cards::*;
#[cfg(test)]
use playthrough_lifecycle::delete_campaign_playthrough_in_store;
#[cfg(test)]
use storyforge_domain::conversation::Conversation;
// 根测试模块（lib_tests*.rs，`use super::*`）沿用的导入：生产代码抽到
// storyforge-app-service 后这些名字只剩测试在用。
#[cfg(test)]
use {
    connection_store::ConnectionStore,
    preset_store::PresetStore,
    std::path::Path,
    std::sync::RwLock,
    storyforge_app_agent::ToolContext,
    storyforge_app_conversation::ConversationStore,
    storyforge_infra_llm::LlmClient,
    storyforge_infra_util::secret_store::{SecretStore, is_secret_ref},
    storyforge_infra_vector::{BruteForceStore, VectorKind, VectorRecord, VectorStore},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::watch;

use storyforge_app_logging::{ExportOptions, LogFilter, LogKind, LogLevel};
use storyforge_app_meta::MvuApplyPreview;
use storyforge_app_pipeline::WritingContext;
use storyforge_domain::Id;
use storyforge_domain::agent::PipelineEvent;
use storyforge_domain::conversation::Provenance;
#[cfg(test)]
use storyforge_domain::conversation::Role as ConversationRole;
#[cfg(test)]
use storyforge_domain::conversation::VariantStatus;
use storyforge_domain::llm::{
    ChatMessage, LlmConnection, LlmConnectionSummary, LlmProtocol, SamplingParams, ToolMode,
};
use storyforge_domain::prompt_module::PromptProfile;
use storyforge_infra_plugin_host::mvu_runtime::MvuExecuteResponse;
use storyforge_infra_util::secret_store::SystemSecretStore;
use tauri::Manager;

use crate::error::TauriCommandError;
use crate::mvu_webview_runtime::{MvuPendingMap, WebViewMvuRuntime, new_mvu_pending_map};
pub use services::writing_regenerate::{RegenerateRequestDto, RegenerateTargetDto};

// On Android, the ndk-context needed by android_native_keyring_store is
// initialized asynchronously from the webview's JNI thread (see setup hook).
// The JNI callback sets this flag once ndk-context is ready; a setup-spawned
// task polls it and then retries the plaintext→SecretRef migration on a normal
// thread, after APP_DATA_DIR and the global ConnectionStore exist. We cannot
// call get_conn_store() from the JNI callback itself because that callback can
// run during webview bring-up, before setup has populated APP_DATA_DIR
// (get_app_data_dir() would panic).
#[cfg(target_os = "android")]
static NDK_CONTEXT_READY: std::sync::OnceLock<()> = std::sync::OnceLock::new();

fn get_card_shell_cache() -> &'static card_shell_cache::CardShellCache {
    use std::sync::OnceLock;
    static CACHE: OnceLock<card_shell_cache::CardShellCache> = OnceLock::new();
    CACHE.get_or_init(|| card_shell_cache::CardShellCache::new(&get_app_data_dir()))
}

/// Android-only: read the `debug.storyforge.storage_backend` system property
/// via the libc `__system_property_get` FFI (no extra crate). Returns the value
/// only if it is a recognized backend token (`json`/`sqlite`); anything else
/// (including unset) yields `None`, so this never perturbs production behavior.
/// Used solely to feed the same `STORYFORGE_STORAGE_BACKEND` env var the
/// production resolver reads, because `am start` cannot inherit a caller env.
#[cfg(target_os = "android")]
fn read_debug_storage_backend_property() -> Option<String> {
    unsafe extern "C" {
        fn __system_property_get(
            name: *const std::os::raw::c_char,
            value: *mut std::os::raw::c_char,
        ) -> std::os::raw::c_int;
    }
    let key = b"debug.storyforge.storage_backend\0";
    // 32 bytes is more than enough for "json"/"sqlite"; the recognized tokens
    // are validated below, so anything longer is rejected anyway.
    let mut buf: [std::os::raw::c_char; 32] = [0; 32];
    // SAFETY: `key` is a NUL-terminated C string; `buf` is a 32-byte buffer.
    // __system_property_get writes the property value + NUL and returns its
    // length (≤ PROP_VALUE_MAX). We only read `n` bytes back out.
    let n = unsafe { __system_property_get(key.as_ptr() as *const _, buf.as_mut_ptr()) };
    if n <= 0 {
        return None;
    }
    let bytes: Vec<u8> = buf[..n as usize].iter().map(|&b| b as u8).collect();
    let val = String::from_utf8(bytes).ok()?;
    match val.trim() {
        "json" | "sqlite" => Some(val.trim().to_string()),
        _ => None,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // W8 MVU JS Runtime：共享 pending map
    let mvu_pending: MvuPendingMap = new_mvu_pending_map();

    tauri::Builder::default()
        // P0-2/M-01: appended to the invoke initialization script so it runs
        // in every frame right after the IPC bootstrap. wry on Windows
        // injects init scripts into subframes, and the runtime PoC
        // (2026-09-28) confirmed a storyforge-shell iframe could invoke app
        // commands; this guard neuters IPC in subframes only. See
        // shell_doc_protocol::SUBFRAME_IPC_GUARD docs for the mechanism.
        .append_invoke_initialization_script(shell_doc_protocol::SUBFRAME_IPC_GUARD)
        .register_uri_scheme_protocol(card_shell_cache::LOCAL_PROTOCOL_SCHEME, |_ctx, request| {
            card_shell_cache_protocol_response(request)
        })
        // V5 CSP isolation: serve shell documents (CardShell/TavernHelper/MVU/
        // PluginHost iframes) on a dedicated origin so they do NOT inherit the
        // main app's policy container. See shell_doc_protocol.rs. Android line
        // of ownership: this is an additive registration on the shared Builder
        // chain; it does not touch Android picker/data-dir logic.
        .register_uri_scheme_protocol(shell_doc_protocol::SHELL_DOC_SCHEME, |_ctx, request| {
            shell_doc_protocol::shell_doc_protocol_response(request)
        })
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(mvu_pending.clone())
        .setup(move |app| {
            // 服务层的后台任务（向量补建等）派发到 tauri 自己的运行时，
            // 与抽出前直接调 `tauri::async_runtime::spawn` 同一运行时。
            background::install_background_runtime(tauri::async_runtime::handle().inner().clone());

            // Initialize ndk-context on Android so android-native-keyring-store
            // (used by SystemSecretStore for API-key persistence) can access the
            // Android Keystore. The closure runs asynchronously on the webview's
            // main-thread event loop, *after* AppState::new_with_backend below
            // has already constructed ConnectionStore synchronously. That first
            // migration attempt fail-closes to plaintext (catch_unwind in
            // secret_store.rs turns the Store::new() panic into an Err, and
            // ensure_native_store does not cache the Err). Once this closure
            // runs, ndk-context is ready, so we trigger retry_migration() to
            // actually move any plaintext API keys into the Keystore.
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.with_webview(|webview| {
                        webview.jni_handle().exec(|env, activity, _webview| {
                            static CTX_REF: std::sync::OnceLock<Option<jni::objects::GlobalRef>> =
                                std::sync::OnceLock::new();
                            CTX_REF.get_or_init(|| {
                                let global = env.new_global_ref(activity).ok()?;
                                let vm = env.get_java_vm().ok()?;
                                let vm_ptr = vm.get_java_vm_pointer() as *mut std::ffi::c_void;
                                let ctx_ptr = global.as_obj().as_raw() as *mut std::ffi::c_void;
                                // SAFETY: vm_ptr is a valid JavaVM* and ctx_ptr is a
                                // global-ref to the Activity Context. Both outlive the
                                // process (CTX_REF is a static OnceLock, never dropped).
                                unsafe {
                                    ndk_context::initialize_android_context(vm_ptr, ctx_ptr);
                                }
                                Some(global)
                            });
                            // Signal that ndk-context is ready. The retry task spawned
                            // in setup polls this flag and runs the plaintext→SecretRef
                            // migration on a normal thread (not this JNI thread), where
                            // it can safely call get_conn_store() once APP_DATA_DIR is
                            // populated. We must NOT touch get_conn_store() here: this
                            // callback can run during webview bring-up, before setup has
                            // set APP_DATA_DIR, and get_app_data_dir() would panic.
                            let _ = NDK_CONTEXT_READY.set(());
                        });
                    });
                }
            }
            // Android's private data path is provided by the running Activity.
            // Resolve it here, never from a hardcoded /data/data alias. Desktop
            // keeps its existing StoryForge locations for data compatibility.
            #[cfg(target_os = "android")]
            let framework_data_dir = Some(
                app.path()
                    .app_data_dir()
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            );
            #[cfg(not(target_os = "android"))]
            let framework_data_dir: Option<PathBuf> = None;

            let data_dir = initialize_app_data_dir(framework_data_dir.as_deref())
                .map_err(std::io::Error::other)?;

            // Resolve storage backend before any store recovery. SQLite is the
            // default (Gate 7); JSON is retained as an explicit fallback
            // (STORYFORGE_STORAGE_BACKEND=json). SQLite startup activates a
            // fail-closed, process-owned production boundary (no dual-write);
            // a legacy JSON tree is migrated in place, a fresh directory is
            // initialized as an empty SQLite authority.
            //
            // Android-only debug bridge: the launch environment cannot be set
            // from adb (am start does not inherit the caller's env), so to allow
            // on-device field verification of the JSON→SQLite cutover and the
            // explicit fallback we also honor the system property
            // `debug.storyforge.storage_backend` (json|sqlite). It only feeds the
            // same env var the production code already reads; it adds no new
            // authority path and is a no-op on release builds where the property
            // is unset. Set with: adb shell setprop debug.storyforge.storage_backend sqlite
            #[cfg(target_os = "android")]
            {
                if std::env::var("STORYFORGE_STORAGE_BACKEND").is_err() {
                    if let Some(val) = read_debug_storage_backend_property() {
                        // SAFETY: single-threaded setup before any store reads the
                        // env var; no concurrent access exists yet.
                        unsafe { std::env::set_var("STORYFORGE_STORAGE_BACKEND", val) };
                    }
                }
            }
            let resolution = storage_backend::resolve_backend(&data_dir)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            tracing::info!(
                backend = resolution.diagnostics.backend,
                source = resolution.diagnostics.source,
                schema_version = ?resolution.diagnostics.schema_version,
                cutover = resolution.cutover_performed,
                "storage backend resolved"
            );
            let storage = Arc::new(storage_backend::StorageFacade::new(
                data_dir.clone(),
                resolution.pinned.clone(),
            ));
            if let Some(db_path) = resolution.db_path.as_deref() {
                sqlite_runtime::activate(db_path)
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
            }

            // AppState and every process-owned store now share the setup-resolved
            // directory. Only after construction can tracing write into LogStore.
            let app_state = Arc::new(
                AppState::new_with_backend(data_dir, storage).map_err(std::io::Error::other)?,
            );
            storyforge_app_logging::init_tracing(app_state.log_store.clone());
            app.manage(app_state.clone());

            // Eagerly initialize the global ConnectionStore OnceLock so that any
            // later get_conn_store() call (including the ndk-context retry task
            // spawned below) reuses this instance instead of lazily constructing
            // one through get_app_data_dir(). At this point APP_DATA_DIR is set
            // and the data dir exists, so construction is safe.
            #[cfg(target_os = "android")]
            {
                let _ = get_conn_store();
                // Spawn a task that waits for the ndk-context-ready flag (set from
                // the webview JNI callback) and then retries the plaintext→SecretRef
                // migration. The first migration attempt ran inside
                // AppState::new_with_backend before ndk-context was ready and fail-
                // closed to plaintext; ensure_native_store no longer caches that
                // Err, so this retry actually moves seeded keys into the Keystore.
                std::thread::spawn(move || {
                    // Poll (bounded) for the JNI callback to set the flag.
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    while NDK_CONTEXT_READY.get().is_none() {
                        if std::time::Instant::now() > deadline {
                            tracing::warn!(
                                "ndk-context 就绪信号 10s 内未到达，跳过本次 keystore 迁移重试"
                            );
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    // APP_DATA_DIR is set and CONN_STORE is initialized (both done
                    // in setup, which has returned by the time this thread schedules).
                    get_conn_store().retry_migration();
                });
            }

            // 三审9：单一公共启动恢复入口（幂等重放 Committing 态 Turn + 标记非
            // terminal 活动 Turn + 补挂 preaccept 恢复 + 重放 ChronicleCompressor）。
            startup_recovery::run_startup_recovery(&app_state);

            // 派生向量索引对账（世界书按角色库重建、清已删局残留、补齐远记忆缺口）。
            // 只读写本地索引、不调嵌入 API；放后台线程，不拖慢窗口出现。
            {
                let app_state = app_state.clone();
                std::thread::spawn(move || vector_index::reconcile_on_startup(&app_state));
            }

            // W8: 创建 WebViewMvuRuntime，共享同一个 pending map
            let mvu_rt =
                WebViewMvuRuntime::with_shared_pending(app.handle().clone(), mvu_pending.clone());
            // W10: 存入全局 OnceLock，供 new_pipeline 注入到 PipelineOrchestrator
            let mvu_rt = Arc::new(mvu_rt);
            let _ = install_mvu_runtime(mvu_rt.clone());
            app.manage(mvu_rt);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // M0 角色卡命令
            import_character,
            list_characters,
            get_character,
            delete_character,
            // Card Studio Phase 1
            card_studio_api::cardstudio_list_projects,
            card_studio_api::cardstudio_create_project,
            card_studio_api::cardstudio_create_from_novel,
            card_studio_api::cardstudio_create_from_character,
            card_studio_api::cardstudio_prefill_from_novel,
            card_studio_api::cardstudio_get_project,
            card_studio_api::cardstudio_delete_project,
            card_studio_api::cardstudio_update_artifacts,
            card_studio_api::cardstudio_set_stage,
            card_studio_api::cardstudio_set_options,
            card_studio_api::cardstudio_run_checks,
            card_studio_api::cardstudio_run_review,
            card_studio_api::cardstudio_compile,
            card_studio_api::cardstudio_export_gate,
            card_studio_api::cardstudio_export_png,
            card_studio_api::cardstudio_complete_manual_stage,
            card_studio_api::cardstudio_run_stage,
            card_studio_api::cardstudio_import_compiled,
            card_studio_api::cardstudio_list_stages,
            update_world_info_route,
            update_world_info_entry,
            add_world_info_entry,
            delete_world_info_entry,
            meta_accept_patch,
            import_preset,
            list_presets,
            get_preset,
            get_active_preset,
            set_active_preset,
            delete_preset,
            update_preset_prompt,
            update_preset_regex,
            list_global_regex_scripts,
            import_global_regex_settings,
            clear_global_regex_scripts,
            update_global_regex,
            import_preset_as_modules,
            // M4 插件命令
            list_plugins,
            install_plugin,
            uninstall_plugin,
            set_plugin_enabled,
            plugin_list_characters,
            plugin_read_character,
            plugin_read_world_info,
            plugin_get_variable,
            plugin_set_variable,
            // 预设/模块系统命令
            list_modules,
            update_module,
            list_profiles,
            get_active_profile,
            save_profile,
            set_active_profile,
            list_agent_profile_configs,
            get_agent_profile_config,
            get_active_agent_profile_config,
            save_agent_profile_config,
            export_agent_profile_config,
            import_agent_profile_config,
            delete_agent_profile_config,
            set_active_agent_profile_config,
            get_version,
            // LLM 连接管理命令
            list_connection_templates,
            list_connections,
            get_active_connection,
            get_connection,
            create_connection,
            update_connection,
            delete_connection,
            set_active_connection,
            test_connection,
            list_models,
            // M1 写作命令
            start_writing,
            plugin_prompt_hook_result,
            cancel_writing,
            // M1 对话命令
            list_conversations,
            delete_conversation,
            get_conversation,
            plugin_get_conversation,
            // 重 roll 命令
            regenerate,
            // M1 对话操作命令
            edit_variant,
            accept_variant,
            soft_delete_variant,
            delete_message_from,
            add_variant,
            switch_variant,
            abandon_turn,
            // M1 日志命令
            log_query,
            log_get_llm_call,
            log_clear,
            log_export_bundle,
            log_append_frontend,
            configure_embedder,
            get_embed_config,
            clear_embed_config,
            get_memory_index_status,
            rebuild_memory_embeddings,
            archive_conversation,
            // P1：角色识别 / CharacterCard / Campaign / 角色实例 / 变量
            extract_characters,
            list_cards,
            get_card,
            delete_card,
            create_campaign,
            apply_campaign_opening,
            fork_campaign,
            list_campaigns,
            get_campaign,
            delete_campaign,
            set_active_campaign,
            get_active_campaign,
            list_instances,
            get_instance,
            add_campaign_instance,
            get_character_variables,
            set_character_variable,
            get_campaign_variables,
            get_campaign_variable_schema,
            add_campaign_variable,
            sync_campaign_variable_schema,
            set_campaign_variable,
            promote_temporary_instance,
            // P2 后处理产出查询 / 任务管理
            list_character_knowledge,
            list_tasks,
            create_task,
            complete_task,
            abandon_task,
            list_round_summaries,
            list_campaign_world_info,
            add_campaign_world_info_entry,
            update_campaign_world_info_entry,
            set_campaign_world_info_enabled,
            delete_campaign_world_info_entry,
            set_campaign_world_info_route,
            get_character_world_info,
            get_character_world_info_entry,
            get_campaign_world_info_entry,
            get_card_shell_manifest,
            get_card_shell_inline_js,
            card_shell_list_allowed_hosts,
            card_shell_register_doc,
            card_shell_register_module,
            card_shell_unregister_doc,
            card_shell_allow_host,
            card_shell_clear_cache,
            card_shell_fetch_url,
            get_active_turn_quality,
            get_active_turn_receipt,
            retry_active_turn_postprocess,
            // V4 存储健康：损坏启动拦截 + 恢复确认
            storage_health_report,
            storage_health_acknowledge,
            // P3 Meta Agent / MVU 五合一 / ST 预设分类
            meta_start_conversation,
            meta_chat,
            meta_get_conversation,
            meta_list_pending_patches,
            meta_dismiss_patch,
            meta_analyze_mvu_card,
            meta_list_mvu_translations,
            meta_get_mvu_translation,
            meta_preview_mvu_apply,
            meta_apply_mvu_schema,
            meta_classify_st_preset,
            meta_health_check,
            meta_explain_generation,
            // 第三轮：类型化 Patch 修复闭环
            meta_propose_campaign_repairs,
            meta_list_typed_patches,
            meta_preview_typed_patch,
            meta_accept_typed_patch,
            meta_dismiss_typed_patch,
            // W7 导出命令
            export_st_card_png,
            export_campaign_st_cards,
            export_campaign_bundle,
            import_campaign_bundle,
            // W8 MVU JS Runtime 命令
            mvu_unload_ack,
            mvu_load_ack,
            mvu_execute_result,
        ])
        .run(tauri::generate_context!())
        .expect("StoryForge 启动失败");
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
