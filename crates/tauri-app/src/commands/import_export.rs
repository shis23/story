use super::super::*;

// ─── 导出命令（W7: ST 卡 PNG + 共享 Lorebook + JSON Bundle）─────────────────

/// 导出的文件 DTO（文件名 + 字节）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedFile {
    pub filename: String,
    pub data: Vec<u8>,
}

/// Campaign 导出结果 DTO（多角色 PNG + 共享 lorebook）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignExportResult {
    /// 每个角色的 ST 卡 PNG
    pub cards: Vec<ExportedFile>,
    /// 共享 lorebook JSON（ST 格式，可独立保存）
    pub lorebook_json: String,
}

/// StoryForge JSON Bundle 格式版本
pub(crate) const BUNDLE_FORMAT_VERSION: u32 = 2;

/// StoryForge Campaign 完整 JSON Bundle
#[derive(Debug, Serialize, Deserialize)]
pub struct CampaignBundle {
    pub(crate) format_version: u32,
    pub(crate) exported_at: String,
    /// v2 起保留完整 CharacterCard，便于跨设备导入后继续开新档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) card: Option<storyforge_domain::character::CharacterCard>,
    pub(crate) campaign: storyforge_domain::campaign::Campaign,
    pub(crate) instances: Vec<storyforge_domain::campaign::CharacterInstance>,
    /// key = definition_id
    pub(crate) definitions: Vec<storyforge_domain::character::CharacterDefinition>,
    pub(crate) knowledge: Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry>,
    pub(crate) tasks: Vec<storyforge_domain::story_task::StoryTask>,
    pub(crate) summaries: Vec<storyforge_domain::agent::RoundSummary>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct BundleStoreSnapshot {
    cards: serde_json::Value,
    campaigns: serde_json::Value,
    instances: serde_json::Value,
    knowledge: serde_json::Value,
    tasks: serde_json::Value,
    summaries: serde_json::Value,
    mvu: serde_json::Value,
    conversations: serde_json::Value,
}

pub(crate) fn value_of<T: Serialize>(value: T, label: &str) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|error| format!("serialize {label}: {error}"))
}

pub(crate) fn bundle_store_snapshot_in_memory(
    store: &campaign_store::CampaignStore,
    conv_store: &ConversationStore,
) -> Result<BundleStoreSnapshot, String> {
    let mut conversations: Vec<_> = conv_store
        .list()
        .into_iter()
        .filter_map(|summary| conv_store.get(&summary.id))
        .collect();
    conversations.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    Ok(BundleStoreSnapshot {
        cards: value_of(store.list_cards(), "cards")?,
        campaigns: value_of(store.list_campaigns(), "campaigns")?,
        instances: value_of(store.list_all_instances(), "instances")?,
        knowledge: value_of(store.list_all_knowledge(), "knowledge")?,
        tasks: value_of(store.list_all_tasks(), "tasks")?,
        summaries: value_of(store.list_all_summaries(), "summaries")?,
        mvu: value_of(store.list_all_mvu(), "mvu_translations")?,
        conversations: value_of(conversations, "conversations")?,
    })
}

pub(crate) fn read_json_collection_strict<T>(
    path: &std::path::Path,
) -> Result<serde_json::Value, String>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    if !path.exists() {
        return Ok(serde_json::json!([]));
    }
    // M-1：错误信息脱敏，避免把绝对数据目录路径泄漏到前端。
    let safe = crate::error::sanitize_path_for_ipc(path);
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("{safe} metadata: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err(format!("{safe} is not a regular file"));
    }
    let bytes = std::fs::read(path).map_err(|error| format!("{safe} read: {error}"))?;
    let parsed: Vec<T> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{safe} parse: {error}"))?;
    value_of(parsed, &safe)
}

pub(crate) fn read_conversations_strict(
    dir: &std::path::Path,
) -> Result<serde_json::Value, String> {
    if !dir.exists() {
        return Ok(serde_json::json!([]));
    }
    // M-1：错误信息脱敏，避免把绝对数据目录路径泄漏到前端。
    let safe_dir = crate::error::sanitize_path_for_ipc(dir);
    let metadata = std::fs::symlink_metadata(dir)
        .map_err(|error| format!("{safe_dir} metadata: {error}"))?;
    if !metadata.file_type().is_dir() {
        return Err(format!("{safe_dir} is not a directory"));
    }
    let mut conversations = Vec::new();
    for entry in
        std::fs::read_dir(dir).map_err(|error| format!("{safe_dir} read_dir: {error}"))?
    {
        let entry = entry.map_err(|error| format!("{safe_dir} entry: {error}"))?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let safe_path = crate::error::sanitize_path_for_ipc(&path);
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("{safe_path} read: {error}"))?;
            let conversation: storyforge_domain::conversation::Conversation =
                serde_json::from_slice(&bytes)
                    .map_err(|error| format!("{safe_path} parse: {error}"))?;
            conversations.push(conversation);
        }
    }
    conversations.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    value_of(conversations, "conversations")
}

