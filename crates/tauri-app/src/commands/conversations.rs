use super::super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummaryDto {
    pub id: String,
    pub character_id: Option<String>,
    pub campaign_id: Option<String>,
    /// \u{5173}\u{8054}\u{89d2}\u{8272}\u{5361}\u{540d}\u{ff08}\u{524d}\u{7aef}\u{5217}\u{8868}\u{663e}\u{793a}\u{7528}\u{ff09}
    pub card_name: Option<String>,
    pub message_count: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[tauri::command]
pub(crate) fn list_conversations(
    state: tauri::State<'_, Arc<AppState>>,
) -> Vec<ConversationSummaryDto> {
    // \u{8054}\u{67e5}\u{89d2}\u{8272}\u{5361}\u{540d}\u{3002}
    // conversation.character_id \u{5b58}\u{7684}\u{662f} CharacterCard.id\u{ff08}\u{800c}\u{975e} domain Character \u{7684}
    // source_character_id\u{ff09}\u{ff0c}\u{6240}\u{4ee5}\u{5fc5}\u{987b}\u{7528} campaign_store \u{7684}\u{5361}\u{7247}\u{8868}\u{6309} card.id \u{8054}\u{67e5},
    // \u{4e0d}\u{80fd}\u{7528} tool_ctx.characters\u{ff08}\u{90a3}\u{662f}\u{6241}\u{5e73} Character,id=source_character_id\u{ff09}\u{3002}
    // \u{515c}\u{5e95}:character_id \u{8054}\u{67e5}\u{4e0d}\u{5230}\u{65f6},\u{8d70} campaign_id \u{2192} campaign.card_id \u{2192} card.name\u{3002}
    let store = get_campaign_store();
    let cards = store.list_cards();
    let card_by_id: std::collections::HashMap<&Id, &str> = cards
        .iter()
        .map(|sc| (&sc.card.id, sc.card.name.as_str()))
        .collect();
    state
        .conv_store
        .list()
        .into_iter()
        .map(|c| {
            let card_name = c
                .character_id
                .as_ref()
                .and_then(|cid| {
                    // \u{9996}\u{9009}:\u{76f4}\u{63a5}\u{6309} character_id(=CharacterCard.id)\u{67e5}\u{5361}\u{540d}
                    let cid_id = Id::from_str(cid);
                    card_by_id.get(&cid_id).map(|n| (*n).to_string())
                })
                .or_else(|| {
                    // \u{515c}\u{5e95}:campaign_id \u{2192} campaign.card_id \u{2192} card.name
                    c.campaign_id.as_ref().and_then(|camp_id| {
                        store.get_campaign(camp_id).and_then(|campaign| {
                            card_by_id.get(&campaign.card_id).map(|n| (*n).to_string())
                        })
                    })
                });
            ConversationSummaryDto {
                id: c.id.to_string(),
                character_id: c.character_id,
                campaign_id: c.campaign_id.map(|id| id.to_string()),
                card_name,
                message_count: c.message_count,
                created_at: c.created_at.to_rfc3339(),
                updated_at: c.updated_at.to_rfc3339(),
            }
        })
        .collect()
}

/// \u{5220}\u{9664}\u{6574}\u{4e2a}\u{4f1a}\u{8bdd}\u{3002}
///
/// \u{4e00} Campaign \u{4e00}\u{5bf9}\u{8bdd}\u{ff1a}\u{82e5}\u{8be5}\u{4f1a}\u{8bdd}\u{7ed1}\u{5b9a}\u{4e86} Campaign\u{ff08}\u{6216}\u{67d0} Campaign \u{7684} conversation_id \u{6307}\u{5411}\u{5b83}\u{ff09}\u{ff0c}
/// \u{5219}\u{6309} **\u{6574}\u{5c40}\u{6d3b}\u{52a8}** \u{7ea7}\u{8054}\u{5220}\u{9664}\u{ff08}\u{5b9e}\u{4f8b}/\u{77e5}\u{8bc6}/\u{4efb}\u{52a1}/\u{603b}\u{7ed3} + \u{4f1a}\u{8bdd}\u{ff09}\u{ff0c}\u{800c}\u{4e0d}\u{662f}\u{53ea}\u{6e05}\u{6d88}\u{606f}\u{6811}\u{3002}
#[tauri::command]
pub(crate) fn delete_conversation(
    conversation_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    if sqlite_runtime::is_sqlite_active() {
        return Err(TauriCommandError::validation(
            "conversation/campaign deletion is not available in the SQLite opt-in backend yet"
                .to_string(),
        ));
    }
    let conv_id = Id::from_str(&conversation_id);
    let store = get_campaign_store();

    // \u{4f18}\u{5148}\u{ff1a}\u{4f1a}\u{8bdd}\u{81ea}\u{5df1}\u{8bb0}\u{5f55}\u{7684} campaign_id
    let campaign_id = state
        .conv_store
        .get(&conv_id)
        .and_then(|c| c.campaign_id.clone())
        // \u{515c}\u{5e95}\u{ff1a}Campaign.conversation_id \u{53cd}\u{5411}\u{6307}\u{5411}\u{ff08}\u{60ac}\u{7a7a}/\u{534a}\u{7ed1}\u{5b9a}\u{65f6}\u{ff09}
        .or_else(|| {
            store
                .list_campaigns()
                .into_iter()
                .find(|c| c.conversation_id.as_ref() == Some(&conv_id))
                .map(|c| c.id)
        });

    if let Some(campaign_id) = campaign_id {
        return delete_campaign_playthrough_in_store(
            store,
            state.conv_store.as_ref(),
            state.inner().as_ref(),
            &campaign_id,
        );
    }

    // \u{65e0} Campaign \u{7684}\u{9057}\u{7559}/\u{5b64}\u{513f}\u{4f1a}\u{8bdd}\u{ff1a}\u{53ea}\u{5220}\u{5bf9}\u{8bdd}
    state
        .conv_store
        .delete(&conv_id)
        .map_err(|e| TauriCommandError::internal(e.to_string()))
}

/// \u{5220}\u{9664}\u{6574}\u{5c40}\u{6d3b}\u{52a8}\u{ff08}\u{4e00} Campaign \u{4e00}\u{5bf9}\u{8bdd}\u{6a21}\u{578b}\u{7684}\u{771f}\u{76f8}\u{6e90}\u{5220}\u{9664}\u{ff09}\u{3002}
///
/// \u{7ea7}\u{8054}\u{ff1a}instances / knowledge / tasks / round_summaries + \u{7ed1}\u{5b9a}\u{4f1a}\u{8bdd}\u{ff1b}
/// \u{82e5}\u{5220}\u{7684}\u{662f}\u{5f53}\u{524d}\u{6d3b}\u{8dc3}\u{6d3b}\u{52a8}\u{ff0c}\u{6e05}\u{9664} active_campaign \u{6307}\u{9488}\u{3002}
#[tauri::command]
pub(crate) fn delete_campaign(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    if sqlite_runtime::is_sqlite_active() {
        return Err(TauriCommandError::validation(
            "campaign deletion is not available in the SQLite opt-in backend yet".to_string(),
        ));
    }
    let campaign_id = Id::from_str(&id);
    delete_campaign_playthrough_in_store(
        get_campaign_store(),
        state.conv_store.as_ref(),
        state.inner().as_ref(),
        &campaign_id,
    )
}

/// \u{5220}\u{9664}\u{4e00}\u{5c40} playthrough\u{ff1a}Campaign \u{672c}\u{4f53}\u{ff08}\u{542b}\u{5b9e}\u{4f8b}/\u{77e5}\u{8bc6}/\u{4efb}\u{52a1}/\u{603b}\u{7ed3}\u{ff09}+ \u{7ed1}\u{5b9a}\u{4f1a}\u{8bdd} + \u{6d3b}\u{8dc3}\u{6307}\u{9488}\u{3002}
pub(crate) fn delete_campaign_playthrough_in_store(
    store: &campaign_store::CampaignStore,
    conv_store: &ConversationStore,
    state: &AppState,
    campaign_id: &Id,
) -> Result<(), TauriCommandError> {
    let campaign = store.get_campaign(campaign_id).ok_or_else(|| {
        TauriCommandError::not_found(format!(
            "\u{627e}\u{4e0d}\u{5230} campaign id={}",
            campaign_id.as_str()
        ))
    })?;

    // \u{6536}\u{96c6}\u{5e94}\u{5220}\u{9664}\u{7684}\u{4f1a}\u{8bdd} id\u{ff1a}Campaign \u{7ed1}\u{5b9a} + \u{53cd}\u{5411} campaign_id \u{5339}\u{914d}\u{ff08}\u{9632}\u{53ea}\u{7ed1}\u{4e00}\u{8fb9}\u{ff09}
    let mut conversation_ids = std::collections::HashSet::new();
    if let Some(cid) = campaign.conversation_id.clone() {
        conversation_ids.insert(cid);
    }
    if let Some(found) = conv_store.find_by_campaign(campaign_id) {
        conversation_ids.insert(found.id);
    }

    // \u{5148}\u{5220} Campaign\u{ff08}\u{7ea7}\u{8054} P2 \u{96c6}\u{5408}\u{ff09}\u{ff0c}\u{518d}\u{5220}\u{4f1a}\u{8bdd}\u{ff0c}\u{907f}\u{514d}\u{5199}\u{4e00}\u{534a}\u{7559}\u{4e0b}\u{6d3b}\u{52a8}
    let deleted = store.delete_campaign(campaign_id).map_err(|e| {
        TauriCommandError::storage(format!(
            "\u{5220}\u{9664}\u{6d3b}\u{52a8}\u{5931}\u{8d25}: {e}"
        ))
    })?;
    if !deleted {
        return Err(TauriCommandError::not_found(format!(
            "\u{627e}\u{4e0d}\u{5230} campaign id={}",
            campaign_id.as_str()
        )));
    }

    for conv_id in conversation_ids {
        if let Err(e) = conv_store.delete(&conv_id) {
            tracing::warn!(
                "\u{5220}\u{9664}\u{6d3b}\u{52a8} {} \u{540e}\u{6e05}\u{7406}\u{4f1a}\u{8bdd} {} \u{5931}\u{8d25}: {e}",
                campaign_id.as_str(),
                conv_id.as_str()
            );
            return Err(TauriCommandError::storage(format!(
                "\u{6d3b}\u{52a8}\u{5df2}\u{5220}\u{9664}\u{ff0c}\u{4f46}\u{6e05}\u{7406}\u{4f1a}\u{8bdd}\u{5931}\u{8d25}: {e}"
            )));
        }
    }

    // \u{6e05}\u{6d3b}\u{8dc3}\u{6307}\u{9488}
    let mut active = state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if active.as_ref() == Some(campaign_id) {
        *active = None;
        if !sqlite_runtime::is_sqlite_active() {
            save_active_campaign(&state.data_dir, None);
        }
    }

    Ok(())
}

#[tauri::command]
pub(crate) fn get_conversation(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, TauriCommandError> {
    let conv_id = storyforge_domain::Id::from_str(&id);
    let conversation = state.conv_store.get(&conv_id).ok_or_else(|| {
        TauriCommandError::not_found(format!("\u{5bf9}\u{8bdd}\u{4e0d}\u{5b58}\u{5728}: {id}"))
    })?;
    let regex_scripts = collect_conversation_regex_scripts(&conversation, state.inner().as_ref());
    Ok(
        serde_json::to_value(conversation_display_dto(&conversation, &regex_scripts))
            .unwrap_or_default(),
    )
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConversationDisplayDto {
    pub(crate) id: Id,
    pub(crate) character_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) campaign_id: Option<Id>,
    pub(crate) nodes: Vec<MessageNodeDisplayDto>,
    pub(crate) created_at: chrono::DateTime<Utc>,
    pub(crate) updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct MessageNodeDisplayDto {
    pub(crate) id: Id,
    pub(crate) parent_id: Option<Id>,
    pub(crate) variants: Vec<MessageVariantDisplayDto>,
    pub(crate) active_variant: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct MessageVariantDisplayDto {
    pub(crate) id: Id,
    pub(crate) role: ConversationRole,
    pub(crate) content: String,
    pub(crate) display_content: String,
    pub(crate) created_at: chrono::DateTime<Utc>,
    pub(crate) status: VariantStatus,
    pub(crate) provenance: Option<Provenance>,
}

pub(crate) fn collect_conversation_regex_scripts(
    conversation: &Conversation,
    state: &AppState,
) -> Vec<RegexScript> {
    let scoped_scripts = if let Some(campaign_id) = &conversation.campaign_id {
        collect_campaign_scoped_regex_scripts(campaign_id, get_campaign_store())
    } else {
        let tool_snapshot = state.snapshot_tool_ctx();
        collect_scoped_regex_scripts(
            conversation.character_id.as_deref(),
            &tool_snapshot.characters,
        )
    };

    merge_runtime_regex_scripts(scoped_scripts, get_preset_store(), get_global_regex_store())
}

pub(crate) fn conversation_display_dto(
    conversation: &Conversation,
    regex_scripts: &[RegexScript],
) -> ConversationDisplayDto {
    let display_scripts = display_only_regex_scripts(regex_scripts);
    ConversationDisplayDto {
        id: conversation.id.clone(),
        character_id: conversation.character_id.clone(),
        campaign_id: conversation.campaign_id.clone(),
        nodes: conversation
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let depth = conversation.nodes.len().saturating_sub(index + 1);
                message_node_display_dto(node, &display_scripts, depth)
            })
            .collect(),
        created_at: conversation.created_at,
        updated_at: conversation.updated_at,
    }
}

pub(crate) fn message_node_display_dto(
    node: &MessageNode,
    display_scripts: &[RegexScript],
    depth: usize,
) -> MessageNodeDisplayDto {
    MessageNodeDisplayDto {
        id: node.id.clone(),
        parent_id: node.parent_id.clone(),
        variants: node
            .variants
            .iter()
            .map(|variant| message_variant_display_dto(variant, display_scripts, depth))
            .collect(),
        active_variant: node.active_variant,
    }
}

pub(crate) fn message_variant_display_dto(
    variant: &MessageVariant,
    display_scripts: &[RegexScript],
    depth: usize,
) -> MessageVariantDisplayDto {
    MessageVariantDisplayDto {
        id: variant.id.clone(),
        role: variant.role.clone(),
        content: variant.content.clone(),
        display_content: render_variant_display_content(variant, display_scripts, depth),
        created_at: variant.created_at,
        status: variant.status.clone(),
        // \u{666e}\u{901a}\u{4f1a}\u{8bdd}\u{8bfb}\u{53d6}\u{53ea}\u{8fd4}\u{56de}\u{91cd} roll \u{6240}\u{9700}\u{7684}\u{975e}\u{654f}\u{611f}\u{6eaf}\u{6e90}\u{3002}reasoning \u{539f}\u{6587}\u{4ec5}\u{7531}
        // meta_explain_generation \u{663e}\u{5f0f}\u{5ba1}\u{8ba1}\u{547d}\u{4ee4}\u{6309}\u{9700}\u{8fd4}\u{56de}\u{ff0c}\u{907f}\u{514d}\u{9875}\u{9762}\u{52a0}\u{8f7d}\u{5373}\u{4e0b}\u{53d1}\u{3002}
        provenance: variant.provenance.clone().map(|mut provenance| {
            provenance.director_reasoning = None;
            provenance.writer_reasoning = None;
            provenance.editor_reasoning = None;
            for subagent in &mut provenance.subagent_results {
                subagent.reasoning_content = None;
            }
            provenance
        }),
    }
}

pub(crate) fn display_only_regex_scripts(regex_scripts: &[RegexScript]) -> Vec<RegexScript> {
    regex_scripts
        .iter()
        .filter(|script| script.markdown_only.unwrap_or(false))
        .cloned()
        .collect()
}

pub(crate) fn render_variant_display_content(
    variant: &MessageVariant,
    display_scripts: &[RegexScript],
    depth: usize,
) -> String {
    if variant.role != ConversationRole::Assistant || display_scripts.is_empty() {
        return variant.content.clone();
    }

    let reasoning_applied = apply_reasoning_regex_to_think_blocks_at_depth(
        &variant.content,
        display_scripts,
        RegexExecutionTarget::Display,
        depth,
    )
    .and_then(|text| {
        apply_regex_scripts_for_target_at_depth(
            &text,
            display_scripts,
            RegexPlacement::Output,
            RegexExecutionTarget::Display,
            depth,
        )
    });

    reasoning_applied.unwrap_or_else(|e| {
        tracing::warn!("\u{5c55}\u{793a}\u{6b63}\u{5219}\u{6267}\u{884c}\u{5931}\u{8d25}\u{ff0c}\u{4f7f}\u{7528}\u{539f}\u{59cb}\u{6d88}\u{606f}\u{5185}\u{5bb9}: {e}");
        variant.content.clone()
    })
}
