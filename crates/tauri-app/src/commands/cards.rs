use super::super::*;

// ─── P1：角色识别 / CharacterCard / Campaign / 角色实例 / 变量 ──────────────

/// 角色（CharacterDefinition）的精简 DTO（前端展示用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterDefinitionDto {
    pub id: String,
    pub name: String,
    pub persona_prompt: String,
    pub behavior_rules: String,
    pub base_backstory: Vec<String>,
    pub group: Option<String>,
    pub role_type: String,
    pub variable_schema: Vec<storyforge_domain::variables::VariableField>,
}

impl From<&storyforge_domain::character::CharacterDefinition> for CharacterDefinitionDto {
    fn from(d: &storyforge_domain::character::CharacterDefinition) -> Self {
        Self {
            id: d.id.as_str().to_string(),
            name: d.name.clone(),
            persona_prompt: d.persona_prompt.clone(),
            behavior_rules: d.behavior_rules.clone(),
            base_backstory: d.base_backstory.clone(),
            group: d.group.clone(),
            role_type: format!("{:?}", d.role_type).to_lowercase(),
            variable_schema: d.variable_schema.clone(),
        }
    }
}

/// CharacterCard 的详情 DTO（含卡内角色定义列表）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardDetailDto {
    pub id: String,
    pub name: String,
    pub source_character_id: String,
    pub first_mes: String,
    pub alternate_greetings: Vec<String>,
    pub character_definitions: Vec<CharacterDefinitionDto>,
    pub campaign_variable_schema: Vec<storyforge_domain::variables::VariableField>,
    pub definition_count: usize,
    pub character_count: usize,
    pub imported_at: String,
    /// 识别是否成功（false = 未识别/历史状态未知/降级 fallback）
    pub extracted: bool,
    pub extraction_status: String,
    pub extraction_message: Option<String>,
}

/// CharacterCard 列表项（轻量）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardSummaryDto {
    pub id: String,
    pub name: String,
    pub source_character_id: String,
    pub definition_count: usize,
    pub character_count: usize,
    pub imported_at: String,
    pub extracted: bool,
    pub extraction_status: String,
    pub extraction_message: Option<String>,
}

impl From<&campaign_store::StoredCard> for CardSummaryDto {
    fn from(s: &campaign_store::StoredCard) -> Self {
        let definition_count = s.card.character_definitions.len();
        Self {
            id: s.card.id.as_str().to_string(),
            name: s.card.name.clone(),
            source_character_id: s.card.source_character_id.as_str().to_string(),
            definition_count,
            character_count: definition_count,
            imported_at: s.imported_at.clone(),
            extracted: s.card.extraction_succeeded(),
            extraction_status: s.card.extraction_status.as_str().to_string(),
            extraction_message: card_extraction_message(&s.card),
        }
    }
}

pub(crate) const FALLBACK_EXTRACTION_MESSAGE: &str = "识别失败，已按单角色处理，可重新识别。";

pub(crate) fn fallback_character_extraction(
    character: &storyforge_domain::character::Character,
    mvu_schema: &[storyforge_domain::variables::VariableField],
) -> (
    Vec<storyforge_domain::character::CharacterDefinition>,
    storyforge_domain::character::CharacterExtractionStatus,
    Option<String>,
) {
    use storyforge_domain::character::{CharacterDefinition, CharacterExtractionStatus};

    (
        vec![CharacterDefinition::fallback_from_character(
            character, mvu_schema,
        )],
        CharacterExtractionStatus::Fallback,
        Some(FALLBACK_EXTRACTION_MESSAGE.to_string()),
    )
}

pub(crate) fn card_extraction_message(
    card: &storyforge_domain::character::CharacterCard,
) -> Option<String> {
    use storyforge_domain::character::CharacterExtractionStatus;

    match card.extraction_status {
        CharacterExtractionStatus::Extracted => card.extraction_message.clone(),
        CharacterExtractionStatus::Fallback => Some(
            card.extraction_message
                .clone()
                .unwrap_or_else(|| FALLBACK_EXTRACTION_MESSAGE.to_string()),
        ),
        CharacterExtractionStatus::Unknown if card.character_definitions.is_empty() => {
            Some("尚未识别角色。".into())
        }
        CharacterExtractionStatus::Unknown => {
            Some("历史角色定义缺少识别状态，可重新识别确认。".into())
        }
    }
}