pub(crate) fn read_bundle_disk_snapshot_strict(
    data_dir: &std::path::Path,
    conversations_dir: &std::path::Path,
) -> Result<BundleStoreSnapshot, String> {
    Ok(BundleStoreSnapshot {
        cards: read_json_collection_strict::<campaign_store::StoredCard>(
            &data_dir.join("cards.json"),
        )?,
        campaigns: read_json_collection_strict::<storyforge_domain::campaign::Campaign>(
            &data_dir.join("campaigns.json"),
        )?,
        instances: read_json_collection_strict::<storyforge_domain::campaign::CharacterInstance>(
            &data_dir.join("instances.json"),
        )?,
        knowledge: read_json_collection_strict::<
            storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
        >(&data_dir.join("knowledge.json"))?,
        tasks: read_json_collection_strict::<storyforge_domain::story_task::StoryTask>(
            &data_dir.join("tasks.json"),
        )?,
        summaries: read_json_collection_strict::<storyforge_domain::agent::RoundSummary>(
            &data_dir.join("round_summaries.json"),
        )?,
        mvu: read_json_collection_strict::<campaign_store::StoredMvuTranslation>(
            &data_dir.join("mvu_translations.json"),
        )?,
        conversations: read_conversations_strict(conversations_dir)?,
    })
}

pub(crate) fn validate_bundle_summary_graph(
    campaign: &storyforge_domain::campaign::Campaign,
    summaries: &[storyforge_domain::agent::RoundSummary],
) -> Result<(), TauriCommandError> {
    use std::collections::{HashMap, HashSet};
    use storyforge_domain::agent::RoundSummary;

    if summaries.is_empty() {
        return Ok(());
    }
    let Some(conversation_id) = campaign.conversation_id.as_ref() else {
        return Err(TauriCommandError::validation(
            "Bundle 含 Chronicle 摘要但 Campaign 缺少 conversation_id",
        ));
    };
    let Some(lineage_id) = campaign.lineage_id.as_ref() else {
        return Err(TauriCommandError::validation(
            "Bundle 含 Chronicle 摘要但 Campaign 缺少 lineage_id",
        ));
    };

    let mut by_id: HashMap<Id, &RoundSummary> = HashMap::with_capacity(summaries.len());
    let mut codes = HashSet::with_capacity(summaries.len());
    for summary in summaries {
        if by_id.insert(summary.id.clone(), summary).is_some() {
            return Err(TauriCommandError::validation(format!(
                "Bundle 摘要 id 重复: {}",
                summary.id
            )));
        }
        if summary.campaign_id != campaign.id {
            return Err(TauriCommandError::validation(format!(
                "Bundle 摘要 {} campaign scope 不匹配",
                summary.id
            )));
        }
        if summary.conversation_id != *conversation_id {
            return Err(TauriCommandError::validation(format!(
                "Bundle 摘要 {} conversation scope 不匹配",
                summary.id
            )));
        }
        if summary.lineage_id.as_ref() != Some(lineage_id) {
            return Err(TauriCommandError::validation(format!(
                "Bundle 摘要 {} lineage scope 不匹配",
                summary.id
            )));
        }
        if summary.level > 2 {
            return Err(TauriCommandError::validation(format!(
                "Bundle 摘要 {} level={} 非法",
                summary.id, summary.level
            )));
        }
        if let Some(raw_code) = summary.code.as_deref() {
            let code =
                storyforge_domain::chronicle::ChronicleCode::parse(raw_code).ok_or_else(|| {
                    TauriCommandError::validation(format!(
                        "Bundle 摘要 {} code 非法: {raw_code}",
                        summary.id
                    ))
                })?;
            if code
                .level()
                .is_none_or(|level| level.as_u8() != summary.level)
            {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要 {} code={} 与 level={} 不匹配",
                    summary.id, raw_code, summary.level
                )));
            }
            if !codes.insert(code.as_str().to_string()) {
                return Err(TauriCommandError::validation(format!(
                    "Bundle lineage {} 内 Chronicle code 重复: {}",
                    lineage_id, raw_code
                )));
            }
        }
        if summary.turn == 0 || (summary.turn_end != 0 && summary.turn_end < summary.turn) {
            return Err(TauriCommandError::validation(format!(
                "Bundle 摘要 {} turn span 非法: {}..{}",
                summary.id, summary.turn, summary.turn_end
            )));
        }
        if summary.level == 0 {
            if !summary.covers.is_empty() || summary.effective_turn_end() != summary.turn {
                return Err(TauriCommandError::validation(format!(
                    "Bundle A 摘要 {} 必须是无 covers 的单轮 leaf",
                    summary.id
                )));
            }
        } else if summary.covers.is_empty() {
            return Err(TauriCommandError::validation(format!(
                "Bundle B/C 摘要 {} 必须覆盖子摘要",
                summary.id
            )));
        }

        let mut unique_covers = HashSet::with_capacity(summary.covers.len());
        for child_id in &summary.covers {
            if child_id == &summary.id || !unique_covers.insert(child_id) {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要 {} covers 含 self/重复 child {}",
                    summary.id, child_id
                )));
            }
        }
    }

    for summary in summaries {
        if let Some(parent_id) = &summary.covered_by {
            let Some(parent) = by_id.get(parent_id).copied() else {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要 {} covered_by 引用无效: {}",
                    summary.id, parent_id
                )));
            };
            if !parent.covers.contains(&summary.id) {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要图不对称: {} covered_by {}，但 parent 未 covers child",
                    summary.id, parent_id
                )));
            }
            if parent.level != summary.level + 1 {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要层级非法: parent {} level={} child {} level={}",
                    parent.id, parent.level, summary.id, summary.level
                )));
            }
        }

        if summary.covers.is_empty() {
            continue;
        }
        let mut children = Vec::with_capacity(summary.covers.len());
        for child_id in &summary.covers {
            let Some(child) = by_id.get(child_id).copied() else {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要 {} covers 引用无效: {}",
                    summary.id, child_id
                )));
            };
            if child.covered_by.as_ref() != Some(&summary.id) {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要图不对称: {} covers {}，但 child.covered_by 不匹配",
                    summary.id, child.id
                )));
            }
            if summary.level != child.level + 1 {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要层级非法: parent {} level={} child {} level={}",
                    summary.id, summary.level, child.id, child.level
                )));
            }
            children.push(child);
        }
        children.sort_by_key(|child| child.turn);
        let mut expected_turn = summary.turn;
        for child in children {
            if child.turn != expected_turn {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要 {} covers span 不连续/不重合: expected turn {}, child {} starts {}",
                    summary.id, expected_turn, child.id, child.turn
                )));
            }
            expected_turn = child.effective_turn_end().checked_add(1).ok_or_else(|| {
                TauriCommandError::validation(format!("Bundle 摘要 {} child span 溢出", summary.id))
            })?;
        }
        if expected_turn - 1 != summary.effective_turn_end() {
            return Err(TauriCommandError::validation(format!(
                "Bundle 摘要 {} span 与 covers 不一致",
                summary.id
            )));
        }
    }

    // Level checks already make cycles impossible for valid graphs. Keep an explicit
    // DFS guard so malformed legacy levels cannot hide a cyclic covers graph.
    fn visit(
        id: &Id,
        by_id: &HashMap<Id, &RoundSummary>,
        visiting: &mut HashSet<Id>,
        visited: &mut HashSet<Id>,
    ) -> Result<(), TauriCommandError> {
        if visited.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id.clone()) {
            return Err(TauriCommandError::validation(format!(
                "Bundle 摘要 covers 图存在环: {id}"
            )));
        }
        if let Some(summary) = by_id.get(id) {
            for child in &summary.covers {
                visit(child, by_id, visiting, visited)?;
            }
        }
        visiting.remove(id);
        visited.insert(id.clone());
        Ok(())
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for id in by_id.keys() {
        visit(id, &by_id, &mut visiting, &mut visited)?;
    }
    Ok(())
}

