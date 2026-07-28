use super::*;

#[test]
fn test_diagnostic_context_summarizes_stores_without_secret_values() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_diag_context_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(dir.join("logs")).unwrap();
    storyforge_infra_util::atomic_write_json_str(
        &dir.join("connections.json"),
        r#"{"items":[{"api_key":"sk-live-secret"}]}"#,
    )
    .unwrap();
    storyforge_infra_util::atomic_write_json_str(
        &dir.join("embed.json"),
        r#"{"api_key":"embed-live-secret"}"#,
    )
    .unwrap();

    let context = diagnostic_context_for_data_dir(&dir);
    let json = serde_json::to_string(&context).unwrap();

    assert!(json.contains("connections.json"));
    assert!(json.contains("embed.json"));
    assert!(json.contains("active_preset.json"));
    assert!(json.contains("global_regex_scripts.json"));
    assert!(json.contains("logs"));
    assert!(!json.contains("sk-live-secret"));
    assert!(!json.contains("embed-live-secret"));
    assert_eq!(context["store_files"][0]["has_bytes"], true);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_diagnostic_export_bundle_summarizes_secret_stores_without_leaking_keys() {
    let state = AppState::new_for_test();
    let data_dir = state.data_dir.clone();
    let key_prefix = ["s", "k", "-"].concat();
    let connection_key = format!("{key_prefix}test-diagnostic-connection-secret");
    let embed_key = format!("{key_prefix}test-diagnostic-embed-secret");
    let connection_bearer = "diagnostic-connection-bearer-secret";
    let embed_bearer = "diagnostic-embed-bearer-secret";
    storyforge_infra_util::atomic_write_json_str(
        &data_dir.join("connections.json"),
        &format!(
            r#"{{
                "active_id": "test-openai",
                "items": [{{
                    "id": "test-openai",
                    "api_key": "{connection_key}",
                    "headers": {{
                        "Authorization": "Bearer {connection_bearer}"
                    }}
                }}]
            }}"#
        ),
    )
    .unwrap();
    storyforge_infra_util::atomic_write_json_str(
        &data_dir.join("embed.json"),
        &format!(
            r#"{{
                "endpoint": "https://example.invalid/embeddings",
                "api_key": "{embed_key}",
                "headers": {{
                    "Authorization": "Bearer {embed_bearer}"
                }}
            }}"#
        ),
    )
    .unwrap();
    state.log_store.push(storyforge_app_logging::LogEntry {
        id: Id::new(),
        kind: LogKind::Backend,
        level: LogLevel::Info,
        timestamp: Utc::now(),
        message: "diagnostic export marker".into(),
        fields: HashMap::new(),
        llm_detail: None,
    });

    let opts = ExportOptions {
        redact_content: true,
        ..Default::default()
    };
    let mut bundle = storyforge_app_logging::export_bundle(&state.log_store, &opts).unwrap();
    bundle.as_object_mut().unwrap().insert(
        "diagnostic_context".into(),
        diagnostic_context_for_data_dir(&data_dir),
    );
    let store_files = bundle["diagnostic_context"]["store_files"]
        .as_array()
        .unwrap();
    let connection_summary = store_files
        .iter()
        .find(|file| file["name"] == "connections.json")
        .unwrap();
    let embed_summary = store_files
        .iter()
        .find(|file| file["name"] == "embed.json")
        .unwrap();

    assert_eq!(bundle["counts"]["backend"], 1);
    assert_eq!(
        bundle["backend_logs"][0]["message"],
        "diagnostic export marker"
    );
    assert_eq!(connection_summary["exists"], true);
    assert_eq!(connection_summary["has_bytes"], true);
    assert_eq!(embed_summary["exists"], true);
    assert_eq!(embed_summary["has_bytes"], true);
    assert!(connection_summary.get("content").is_none());
    assert!(embed_summary.get("content").is_none());

    let json = serde_json::to_string(&bundle).unwrap();
    for needle in &[
        connection_key,
        embed_key,
        connection_bearer.into(),
        embed_bearer.into(),
        "Authorization".into(),
        "Bearer".into(),
        key_prefix,
        "api_key".into(),
    ] {
        assert!(!json.contains(needle), "bundle leaked {needle}");
    }

    let _ = std::fs::remove_dir_all(data_dir);
}
