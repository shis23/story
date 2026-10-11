//! `vector_index` 单测（文件名沿用 `lib_tests_*` 约定：测试里直接用 legacy
//! CampaignStore 预置摘要，不计入 backend-baseline 的生产源扫描）。

use super::*;
use crate::commands::characters::{CharacterInfo, WorldInfoEntryInfo};

fn entry(keys: &[&str], content: &str, route: &str) -> WorldInfoEntryInfo {
    WorldInfoEntryInfo {
        keys: keys.iter().map(|k| k.to_string()).collect(),
        content: content.into(),
        constant: route == "Constant",
        route: route.into(),
        is_global: false,
        depth: 2,
        order: 100,
    }
}

fn save_character(state: &AppState, name: &str, entries: Vec<WorldInfoEntryInfo>) -> String {
    let info = CharacterInfo {
        source_character_id: Some(format!("source-{name}")),
        name: name.into(),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: String::new(),
        mes_example: String::new(),
        post_history_instructions: String::new(),
        alternate_greetings: vec![],
        system_prompt: String::new(),
        tags: vec![],
        creator: String::new(),
        character_version: String::new(),
        spec_version: "3.0".into(),
        extensions: serde_json::json!({}),
        embedded_world_info: None,
        renderable_assets: None,
        raw_card_json: serde_json::json!({}),
        has_world_info: !entries.is_empty(),
        has_renderable_assets: false,
        world_info_count: entries.len(),
        world_info_entries: entries,
    };
    state
        .storage()
        .save_character(info)
        .expect("save character")
        .id
}

fn save_summary(state: &AppState, summary: &RoundSummary) {
    state
        .storage()
        .json_campaign_store(
            crate::storage_backend::BackendCapability::ChroniclePublication,
            "test save summary",
        )
        .expect("json campaign store")
        .upsert_summary(summary.clone())
        .expect("save summary");
}

fn world_info_contents(state: &AppState) -> Vec<String> {
    let mut contents: Vec<String> = state
        .vector_store
        .records_matching(&MetadataFilter::new())
        .into_iter()
        .filter(|r| r.kind == VectorKind::WorldInfo)
        .map(|r| r.content)
        .collect();
    contents.sort();
    contents
}

#[test]
fn world_info_vectors_follow_the_character_library() {
    let state = AppState::new_for_test();
    // 旧版本写入的无标签记录（来自早已删除的卡）——重建后必须消失。
    state
        .vector_store
        .upsert(VectorRecord {
            id: Id::new(),
            content: "已删除卡的旧条目".into(),
            vector: vec![],
            keywords: vec!["王宫".into()],
            kind: VectorKind::WorldInfo,
            metadata: HashMap::new(),
        })
        .unwrap();
    let alice = save_character(
        &state,
        "Alice",
        vec![
            entry(&["王宫"], "Alice 的王宫", "Selective"),
            entry(&["誓言"], "Alice 的誓言", "Both"),
            entry(&["常驻"], "蓝灯不进检索池", "Constant"),
            entry(&["禁用"], "禁用条目", "Disabled"),
            entry(&["  "], "没有有效关键词", "Selective"),
        ],
    );
    save_character(
        &state,
        "Bob",
        vec![entry(&["王宫"], "Bob 的王宫", "Selective")],
    );

    sync_world_info_vectors(&state);
    assert_eq!(
        world_info_contents(&state),
        vec!["Alice 的王宫", "Alice 的誓言", "Bob 的王宫"]
    );

    // 删除 Alice 后重建：只剩 Bob 的条目（不会因为共享关键词被误删）。
    assert!(state.storage().delete_character(&alice).unwrap());
    sync_world_info_vectors(&state);
    assert_eq!(world_info_contents(&state), vec!["Bob 的王宫"]);

    // 重建幂等：再跑一次记录数不变。
    sync_world_info_vectors(&state);
    assert_eq!(world_info_contents(&state).len(), 1);
}

fn summary(campaign: &Id, conversation: &Id, turn: u32, content: &str) -> RoundSummary {
    RoundSummary::new(campaign.clone(), conversation.clone(), turn, content.into())
}