/// StoryForge Campaign Bundle 导入结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignImportResult {
    pub campaign_id: String,
    pub card_id: String,
    pub conversation_id: String,
    pub instance_count: usize,
    pub knowledge_count: usize,
    pub task_count: usize,
    pub summary_count: usize,
}

/// 导出单个角色卡为 ST PNG
///
/// 从 CharacterStore 取原始 Character（含 raw_card_json），
/// 用 to_st_data 构建 StCharacterData，再写入 PNG。
#[tauri::command]
pub(crate) fn export_st_card_png(
    character_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<u8>, TauriCommandError> {
    let stored = state
        .storage()
        .get_character(&character_id)
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| TauriCommandError::not_found(format!("角色卡不存在: {character_id}")))?;

    // 从 CharacterStore 恢复 Character（精简版，但够 to_st_data 用）
    let character = stored_info_to_character(&stored);

    // 构建 ST 数据（用内嵌世界书作为 character_book）
    let book = character
        .embedded_world_info
        .as_ref()
        .map(|b| b.to_st_book());
    let st_data = storyforge_domain::character::to_st_data(&character, None, book);
    let card = storyforge_infra_import::png::make_st_card(st_data, &character.spec_version);

    storyforge_infra_import::png::write_st_card_png(&card, None)
        .map_err(|e| TauriCommandError::internal(format!("PNG 导出失败: {e}")))
}

