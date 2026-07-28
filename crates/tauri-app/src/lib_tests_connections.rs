use super::import_export::MemorySecretStore;
use super::*;
use crate::commands::connections::configure_embedder_with_secret_store_async;

#[test]
fn test_embed_config_stores_secret_ref_not_plaintext() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_embed_secure_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let secret_store = MemorySecretStore::default();
    let config = storyforge_infra_llm::EmbedConfig {
        endpoint: "https://api.example.com/v1/embeddings".into(),
        api_key: "embed-secret".into(),
        model: "embed-model".into(),
        dim: 3,
    };

    persist_embed_config_secret_ref(&dir, &config, &secret_store).unwrap();
    let raw = std::fs::read_to_string(dir.join("embed.json")).unwrap();
    assert!(!raw.contains("embed-secret"));
    assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));

    let loaded = load_embed_config_with_secret_store(&dir, &secret_store).unwrap();
    assert_eq!(loaded.api_key, "embed-secret");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_embed_config_load_migrates_plaintext_key() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_embed_migrate_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let secret_store = MemorySecretStore::default();
    let config = storyforge_infra_llm::EmbedConfig {
        endpoint: "https://api.example.com/v1/embeddings".into(),
        api_key: "legacy-embed-secret".into(),
        model: "embed-model".into(),
        dim: 3,
    };
    storyforge_infra_util::atomic_write_json(&dir.join("embed.json"), &config).unwrap();

    let loaded = load_embed_config_with_secret_store(&dir, &secret_store).unwrap();
    assert_eq!(loaded.api_key, "legacy-embed-secret");
    let raw = std::fs::read_to_string(dir.join("embed.json")).unwrap();
    assert!(!raw.contains("legacy-embed-secret"));
    assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_embed_config_recovers_tmp_and_migrates_plaintext_key() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_embed_tmp_migrate_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let secret_store = MemorySecretStore::default();
    let api_key = ["s", "k", "-embed-recovered-secret"].concat();
    let config = storyforge_infra_llm::EmbedConfig {
        endpoint: "https://api.example.com/v1/embeddings".into(),
        api_key: api_key.clone(),
        model: "embed-model".into(),
        dim: 3,
    };
    let path = dir.join("embed.json");
    std::fs::write(&path, "{ invalid").unwrap();
    storyforge_infra_util::atomic_write_json(
        &PathBuf::from(format!("{}.tmp", path.display())),
        &config,
    )
    .unwrap();

    let loaded = load_embed_config_with_secret_store(&dir, &secret_store).unwrap();
    assert_eq!(loaded.api_key, api_key);

    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains(&api_key));
    assert!(!raw.contains("Authorization"));
    assert!(!raw.contains("Bearer"));
    assert!(!raw.contains("sk-"));
    assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));
    assert!(!path.with_extension("json.corrupt").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_configure_embedder_async_persists_secret_ref_and_updates_state() {
    let state = Arc::new(AppState::new_for_test());
    let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
    let config = storyforge_infra_llm::EmbedConfig {
        endpoint: "https://api.example.com/v1/embeddings".into(),
        api_key: "embed-async-secret".into(),
        model: "embed-model".into(),
        dim: 3,
    };

    configure_embedder_with_secret_store_async(state.clone(), config.clone(), secret_store.clone())
        .await
        .unwrap();

    let raw = std::fs::read_to_string(state.data_dir.join("embed.json")).unwrap();
    assert!(!raw.contains("embed-async-secret"));
    assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));

    let loaded =
        load_embed_config_with_secret_store(&state.data_dir, secret_store.as_ref()).unwrap();
    assert_eq!(loaded.api_key, "embed-async-secret");

    let in_memory = state
        .embed_config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap();
    assert_eq!(in_memory.endpoint, config.endpoint);
    assert_eq!(in_memory.api_key, "embed-async-secret");
    assert_eq!(in_memory.model, config.model);
    assert_eq!(in_memory.dim, config.dim);
}
#[tokio::test]
async fn test_set_active_connection_async_persists_and_updates_state() {
    let state = Arc::new(AppState::new_for_test());
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_conn_async_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let secret_store = Arc::new(MemorySecretStore::default());
    let store = Arc::new(ConnectionStore::new_with_secret_store(
        &dir,
        secret_store.clone(),
    ));
    store
        .save(make_test_llm_connection(
            "async-active",
            "async-active-secret",
        ))
        .unwrap();

    set_active_connection_with_store_async(state.clone(), store.clone(), "async-active".into())
        .await
        .unwrap();

    assert_eq!(state.active_conn_id().as_deref(), Some("async-active"));
    assert!(store.active_connection().is_some());

    let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
    assert!(!raw.contains("async-active-secret"));
    let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(stored["active_id"], "async-active");
    assert!(stored["connections"][0]["last_used_at"].is_string());

    let reloaded = ConnectionStore::new_with_secret_store(&dir, secret_store);
    assert_eq!(
        reloaded.active_connection().unwrap().id.as_str(),
        "async-active"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_set_active_connection_async_serializes_concurrent_updates() {
    let state = Arc::new(AppState::new_for_test());
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_conn_async_race_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let secret_store = Arc::new(MemorySecretStore::default());
    let store = Arc::new(ConnectionStore::new_with_secret_store(
        &dir,
        secret_store.clone(),
    ));
    store
        .save(make_test_llm_connection("async-a", "async-secret-a"))
        .unwrap();
    store
        .save(make_test_llm_connection("async-b", "async-secret-b"))
        .unwrap();

    let queued_guard = state.active_connection_update.lock().await;
    let task_a = tokio::spawn(set_active_connection_with_store_async(
        state.clone(),
        store.clone(),
        "async-a".into(),
    ));
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let task_b = tokio::spawn(set_active_connection_with_store_async(
        state.clone(),
        store.clone(),
        "async-b".into(),
    ));
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    drop(queued_guard);

    task_a.await.unwrap().unwrap();
    task_b.await.unwrap().unwrap();

    assert_eq!(state.active_conn_id().as_deref(), Some("async-b"));
    assert_eq!(store.active_connection().unwrap().id.as_str(), "async-b");

    let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
    let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(stored["active_id"], "async-b");

    let reloaded = ConnectionStore::new_with_secret_store(&dir, secret_store);
    assert_eq!(reloaded.active_connection().unwrap().id.as_str(), "async-b");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_create_connection_async_auto_activates_first_connection() {
    let state = Arc::new(AppState::new_for_test());
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_conn_async_create_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let secret_store = Arc::new(MemorySecretStore::default());
    let store = Arc::new(ConnectionStore::new_with_secret_store(
        &dir,
        secret_store.clone(),
    ));
    let conn = make_test_llm_connection("created-first", "created-first-secret");

    let conn_id = create_connection_with_store_async(state.clone(), store.clone(), conn)
        .await
        .unwrap();

    assert_eq!(conn_id, "created-first");
    assert_eq!(state.active_conn_id().as_deref(), Some("created-first"));

    let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
    assert!(!raw.contains("created-first-secret"));
    let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(stored["active_id"], "created-first");

    let reloaded = ConnectionStore::new_with_secret_store(&dir, secret_store);
    assert_eq!(
        reloaded.active_connection().unwrap().id.as_str(),
        "created-first"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_update_connection_async_keeps_key_and_refreshes_active() {
    let state = Arc::new(AppState::new_for_test());
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_conn_async_update_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let secret_store = Arc::new(MemorySecretStore::default());
    let store = Arc::new(ConnectionStore::new_with_secret_store(
        &dir,
        secret_store.clone(),
    ));
    let conn = make_test_llm_connection("edit-active", "edit-secret-old");
    create_connection_with_store_async(state.clone(), store.clone(), conn)
        .await
        .unwrap();
    assert_eq!(state.active_conn_id().as_deref(), Some("edit-active"));

    update_connection_with_store_async(
        state.clone(),
        store.clone(),
        UpdateConnectionDto {
            id: "edit-active".into(),
            name: "renamed-active".into(),
            base_url: "https://api.example.com/v1/chat/completions".into(),
            protocol: "openai".into(),
            model: "new-model".into(),
            api_key: String::new(), // 閻ｆ瑧鈹栨穱婵堟殌閸?key
            tool_mode: "native".into(),
            temperature: Some(0.7),
            top_p: Some(0.9),
            max_tokens: None,
            max_tokens_explicit: false,
            reasoning: Some("disabled".into()),
            extra: None,
        },
    )
    .await
    .unwrap();

    let stored = store.get("edit-active").unwrap();
    assert_eq!(stored.connection.name, "renamed-active");
    assert_eq!(stored.connection.model, "new-model");
    assert!(is_secret_ref(&stored.connection.api_key));
    assert_eq!(
        store.resolved("edit-active").unwrap().unwrap().api_key,
        "edit-secret-old"
    );
    assert_eq!(state.active_conn_id().as_deref(), Some("edit-active"));

    let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
    assert!(!raw.contains("edit-secret-old"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_update_connection_async_replaces_key_when_provided() {
    let state = Arc::new(AppState::new_for_test());
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_conn_async_update_key_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let secret_store = Arc::new(MemorySecretStore::default());
    let store = Arc::new(ConnectionStore::new_with_secret_store(
        &dir,
        secret_store.clone(),
    ));
    store
        .save(make_test_llm_connection("edit-key", "old-secret"))
        .unwrap();

    update_connection_with_store_async(
        state.clone(),
        store.clone(),
        UpdateConnectionDto {
            id: "edit-key".into(),
            name: "edit-key".into(),
            base_url: "https://api.example.com/v1/chat/completions".into(),
            protocol: "openai".into(),
            model: "test-model".into(),
            api_key: "new-secret".into(),
            tool_mode: "native".into(),
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_tokens_explicit: false,
            reasoning: None,
            extra: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        store.resolved("edit-key").unwrap().unwrap().api_key,
        "new-secret"
    );
    // 闂堢偞妞跨捄鍐╂纯閺傞绗夋惔鏃囶嚖鐠?active
    assert!(state.active_conn_id().is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_delete_connection_async_serializes_with_set_active() {
    let state = Arc::new(AppState::new_for_test());
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_conn_async_delete_race_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let secret_store = Arc::new(MemorySecretStore::default());
    let store = Arc::new(ConnectionStore::new_with_secret_store(
        &dir,
        secret_store.clone(),
    ));
    store
        .save(make_test_llm_connection("async-a", "async-secret-a"))
        .unwrap();
    store
        .save(make_test_llm_connection("async-b", "async-secret-b"))
        .unwrap();

    set_active_connection_with_store_async(state.clone(), store.clone(), "async-a".into())
        .await
        .unwrap();

    let queued_guard = state.active_connection_update.lock().await;
    let set_b = tokio::spawn(set_active_connection_with_store_async(
        state.clone(),
        store.clone(),
        "async-b".into(),
    ));
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let delete_b = tokio::spawn(delete_connection_with_store_async(
        state.clone(),
        store.clone(),
        "async-b".into(),
    ));
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    drop(queued_guard);

    set_b.await.unwrap().unwrap();
    delete_b.await.unwrap().unwrap();

    assert!(state.active_conn_id().is_none());
    assert!(store.get("async-b").is_none());
    assert!(store.active_connection().is_none());

    let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
    let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(stored["active_id"].is_null());

    let reloaded = ConnectionStore::new_with_secret_store(&dir, secret_store);
    assert!(reloaded.active_connection().is_none());

    let _ = std::fs::remove_dir_all(&dir);
}
