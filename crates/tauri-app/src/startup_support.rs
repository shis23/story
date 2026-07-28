use super::*;

pub(crate) fn stored_info_to_character(
    stored: &storage::StoredCharacter,
) -> storyforge_domain::character::Character {
    let embedded_world_info = stored
        .info
        .embedded_world_info
        .clone()
        .or_else(|| world_info_book_from_entries(&stored.info.world_info_entries));

    storyforge_domain::character::Character {
        id: Id::from_str(
            stored
                .info
                .source_character_id
                .as_deref()
                .unwrap_or(&stored.id),
        ),
        name: stored.info.name.clone(),
        description: stored.info.description.clone(),
        personality: stored.info.personality.clone(),
        scenario: stored.info.scenario.clone(),
        first_mes: stored.info.first_mes.clone(),
        mes_example: stored.info.mes_example.clone(),
        system_prompt: stored.info.system_prompt.clone(),
        post_history_instructions: stored.info.post_history_instructions.clone(),
        tags: stored.info.tags.clone(),
        creator: stored.info.creator.clone(),
        character_version: stored.info.character_version.clone(),
        alternate_greetings: stored.info.alternate_greetings.clone(),
        embedded_world_info,
        extensions: stored.info.extensions.clone(),
        renderable_assets: stored.info.renderable_assets.clone(),
        source: storyforge_domain::Source::Native,
        spec_version: stored.info.spec_version.clone(),
        raw_card_json: stored.info.raw_card_json.clone(),
    }
}

pub(crate) fn world_info_entry_from_info(
    e: &WorldInfoEntryInfo,
) -> storyforge_domain::world_info::WorldInfoEntry {
    use storyforge_domain::world_info::{LoreRoute, WorldInfoEntry};

    let route = match e.route.as_str() {
        "Constant" => LoreRoute::Constant,
        "Selective" => LoreRoute::Selective,
        "Both" => LoreRoute::Both,
        "Disabled" => LoreRoute::Disabled,
        _ => LoreRoute::Selective,
    };

    WorldInfoEntry {
        st_id: None,
        keys: e.keys.clone(),
        secondary_keys: vec![],
        content: e.content.clone(),
        constant: e.constant,
        selective: !e.constant,
        selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
        disabled: false,
        position: 0,
        depth: e.depth,
        order: e.order,
        route,
        extensions: serde_json::json!({}),
        extra: Default::default(),
    }
}

pub(crate) fn world_info_book_from_entries(
    entries: &[WorldInfoEntryInfo],
) -> Option<storyforge_domain::world_info::WorldInfoBook> {
    if entries.is_empty() {
        return None;
    }

    Some(storyforge_domain::world_info::WorldInfoBook {
        entries: entries.iter().map(world_info_entry_from_info).collect(),
        source: storyforge_domain::Source::Native,
        metadata: Default::default(),
    })
}

/// 收集世界书条目：当前活跃角色的全部 + 其他角色的 is_global 条目
///
/// `active_name` = 当前激活的角色卡名（来自 tool_ctx.characters 的最后一个）。
/// 返回的 WorldInfoBook 包含所有应生效的条目（含全局共享的）。
pub(crate) fn collect_world_info_for_active(
    all_chars: &[storage::StoredCharacter],
    active_name: &str,
) -> storyforge_domain::world_info::WorldInfoBook {
    use storyforge_domain::world_info::WorldInfoBook;

    let mut entries = Vec::new();
    for stored in all_chars {
        let is_active = stored.info.name == active_name;
        for e in &stored.info.world_info_entries {
            if !is_active && !e.is_global {
                continue;
            }
            entries.push(world_info_entry_from_info(e));
        }
    }

    WorldInfoBook {
        entries,
        source: storyforge_domain::Source::Native,
        metadata: Default::default(),
    }
}