#[test]
fn fork_indexing_reuses_source_vectors_and_keyword_indexes_the_rest() {
    let state = Arc::new(AppState::new_for_test());
    let source = Id::from_str("camp-source");
    let fork = Id::from_str("camp-fork");
    let conv = Id::from_str("conv-fork");

    // 来源局：一条已嵌入的摘要。
    let embedded = summary(&source, &conv, 1, "昨夜有人潜入诊所");
    crate::runtime_support::index_summaries_with_vectors(
        state.vector_store.as_ref(),
        [&embedded],
        &[(embedded.id.clone(), vec![0.6, 0.8])],
    );

    // 分支局：同正文换了新 id，另有一条来源没有向量的摘要，以及一条 B 级摘要。
    let inherited = summary(&fork, &conv, 1, "昨夜有人潜入诊所");
    let keyword_only = summary(&fork, &conv, 2, "陈警官上门调查");
    let mut level_b = summary(&fork, &conv, 3, "B 级合并摘要");
    level_b.level = 1;
    for s in [&inherited, &keyword_only, &level_b] {
        save_summary(&state, s);
    }

    let indexed = index_campaign_summaries(&state, &fork, Some(&source));
    assert_eq!(indexed, 2, "只索引 A 级摘要");

    let fork_records = state
        .vector_store
        .records_matching(&MetadataFilter::new().for_campaign(&fork));
    let by_id = |id: &Id| fork_records.iter().find(|r| &r.id == id).cloned();
    assert_eq!(
        by_id(&inherited.id).expect("inherited indexed").vector,
        vec![0.6, 0.8],
        "正文相同的摘要复用来源向量，不重新嵌入"
    );
    assert!(
        by_id(&keyword_only.id)
            .expect("keyword indexed")
            .vector
            .is_empty()
    );
    assert!(by_id(&level_b.id).is_none());

    // 新局能被远记忆召回到。
    let hits = storyforge_app_memory::recall_archived_by_query_filtered(
        state.vector_store.as_ref(),
        "诊所",
        5,
        Some(fork.as_str()),
        None,
    )
    .unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn startup_backfill_indexes_only_missing_summaries() {
    let state = AppState::new_for_test();
    let mut campaign =
        storyforge_domain::campaign::Campaign::new(Id::from_str("card-backfill"), "backfill");
    campaign.id = Id::from_str("camp-backfill");
    state.storage().save_campaign(&campaign).unwrap();
    let conv = Id::from_str("conv-backfill");
    let indexed = summary(&campaign.id, &conv, 1, "已经索引过的摘要");
    let missing = summary(&campaign.id, &conv, 2, "导入后从未索引的摘要");
    save_summary(&state, &indexed);
    save_summary(&state, &missing);
    crate::runtime_support::index_summaries_with_vectors(
        state.vector_store.as_ref(),
        [&indexed],
        &[(indexed.id.clone(), vec![1.0, 0.0])],
    );

    assert_eq!(backfill_missing_summary_vectors(&state), 1);
    assert!(state.vector_store.contains(&missing.id));
    let kept = state
        .vector_store
        .records_matching(&MetadataFilter::new().for_campaign(&campaign.id))
        .into_iter()
        .find(|r| r.id == indexed.id)
        .unwrap();
    assert_eq!(kept.vector, vec![1.0, 0.0], "已有记录（含向量）原样保留");
    assert_eq!(backfill_missing_summary_vectors(&state), 0, "幂等");
}

#[test]
fn startup_purges_vectors_of_deleted_campaigns_only() {
    let state = AppState::new_for_test();
    let mut live = storyforge_domain::campaign::Campaign::new(Id::from_str("card-live"), "live");
    live.id = Id::from_str("camp-live");
    state.storage().save_campaign(&live).unwrap();
    let gone = Id::from_str("camp-deleted-before-fix");
    let conv = Id::from_str("conv-orphan");
    crate::runtime_support::index_summaries_with_vectors(
        state.vector_store.as_ref(),
        [
            &summary(&live.id, &conv, 1, "存活局的摘要"),
            &summary(&gone, &conv, 1, "已删局的残留摘要"),
        ],
        &[],
    );
    state
        .vector_store
        .upsert(VectorRecord {
            id: Id::from_str("legacy-untagged"),
            content: "非 Campaign 旧路径的归档".into(),
            vector: vec![],
            keywords: vec!["归档".into()],
            kind: VectorKind::ArchivedSummary,
            metadata: HashMap::new(),
        })
        .unwrap();

    assert_eq!(purge_orphan_campaign_vectors(&state), 1);
    assert!(
        state
            .vector_store
            .records_matching(&MetadataFilter::new().for_campaign(&gone))
            .is_empty()
    );
    assert_eq!(
        state
            .vector_store
            .records_matching(&MetadataFilter::new().for_campaign(&live.id))
            .len(),
        1
    );
    assert!(
        state
            .vector_store
            .contains(&Id::from_str("legacy-untagged")),
        "无标签记录不动"
    );
    assert_eq!(purge_orphan_campaign_vectors(&state), 0, "幂等");
}

// ─── 消息归档的搬运与向量补建 ────────────────────────────────────────────────

fn archive_record(
    id: &str,
    conversation: &Id,
    content: &str,
    range: Option<(usize, usize)>,
    vector: Vec<f32>,
) -> VectorRecord {
    let mut metadata = HashMap::new();
    metadata.insert("campaign_id".into(), serde_json::json!("camp-source"));
    metadata.insert(
        "conversation_id".into(),
        serde_json::json!(conversation.as_str()),
    );
    metadata.insert("source".into(), serde_json::json!("message_archive"));
    if let Some((start, end)) = range {
        metadata.insert("message_start".into(), start.into());
        metadata.insert("message_end".into(), end.into());
    }
    VectorRecord {
        id: Id::from_str(id),
        content: content.into(),
        vector,
        keywords: storyforge_app_memory::extract_keywords(content),
        kind: VectorKind::ArchivedSummary,
        metadata,
    }
}

fn conversation_records(state: &AppState, conversation: &Id) -> Vec<VectorRecord> {
    let mut records = state.vector_store.records_matching(
        &MetadataFilter::new()
            .match_eq("conversation_id", serde_json::json!(conversation.as_str())),
    );
    records.sort_by(|a, b| a.content.cmp(&b.content));
    records
}

#[test]
fn carried_watermark_never_skips_unarchived_messages() {
    assert_eq!(carried_archive_watermark(&[], 30), 0, "没有归档就从头归档");
    assert_eq!(
        carried_archive_watermark(&[Some((0, 2)), Some((3, 5))], 6),
        6
    );
    assert_eq!(
        carried_archive_watermark(&[Some((3, 5)), Some((0, 2))], 4),
        4,
        "不超过来源水位"
    );
    assert_eq!(
        carried_archive_watermark(&[Some((0, 2)), Some((6, 8))], 9),
        3,
        "中间缺一段时停在缺口前，缺的那段会被自动归档补上"
    );
    assert_eq!(
        carried_archive_watermark(&[Some((0, 2)), None], 9),
        9,
        "旧记录没有区间，无法核对时沿用来源水位"
    );
}

#[test]
fn fork_copies_message_archives_with_vectors_and_watermark() {
    let state = AppState::new_for_test();
    let source = state
        .conv_store
        .create(None, Some(Id::from_str("camp-source")));
    state
        .conv_store
        .advance_archived_upto(&source.id, 6)
        .unwrap();
    state
        .vector_store
        .upsert_batch(vec![
            archive_record(
                "a1",
                &source.id,
                "第一段：诊所遇袭",
                Some((0, 2)),
                vec![0.6, 0.8],
            ),
            archive_record("a2", &source.id, "第二段：警官上门", Some((3, 5)), vec![]),
        ])
        .unwrap();
    let target = state
        .conv_store
        .create(None, Some(Id::from_str("camp-fork")));

    let copied = copy_message_archives(&state, &source.id, &Id::from_str("camp-fork"), &target.id);

    assert_eq!(copied, 2);
    let records = conversation_records(&state, &target.id);
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|r| r.id != Id::from_str("a1") && r.id != Id::from_str("a2"))
    );
    assert!(
        records
            .iter()
            .all(|r| r.metadata["campaign_id"] == "camp-fork")
    );
    assert_eq!(
        records[0].vector,
        vec![0.6, 0.8],
        "复用来源向量，不重新嵌入"
    );
    assert_eq!(state.conv_store.archived_upto(&target.id), 6);
    assert_eq!(
        conversation_records(&state, &source.id).len(),
        2,
        "来源会话的归档原样保留"
    );
}

