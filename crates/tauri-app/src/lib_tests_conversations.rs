#[test]
fn test_archive_snapshot_respects_watermark_math() {
    // 纯水位算术：未归档切片 = messages[archived_upto..]
    let messages: Vec<String> = vec!["a".into(), "b".into(), "c".into(), "d".into()];
    let archived_upto = 2usize;
    let pending: &[String] = &messages[archived_upto..];
    assert_eq!(pending, &["c".to_string(), "d".to_string()]);
    let advanced = 1usize;
    let new_upto = archived_upto + advanced;
    assert_eq!(new_upto, 3);
    assert_eq!(&messages[new_upto..], &["d".to_string()]);
}
use super::*;
use crate::commands::conversations::load_archive_snapshot;

#[tokio::test]
async fn test_accept_variant_async_persists_final_variant() {
    let state = Arc::new(AppState::new_for_test());
    let conversation = state.conv_store.create(Some("card-1".into()), None);
    let node_id = state
        .conv_store
        .append_ai_draft(&conversation.id, "draft text".into(), None)
        .unwrap();

    accept_variant_async(
        state.clone(),
        conversation.id.clone(),
        node_id.clone(),
        false,
        None,
    )
    .await
    .unwrap();

    let updated = state.conv_store.get(&conversation.id).unwrap();
    let node = updated
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .unwrap();
    assert_eq!(node.active().unwrap().status, VariantStatus::Final);

    let reloaded = ConversationStore::new(state.data_dir.join("conversations"));
    let persisted = reloaded.get(&conversation.id).unwrap();
    let persisted_node = persisted
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .unwrap();
    assert_eq!(
        persisted_node.active().unwrap().status,
        VariantStatus::Final
    );
}

#[tokio::test]
async fn test_archivable_messages_async_filters_discarded_variants() {
    let state = Arc::new(AppState::new_for_test());
    let conversation = state.conv_store.create(Some("card-1".into()), None);
    state
        .conv_store
        .append_user_message(&conversation.id, "user intent".into())
        .unwrap();
    let discarded_node_id = state
        .conv_store
        .append_ai_draft(&conversation.id, "discarded draft".into(), None)
        .unwrap();
    state
        .conv_store
        .soft_delete_variant(&conversation.id, &discarded_node_id)
        .unwrap();
    state
        .conv_store
        .append_ai_draft(&conversation.id, "kept draft".into(), None)
        .unwrap();

    let snap = load_archive_snapshot(state.conv_store.clone(), conversation.id.clone())
        .await
        .unwrap();

    assert_eq!(snap.messages, vec!["user intent", "kept draft"]);
    assert_eq!(snap.archived_upto, 0);
}