#[derive(Debug)]
pub(crate) enum CharacterExtractionDecision {
    ReturnExisting(campaign_store::StoredCard),
    Run(storyforge_domain::character::CharacterCard),
}

#[cfg(test)]
pub(crate) fn prepare_character_extraction_card(
    store: &campaign_store::CampaignStore,
    character: &storyforge_domain::character::Character,
    force: bool,
) -> Result<CharacterExtractionDecision, TauriCommandError> {
    match store.get_card_by_source(&character.id) {
        Some(existing) if !force => Ok(CharacterExtractionDecision::ReturnExisting(existing)),
        Some(existing) => {
            if !store.list_campaigns_of_card(&existing.card.id).is_empty() {
                return Err(TauriCommandError::validation(
                    campaign_store::FORCE_RERUN_BLOCKED_BY_CAMPAIGN,
                ));
            }
            Ok(CharacterExtractionDecision::Run(existing.card))
        }
        None => Ok(CharacterExtractionDecision::Run(
            storyforge_domain::character::CharacterCard::from_character(character),
        )),
    }
}

#[tauri::command]
pub fn list_cards(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<CardSummaryDto>, TauriCommandError> {
    let cards = state
        .storage()
        .list_cards()
        .map_err(TauriCommandError::storage)?;
    Ok(cards.iter().map(CardSummaryDto::from).collect())
}

/// 删除角色卡（按 CharacterCard.id，级联删 campaign/instances/mvu）
#[tauri::command]
pub fn delete_card(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let card_id = Id::from_str(&id);
    if !state
        .storage()
        .delete_card(&card_id)
        .map_err(|e| TauriCommandError::storage(format!("存储写入失败: {e}")))?
    {
        return Err(TauriCommandError::not_found(format!("角色卡不存在: {id}")));
    }
    Ok(())
}

#[tauri::command]
pub fn get_card(
    id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<CardDetailDto, TauriCommandError> {
    let stored = state
        .storage()
        .get_card(&Id::from_str(&id))
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到 card id={id}")))?;
    let source_character = state
        .storage()
        .get_character(stored.card.source_character_id.as_str())
        .map_err(TauriCommandError::storage)?;
    let (raw_first_mes, raw_alternate_greetings) = raw_card_greetings(&stored.card.raw_card_json);
    let definition_count = stored.card.character_definitions.len();
    Ok(CardDetailDto {
        id: stored.card.id.as_str().to_string(),
        name: stored.card.name.clone(),
        source_character_id: stored.card.source_character_id.as_str().to_string(),
        first_mes: source_character
            .as_ref()
            .map(|sc| sc.info.first_mes.clone())
            .unwrap_or(raw_first_mes),
        alternate_greetings: source_character
            .as_ref()
            .map(|sc| sc.info.alternate_greetings.clone())
            .unwrap_or(raw_alternate_greetings),
        character_definitions: stored
            .card
            .character_definitions
            .iter()
            .map(CharacterDefinitionDto::from)
            .collect(),
        campaign_variable_schema: stored.card.effective_campaign_variable_schema(),
        definition_count,
        character_count: definition_count,
        imported_at: stored.imported_at.clone(),
        extracted: stored.card.extraction_succeeded(),
        extraction_status: stored.card.extraction_status.as_str().to_string(),
        extraction_message: card_extraction_message(&stored.card),
    })
}

pub(crate) fn raw_card_greetings(raw_card_json: &serde_json::Value) -> (String, Vec<String>) {
    let first_mes = raw_card_json
        .get("first_mes")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let alternate_greetings = raw_card_json
        .get("alternate_greetings")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    (first_mes, alternate_greetings)
}