#[test]
fn bundle_archives_round_trip_as_keyword_records() {
    let state = Arc::new(AppState::new_for_test());
    let source = state
        .conv_store
        .create(None, Some(Id::from_str("camp-source")));
    state
        .vector_store
        .upsert_batch(vec![
            archive_record(
                "b2",
                &source.id,
                "第二段：警官上门",
                Some((3, 5)),
                vec![1.0],
            ),
            archive_record(
                "b1",
                &source.id,
                "第一段：诊所遇袭",
                Some((0, 2)),
                vec![1.0],
            ),
        ])
        .unwrap();

    let exported = export_message_archives(&state, &source.id);
    assert_eq!(exported.len(), 2);
    assert_eq!(exported[0].message_start, Some(0), "按消息区间排序");

    let target = state
        .conv_store
        .create(None, Some(Id::from_str("camp-import")));
    let imported = import_message_archives(
        &state,
        exported,
        9,
        &Id::from_str("camp-import"),
        &target.id,
    );
    assert_eq!(imported, 2);
    let records = conversation_records(&state, &target.id);
    assert!(
        records.iter().all(|r| r.vector.is_empty()),
        "向量不随包搬运"
    );
    assert!(
        records
            .iter()
            .all(|r| r.metadata["campaign_id"] == "camp-import")
    );
    assert_eq!(
        state.conv_store.archived_upto(&target.id),
        6,
        "水位收紧到归档实际覆盖的范围"
    );
    let hits = storyforge_app_memory::recall_archived_by_query_filtered(
        state.vector_store.as_ref(),
        "诊所",
        5,
        Some("camp-import"),
        None,
    )
    .unwrap();
    assert_eq!(hits.len(), 1, "导入后的归档可按关键词召回");
}