/// 导出 Campaign 全部角色为 ST PNG + 共享 lorebook
///
/// 策略（用户已定）：每角色一张 PNG + 共享 lorebook。
/// 共享知识/世界书转 ST lorebook 格式。
#[tauri::command]
pub(crate) fn export_campaign_st_cards(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignExportResult, TauriCommandError> {
    let camp_id = Id::from_str(&campaign_id);

    let campaign = state
        .storage()
        .get_campaign(&camp_id)
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| TauriCommandError::not_found(format!("Campaign 不存在: {campaign_id}")))?;
    let campaign = campaign.campaign;

    let stored_card = state
        .storage()
        .get_card(&campaign.card_id)
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| {
            TauriCommandError::not_found(format!("Campaign 关联的卡不存在: {}", campaign.card_id))
        })?;

    let instances = state
        .storage()
        .list_instances(&camp_id)
        .map_err(TauriCommandError::storage)?;
    if instances.is_empty() {
        return Err("Campaign 无角色实例，无法导出".into());
    }

    // 构建共享 lorebook（Campaign 级知识 → ST WorldInfoBook）
    let shared_knowledge = state
        .storage()
        .list_knowledge(&camp_id)
        .map_err(TauriCommandError::storage)?;
    let shared_lorebook = knowledge_to_st_book(&shared_knowledge);
    let shared_lorebook_json =
        serde_json::to_string_pretty(&shared_lorebook).unwrap_or_else(|_| "{}".into());

    // 尝试从角色库获取原始 Character（用于 raw_card_json）
    let original_character = state
        .storage()
        .get_character(stored_card.card.source_character_id.as_str())
        .map_err(TauriCommandError::storage)?
        .map(|s| stored_info_to_character(&s));

    let mut cards = Vec::new();
    for inst in &instances {
        // 找到对应的 definition
        let definition = inst.definition_id.as_ref().and_then(|did| {
            stored_card
                .card
                .character_definitions
                .iter()
                .find(|d| d.id == *did)
        });

        let (st_data, spec_version) = if let (Some(character), Some(def)) =
            (&original_character, definition)
        {
            // 有原始 Character → 用 to_st_data（round-trip 保底）
            let book = character
                .embedded_world_info
                .as_ref()
                .map(|b| b.to_st_book());
            let data = storyforge_domain::character::to_st_data(character, Some(def), book);
            (data, character.spec_version.clone())
        } else if let Some(def) = definition {
            // 只有 Card + Definition → 用 to_st_data_from_card
            let data =
                storyforge_domain::character::to_st_data_from_card(&stored_card.card, def, None);
            (data, "3.0".into())
        } else {
            // 临时角色（无 definition）→ 用 instance 名字构建最小卡
            let data = storyforge_domain::character::empty_st_data(&inst.name);
            (data, "3.0".into())
        };

        let card = storyforge_infra_import::png::make_st_card(st_data, &spec_version);
        let png_bytes =
            storyforge_infra_import::png::write_st_card_png(&card, None).map_err(|e| {
                TauriCommandError::internal(format!("PNG 导出失败 ({}): {e}", inst.name))
            })?;

        let filename = sanitize_filename(&format!("{}.png", inst.name));
        cards.push(ExportedFile {
            filename,
            data: png_bytes,
        });
    }

    Ok(CampaignExportResult {
        cards,
        lorebook_json: shared_lorebook_json,
    })
}

/// 导出 StoryForge Campaign 完整 JSON Bundle
///
/// 包含 Campaign 元数据 + Instances + Definitions + Knowledge + Tasks + Summaries。
#[tauri::command]
pub(crate) fn export_campaign_bundle(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<String, TauriCommandError> {
    let camp_id = Id::from_str(&campaign_id);
    state.storage().export_campaign_bundle(&camp_id)
}

pub(crate) fn export_campaign_bundle_from_store(
    store: &campaign_store::CampaignStore,
    camp_id: Id,
) -> Result<String, TauriCommandError> {
    let campaign = store.get_campaign(&camp_id).ok_or_else(|| {
        TauriCommandError::not_found(format!("Campaign 不存在: {}", camp_id.as_str()))
    })?;

    let stored_card = store.get_card(&campaign.card_id);
    let instances = store.list_instances(&camp_id);
    let definitions: Vec<_> = stored_card
        .as_ref()
        .map(|c| c.card.character_definitions.clone())
        .unwrap_or_default();
    let knowledge = store.list_knowledge(&camp_id);
    let tasks = store.list_tasks(&camp_id);
    let summaries = store.list_summaries(&camp_id);

    let bundle = CampaignBundle {
        format_version: BUNDLE_FORMAT_VERSION,
        exported_at: chrono::Utc::now().to_rfc3339(),
        card: stored_card.as_ref().map(|c| c.card.clone()),
        campaign,
        instances,
        definitions,
        knowledge,
        tasks,
        summaries,
    };

    serde_json::to_string_pretty(&bundle)
        .map_err(|e| TauriCommandError::internal(format!("Bundle 序列化失败: {e}")))
}

/// 导入 StoryForge Campaign JSON Bundle。
///
/// 导入始终生成全新 card/campaign/instance/knowledge/task/summary ID，避免覆盖现有数据。
#[tauri::command]
pub(crate) fn import_campaign_bundle(
    bundle_json: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CampaignImportResult, TauriCommandError> {
    // H-2: bound the IPC payload before deserializing (16 MiB). A hijacked plugin
    // iframe or buggy frontend could otherwise OOM the process with a huge string.
    crate::error::require_ipc_size(
        &bundle_json,
        crate::error::MAX_BUNDLE_JSON_BYTES,
        "Campaign Bundle",
    )?;
    let bundle: CampaignBundle = serde_json::from_str(&bundle_json)
        .map_err(|e| TauriCommandError::validation(format!("Bundle JSON 解析失败: {e}")))?;
    state
        .storage()
        .import_campaign_bundle(bundle, state.conv_store.as_ref())
}

/// SQLite 分支：与 JSON 路径相同的校验/重写，但整包（含 conversation）在
/// 一个 SQLite 事务内落库 —— 失败整体回滚（`import_campaign_bundle_into_db`
/// 自带 fault-injection 证明）。成功后刷新 ConversationStore 缓存。
pub(crate) fn import_campaign_bundle_into_sqlite(
    conv_store: &ConversationStore,
    bundle: CampaignBundle,
) -> Result<CampaignImportResult, TauriCommandError> {
    if bundle.format_version == 0 || bundle.format_version > BUNDLE_FORMAT_VERSION {
        return Err(TauriCommandError::validation(format!(
            "不支持的 Campaign Bundle 版本: {}",
            bundle.format_version
        )));
    }
    validate_bundle_summary_graph(&bundle.campaign, &bundle.summaries)?;

    let rewritten = rewrite_bundle_ids(bundle)?;
    let mut campaign = rewritten.campaign;
    let conversation = storyforge_domain::conversation::Conversation::new(
        Some(rewritten.card.id.as_str().to_string()),
        Some(campaign.id.clone()),
    );
    campaign.conversation_id = Some(conversation.id.clone());
    let mut rewritten_summaries = rewritten.rewritten_summaries;
    for summary in &mut rewritten_summaries {
        summary.conversation_id = conversation.id.clone();
    }

    crate::sqlite_runtime::import_campaign_bundle_into_db(
        &conversation,
        &rewritten.card,
        &campaign,
        &rewritten.rewritten_instances,
        &rewritten.rewritten_knowledge,
        &rewritten.rewritten_tasks,
        &rewritten_summaries,
    )
    .map_err(|e| TauriCommandError::storage(format!("导入 Campaign Bundle 失败: {e}")))?;

    // SQLite 会话权威按需重载：让新 conversation 对会话缓存可见。
    conv_store.invalidate();

    tracing::info!(
        "Imported Campaign Bundle {} -> {}",
        rewritten.old_campaign_id,
        campaign.id
    );

    Ok(CampaignImportResult {
        campaign_id: campaign.id.as_str().to_string(),
        card_id: rewritten.card.id.as_str().to_string(),
        conversation_id: conversation.id.as_str().to_string(),
        instance_count: rewritten.rewritten_instances.len(),
        knowledge_count: rewritten.rewritten_knowledge.len(),
        task_count: rewritten.rewritten_tasks.len(),
        summary_count: rewritten_summaries.len(),
    })
}

pub(crate) fn import_campaign_bundle_into_store(
    store: &campaign_store::CampaignStore,
    conv_store: &ConversationStore,
    bundle: CampaignBundle,
) -> Result<CampaignImportResult, TauriCommandError> {
    import_campaign_bundle_into_store_with_after_campaign(store, conv_store, bundle, || Ok(()))
}

/// 完整重写后的导入载荷（后端无关：JSON 与 SQLite 路径共用）。
pub(crate) struct RewrittenBundle {
    pub old_campaign_id: Id,
    pub card: storyforge_domain::character::CharacterCard,
    pub campaign: storyforge_domain::campaign::Campaign,
    pub rewritten_instances: Vec<storyforge_domain::campaign::CharacterInstance>,
    pub rewritten_knowledge: Vec<storyforge_domain::character_knowledge::CharacterKnowledgeEntry>,
    pub rewritten_tasks: Vec<storyforge_domain::story_task::StoryTask>,
    pub rewritten_summaries: Vec<storyforge_domain::agent::RoundSummary>,
}

/// 纯函数：为导入生成全新 ID 并重写全部内部引用（card / campaign /
/// instances / knowledge / tasks / summaries 的 id、FK、covers、covered_by）。
///
/// 不做任何存储写入；调用方负责在写入失败时回滚。
pub(crate) fn rewrite_bundle_ids(
    bundle: CampaignBundle,
) -> Result<RewrittenBundle, TauriCommandError> {
    use std::collections::HashMap;

    let old_campaign_id = bundle.campaign.id.clone();
    let new_card_id = Id::new();
    let new_campaign_id = Id::new();
    let new_source_character_id = Id::new();

    let mut definition_id_map: HashMap<Id, Id> = HashMap::new();
    let mut card = bundle
        .card
        .unwrap_or_else(|| storyforge_domain::character::CharacterCard {
            id: bundle.campaign.card_id.clone(),
            name: bundle.campaign.name.clone(),
            source_character_id: new_source_character_id.clone(),
            character_definitions: bundle.definitions.clone(),
            campaign_variable_schema: bundle.campaign.variable_schema.clone(),
            raw_card_json: serde_json::Value::Null,
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Unknown,
            extraction_message: None,
        });
    let definitions = if card.character_definitions.is_empty() {
        bundle.definitions
    } else {
        card.character_definitions
    };
    card.id = new_card_id.clone();
    card.source_character_id = new_source_character_id;
    if card.name.trim().is_empty() {
        card.name = bundle.campaign.name.clone();
    }
    card.character_definitions = definitions
        .into_iter()
        .map(|mut def| {
            let new_id = Id::new();
            definition_id_map.insert(def.id.clone(), new_id.clone());
            def.id = new_id;
            def.card_id = new_card_id.clone();
            def
        })
        .collect();

    // Build fully rewritten payloads first so a mid-write failure can roll back
    // without leaving a partially imported Campaign graph.
    let mut campaign = bundle.campaign;
    campaign.id = new_campaign_id.clone();
    campaign.card_id = new_card_id.clone();
    campaign.fork_from = None;
    // This marker describes an in-flight publication in the source store. Its ids
    // are rewritten below and its job/ledger state is not part of Bundle v2.
    campaign.pending_compress_publication = None;

    let mut instance_id_map: HashMap<Id, Id> = HashMap::new();
    for instance in &bundle.instances {
        instance_id_map.insert(instance.id.clone(), Id::new());
    }
    let mut rewritten_instances = Vec::with_capacity(bundle.instances.len());
    for mut instance in bundle.instances {
        let Some(new_instance_id) = instance_id_map.get(&instance.id).cloned() else {
            return Err(TauriCommandError::validation(format!(
                "Bundle 实例引用无效: {}",
                instance.id
            )));
        };
        instance.id = new_instance_id;
        instance.campaign_id = new_campaign_id.clone();
        if let Some(old_def_id) = instance.definition_id.clone() {
            let Some(new_def_id) = definition_id_map.get(&old_def_id).cloned() else {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 实例引用了不存在的 definition: {}",
                    old_def_id
                )));
            };
            instance.definition_id = Some(new_def_id);
        } else {
            instance.definition_id = None;
        }
        rewritten_instances.push(instance);
    }

    let mut knowledge_id_map: HashMap<Id, Id> = HashMap::new();
    for entry in &bundle.knowledge {
        knowledge_id_map.insert(entry.id.clone(), Id::new());
    }
    let mut rewritten_knowledge = Vec::new();
    for mut entry in bundle.knowledge {
        let Some(new_character_id) = instance_id_map.get(&entry.character_id).cloned() else {
            return Err(TauriCommandError::validation(format!(
                "Bundle 知识引用了不存在的角色实例: {}",
                entry.character_id
            )));
        };
        let Some(new_entry_id) = knowledge_id_map.get(&entry.id).cloned() else {
            return Err(TauriCommandError::validation(format!(
                "Bundle 知识 id 映射失败: {}",
                entry.id
            )));
        };
        entry.id = new_entry_id;
        entry.campaign_id = new_campaign_id.clone();
        entry.character_id = new_character_id;
        if let Some(old_source_character) = entry.source_character_id.clone() {
            let Some(new_source_character) = instance_id_map.get(&old_source_character).cloned()
            else {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 知识 source_character_id 无效: {}",
                    old_source_character
                )));
            };
            entry.source_character_id = Some(new_source_character);
        }
        if let Some(old_source_knowledge) = entry.source_knowledge_id.clone() {
            let Some(new_source_knowledge) = knowledge_id_map.get(&old_source_knowledge).cloned()
            else {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 知识 source_knowledge_id 无效: {}",
                    old_source_knowledge
                )));
            };
            entry.source_knowledge_id = Some(new_source_knowledge);
        }
        rewritten_knowledge.push(entry);
    }

    let mut rewritten_tasks = Vec::with_capacity(bundle.tasks.len());
    for mut task in bundle.tasks {
        task.id = Id::new();
        task.campaign_id = new_campaign_id.clone();
        let mut related = Vec::with_capacity(task.related_characters.len());
        for old_id in task.related_characters {
            let Some(new_id) = instance_id_map.get(&old_id).cloned() else {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 任务 related_characters 引用无效: {}",
                    old_id
                )));
            };
            related.push(new_id);
        }
        task.related_characters = related;
        rewritten_tasks.push(task);
    }

    // Rewrite Chronicle graph ids (covers / covered_by) before write.
    let mut summary_id_map: HashMap<Id, Id> = HashMap::new();
    for summary in &bundle.summaries {
        summary_id_map.insert(summary.id.clone(), Id::new());
    }
    let mut rewritten_summaries = Vec::with_capacity(bundle.summaries.len());
    for mut summary in bundle.summaries {
        let Some(new_summary_id) = summary_id_map.get(&summary.id).cloned() else {
            return Err(TauriCommandError::validation(format!(
                "Bundle 摘要 id 映射失败: {}",
                summary.id
            )));
        };
        summary.id = new_summary_id;
        summary.campaign_id = new_campaign_id.clone();
        if let Some(old_parent) = summary.covered_by.clone() {
            let Some(new_parent) = summary_id_map.get(&old_parent).cloned() else {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要 covered_by 引用无效: {}",
                    old_parent
                )));
            };
            summary.covered_by = Some(new_parent);
        }
        let mut covers = Vec::with_capacity(summary.covers.len());
        for old_child in summary.covers {
            let Some(new_child) = summary_id_map.get(&old_child).cloned() else {
                return Err(TauriCommandError::validation(format!(
                    "Bundle 摘要 covers 引用无效: {}",
                    old_child
                )));
            };
            covers.push(new_child);
        }
        summary.covers = covers;
        // conversation id filled after conversation is created.
        rewritten_summaries.push(summary);
    }

    Ok(RewrittenBundle {
        old_campaign_id,
        card,
        campaign,
        rewritten_instances,
        rewritten_knowledge,
        rewritten_tasks,
        rewritten_summaries,
    })
}