/// 极简 OpenAI 兼容嵌入接口：每条输入返回 `dim` 维向量。
fn spawn_embedding_server(dim: usize) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let body = loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break None;
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            if key.eq_ignore_ascii_case("content-length") {
                                value.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    if buf.len() >= split + 4 + length {
                        break Some(buf[split + 4..split + 4 + length].to_vec());
                    }
                }
            };
            let Some(body) = body else { continue };
            let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let inputs = request["input"].as_array().map_or(0, Vec::len);
            let data: Vec<serde_json::Value> = (0..inputs)
                .map(|i| serde_json::json!({ "index": i, "embedding": vec![0.5_f32; dim] }))
                .collect();
            let payload = serde_json::json!({ "data": data }).to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
        }
    });
    format!("http://{addr}/v1/embeddings")
}

fn embedder_for(endpoint: String, dim: usize) -> storyforge_infra_llm::Embedder {
    storyforge_infra_llm::Embedder::new(storyforge_infra_llm::EmbedConfig {
        endpoint,
        api_key: String::new(),
        model: "test-embed".into(),
        dim,
    })
    .unwrap()
}

#[tokio::test]
async fn backfill_embeds_keyword_only_and_wrong_dimension_records() {
    let state = AppState::new_for_test();
    let conv = Id::from_str("conv-backfill-embed");
    state
        .vector_store
        .upsert_batch(vec![
            archive_record("k1", &conv, "只有关键词的归档", None, vec![]),
            archive_record("old", &conv, "旧模型的向量", None, vec![1.0, 0.0]),
            archive_record("ok", &conv, "已经嵌入过", None, vec![0.1, 0.2, 0.3]),
        ])
        .unwrap();
    assert_eq!(
        archive_embedding_counts(state.vector_store.as_ref(), Some(3)),
        (3, 2)
    );
    assert_eq!(
        archive_embedding_counts(state.vector_store.as_ref(), None),
        (3, 1),
        "未配置嵌入模型时只统计完全没有向量的记录"
    );

    let candidates = archive_records_needing_embedding(state.vector_store.as_ref(), 3);
    assert_eq!(candidates.len(), 2);
    let embedder = embedder_for(spawn_embedding_server(3), 3);
    assert_eq!(embedder.probe_dim().await.unwrap(), 3);
    let report = embed_archive_records(state.vector_store.as_ref(), &embedder, candidates).await;
    assert_eq!(
        report,
        MemoryEmbeddingReport {
            candidates: 2,
            embedded: 2,
            failed: 0,
            error: None,
        }
    );
    assert!(
        archive_records_needing_embedding(state.vector_store.as_ref(), 3).is_empty(),
        "补建后所有记录都是当前维度"
    );
    let kept = conversation_records(&state, &conv)
        .into_iter()
        .find(|r| r.id == Id::from_str("ok"))
        .unwrap();
    assert_eq!(kept.vector, vec![0.1, 0.2, 0.3], "已有向量不动");
}

#[tokio::test]
async fn backfill_stops_at_the_first_embedding_failure() {
    let state = AppState::new_for_test();
    let conv = Id::from_str("conv-backfill-fail");
    state
        .vector_store
        .upsert(archive_record(
            "k1",
            &conv,
            "只有关键词的归档",
            None,
            vec![],
        ))
        .unwrap();
    let embedder = embedder_for("http://127.0.0.1:1/v1/embeddings".into(), 3);
    let candidates = archive_records_needing_embedding(state.vector_store.as_ref(), 3);
    let report = embed_archive_records(state.vector_store.as_ref(), &embedder, candidates).await;
    assert_eq!(report.candidates, 1);
    assert_eq!(report.embedded, 0);
    assert_eq!(report.failed, 1);
    assert!(report.error.is_some());
    assert!(
        conversation_records(&state, &conv)[0].vector.is_empty(),
        "失败时记录保持关键词可召回"
    );
}