pub(crate) fn import_campaign_bundle_into_store_with_after_campaign<F>(
    store: &campaign_store::CampaignStore,
    conv_store: &ConversationStore,
    bundle: CampaignBundle,
    after_campaign_saved: F,
) -> Result<CampaignImportResult, TauriCommandError>
where
    F: FnOnce() -> Result<(), TauriCommandError>,
{
    if bundle.format_version == 0 || bundle.format_version > BUNDLE_FORMAT_VERSION {
        return Err(TauriCommandError::validation(format!(
            "不支持的 Campaign Bundle 版本: {}",
            bundle.format_version
        )));
    }
    validate_bundle_summary_graph(&bundle.campaign, &bundle.summaries)?;

    let rewritten = rewrite_bundle_ids(bundle)?;
    let old_campaign_id = rewritten.old_campaign_id;
    let new_card_id = rewritten.card.id.clone();
    let new_campaign_id = rewritten.campaign.id.clone();
    let card = rewritten.card;
    let mut campaign = rewritten.campaign;
    let rewritten_instances = rewritten.rewritten_instances;
    let rewritten_knowledge = rewritten.rewritten_knowledge;
    let rewritten_tasks = rewritten.rewritten_tasks;
    let rewritten_summaries = rewritten.rewritten_summaries;

    let mut created_conversation_id: Option<Id> = None;
    let memory_baseline = bundle_store_snapshot_in_memory(store, conv_store)
        .map_err(|error| TauriCommandError::storage(format!("导入前快照失败: {error}")))?;
    let data_dir = store
        .data_dir()
        .ok_or_else(|| TauriCommandError::storage("导入前 CampaignStore 缺少 data_dir"))?;
    let disk_baseline = read_bundle_disk_snapshot_strict(data_dir, conv_store.data_dir())
        .map_err(|error| TauriCommandError::storage(format!("导入前磁盘校验失败: {error}")))?;
    if disk_baseline != memory_baseline {
        return Err(TauriCommandError::storage(
            "导入前内存缓存与磁盘状态不一致，拒绝覆盖",
        ));
    }

    let rollback = |created_conversation_id: &Option<Id>| -> Result<(), String> {
        let mut errors = Vec::new();
        // Always compensate known ids: a store method can mutate its cache before a
        // persistence error is returned, so a success flag alone is insufficient.
        // CampaignStore now restores its own files when a cascade write fails. Keep
        // the returned error as diagnostic context, but only classify it as a
        // rollback failure if the snapshot checks below prove compensation failed.
        let delete_campaign_error = store
            .delete_campaign(&new_campaign_id)
            .err()
            .map(|error| format!("delete_campaign: {error}"));
        if let Err(e) = store.delete_card(&new_card_id) {
            errors.push(format!("delete_card: {e}"));
        }
        if let Some(conversation_id) = created_conversation_id
            && let Err(e) = conv_store.delete(conversation_id)
        {
            errors.push(format!("delete_conversation: {e}"));
        }

        match bundle_store_snapshot_in_memory(store, conv_store) {
            Ok(current) if current != memory_baseline => errors.push(
                "rollback verification: in-memory stores differ from pre-import snapshot".into(),
            ),
            Err(error) => errors.push(format!(
                "rollback verification: in-memory snapshot failed: {error}"
            )),
            Ok(_) => {}
        }

        match store.data_dir() {
            Some(data_dir) => {
                match read_bundle_disk_snapshot_strict(data_dir, conv_store.data_dir()) {
                    Ok(current) if current != disk_baseline => errors.push(
                        "rollback verification: durable stores differ from pre-import snapshot"
                            .into(),
                    ),
                    Err(error) => errors.push(format!(
                        "rollback verification: strict disk read failed: {error}"
                    )),
                    Ok(_) => {}
                }
            }
            None => errors.push("rollback verification: CampaignStore has no data_dir".into()),
        }
        if !errors.is_empty()
            && let Some(error) = delete_campaign_error
        {
            errors.insert(0, error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    };

    let result = (|| {
        store
            .save_card(card)
            .map_err(|e| TauriCommandError::storage(format!("导入角色卡失败: {e}")))?;

        let conversation = conv_store
            .create_persisted(
                Some(new_card_id.as_str().to_string()),
                Some(new_campaign_id.clone()),
            )
            .map_err(|e| TauriCommandError::storage(format!("导入对话失败: {e}")))?;
        created_conversation_id = Some(conversation.id.clone());
        campaign.conversation_id = Some(conversation.id.clone());
        store
            .save_campaign(campaign)
            .map_err(|e| TauriCommandError::storage(format!("导入 Campaign 失败: {e}")))?;
        after_campaign_saved()?;

        let mut instance_count = 0;
        for instance in rewritten_instances {
            store
                .add_instance(instance)
                .map_err(|e| TauriCommandError::storage(format!("导入角色实例失败: {e}")))?;
            instance_count += 1;
        }

        let knowledge_count = rewritten_knowledge.len();
        store
            .add_knowledge(rewritten_knowledge)
            .map_err(|e| TauriCommandError::storage(format!("导入知识失败: {e}")))?;

        let mut task_count = 0;
        for task in rewritten_tasks {
            store
                .add_task(task)
                .map_err(|e| TauriCommandError::storage(format!("导入任务失败: {e}")))?;
            task_count += 1;
        }

        let mut summary_count = 0;
        for mut summary in rewritten_summaries {
            summary.conversation_id = conversation.id.clone();
            // Use id-keyed insert so B/C stage summaries are not clobbered by leaf A
            // entries that share the same turn span start.
            match store.insert_stage_summary(summary) {
                Ok(campaign_store::UpsertResult::Inserted)
                | Ok(campaign_store::UpsertResult::AlreadyPresent) => {
                    summary_count += 1;
                }
                Ok(campaign_store::UpsertResult::Conflict(msg)) => {
                    return Err(TauriCommandError::storage(format!("导入摘要冲突: {msg}")));
                }
                Err(e) => {
                    return Err(TauriCommandError::storage(format!("导入摘要失败: {e}")));
                }
            }
        }

        tracing::info!(
            "Imported Campaign Bundle {} -> {}",
            old_campaign_id,
            new_campaign_id
        );

        Ok(CampaignImportResult {
            campaign_id: new_campaign_id.as_str().to_string(),
            card_id: new_card_id.as_str().to_string(),
            conversation_id: conversation.id.as_str().to_string(),
            instance_count,
            knowledge_count,
            task_count,
            summary_count,
        })
    })();

    match result {
        Ok(ok) => Ok(ok),
        Err(err) => match rollback(&created_conversation_id) {
            Ok(()) => Err(err),
            Err(rollback_err) => Err(TauriCommandError::storage(format!(
                "导入失败且回滚未完全验证: 原始错误={err}; 回滚={rollback_err}"
            ))),
        },
    }
}

/// 把 CharacterKnowledgeEntry 列表转为 ST WorldInfoBook（导出用）
pub(crate) fn knowledge_to_st_book(
    entries: &[storyforge_domain::character_knowledge::CharacterKnowledgeEntry],
) -> storyforge_domain::character::StWorldInfoBook {
    use storyforge_domain::character::{StWorldInfoBook, StWorldInfoEntry};

    let st_entries: Vec<StWorldInfoEntry> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| StWorldInfoEntry {
            id: Some(i as i32 + 1),
            keys: vec![e.knowledge_text.chars().take(20).collect()],
            key_alias: None,
            secondary_keys: None,
            keysecondary_alias: None,
            content: Some(e.knowledge_text.clone()),
            constant: e.pinned,   // pinned → 蓝灯（常驻）
            selective: !e.pinned, // 非 pinned → 绿灯（选择性）
            selective_logic: None,
            position: Some(serde_json::json!(0)),
            disable: None,
            order: Some(100),
            depth: Some(2),
            extensions: serde_json::json!({}),
            extra: Default::default(),
        })
        .collect();

    StWorldInfoBook {
        entries: st_entries,
        extra: Default::default(),
    }
}

/// 文件名清理（去除不合法字符）
pub(crate) fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c => c,
        })
        .collect()
}
