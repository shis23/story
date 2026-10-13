//! MVU 原生状态面板与交互按钮（DSH 插件档 2）。
//!
//! 读：Campaign / 实例变量树 + 卡的 MVU 翻译产物（`ui_bindings`、`interactions` 等），
//! 面板怎么画、交互怎么解释由插件按桌面前端同口径算（本层不解释 `value_expr`）。
//! 写：手动分析（会调用一次底层模型）、把翻译出的变量定义应用到卡内角色、按插件
//! 算好的计划写变量——都先等上一轮收尾结束再执行（与写作同一把锁），写变量走与
//! Tauri `set_*_variable` 同一个入口（活动 Turn 屏障 + 键名守卫）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use storyforge_app_service::backend_workflows::{
    apply_mvu_schema_for_backend, preview_mvu_apply_for_backend,
};
use storyforge_app_service::services::meta_typed::analyze_mvu_card_for_host;
use storyforge_app_service::services::variables::{
    set_campaign_variable_for_host, set_character_variable_for_host,
};
use storyforge_app_service::set_active_campaign_for_host;
use storyforge_domain::Id;
use storyforge_domain::mvu_translation::{InteractionMapping, MvuRouting, UiBinding};
use storyforge_domain::variables::VariableValue;

use super::{BridgeError, BridgeService};
use crate::llm::UsageTotals;

/// 一次最多写这么多个变量（一个交互按钮通常一两个）。
pub const MAX_VARIABLE_WRITES: usize = 32;

/// 翻译产物里面板用得上的部分（兜底片段不带原始 JS）。
#[derive(Debug, Clone, Serialize)]
pub struct MvuTranslationView {
    pub ui_bindings: Vec<UiBinding>,
    pub interactions: Vec<InteractionMapping>,
    pub update_rules: Vec<String>,
    pub fallback_fragments: Vec<FallbackSummary>,
    pub routing: MvuRouting,
    pub analysis_confidence: f64,
    pub notes: Vec<String>,
    pub variable_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FallbackSummary {
    pub description: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MvuDefinition {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MvuInstance {
    pub id: String,
    pub name: String,
    pub definition_id: Option<String>,
    pub is_temporary: bool,
    pub variables: Vec<VariableValue>,
}

/// 还没应用到卡内角色的变量定义（每个有变化的角色定义一条）。
#[derive(Debug, Clone, Serialize)]
pub struct PendingApply {
    pub definition_id: String,
    pub definition_name: String,
    pub added: Vec<String>,
    pub overwritten: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MvuStatus {
    pub campaign_id: String,
    pub card_name: String,
    pub source_character_id: String,
    pub analyzed_at: Option<String>,
    pub translation: Option<MvuTranslationView>,
    pub definitions: Vec<MvuDefinition>,
    pub instances: Vec<MvuInstance>,
    pub campaign_variables: Vec<VariableValue>,
    pub pending_apply: Vec<PendingApply>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MvuAnalyzed {
    pub status: MvuStatus,
    /// 分析没产出任何绑定、交互或更新规则（多半是模型调用失败后分析器降级成了字段级 schema）。
    pub empty: bool,
    pub usage: UsageTotals,
}

/// 一个已解析作用域的变量写入（作用域与键名前缀由插件按桌面 `persistShellVariableWrite` 解析）。
#[derive(Debug, Clone, Deserialize)]
pub struct VariableWrite {
    pub scope: WriteScope,
    #[serde(default)]
    pub instance_id: Option<String>,
    pub key: String,
    pub value: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteScope {
    Campaign,
    Instance,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    pub ok: bool,
    pub scope: WriteScope,
    pub instance_id: Option<String>,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VariablesWritten {
    pub results: Vec<WriteResult>,
    pub status: MvuStatus,
}

impl BridgeService {
    /// 面板数据（只读，不占写作锁）。
    pub fn mvu_status(&self, campaign_id: &str) -> Result<MvuStatus, BridgeError> {
        let storage = self.app.storage();
        let campaign_id = Id::from_str(campaign_id);
        let record = storage
            .get_campaign(&campaign_id)
            .map_err(|error| BridgeError::new("storage", error))?
            .ok_or_else(|| BridgeError::new("not_found", format!("找不到存档 {campaign_id}")))?;
        let card = storage
            .get_card(&record.campaign.card_id)
            .map_err(|error| BridgeError::new("storage", error))?
            .ok_or_else(|| {
                BridgeError::new(
                    "not_found",
                    format!("找不到存档的角色卡 {}", record.campaign.card_id),
                )
            })?
            .card;
        let source = card.source_character_id.clone();
        let stored = storage
            .get_mvu(&source)
            .map_err(|error| BridgeError::new("storage", error))?;
        let pending_apply = match stored {
            Some(_) => preview_mvu_apply_for_backend(storage, &source)
                .map_err(|error| BridgeError::new("storage", error))?
                .into_iter()
                .filter(|preview| preview.has_changes)
                .map(|preview| PendingApply {
                    definition_name: card
                        .character_definitions
                        .iter()
                        .find(|definition| definition.id.as_str() == preview.definition_id)
                        .map(|definition| definition.name.clone())
                        .unwrap_or_default(),
                    definition_id: preview.definition_id,
                    added: preview.added_fields.into_iter().map(|f| f.key).collect(),
                    overwritten: preview
                        .overwritten_fields
                        .into_iter()
                        .map(|f| f.key)
                        .collect(),
                })
                .collect(),
            None => Vec::new(),
        };
        let instances = storage
            .list_instances(&campaign_id)
            .map_err(|error| BridgeError::new("storage", error))?
            .into_iter()
            .map(|instance| MvuInstance {
                id: instance.id.to_string(),
                name: instance.name,
                definition_id: instance.definition_id.map(|id| id.to_string()),
                is_temporary: instance.is_temporary,
                variables: instance.variables,
            })
            .collect();
        Ok(MvuStatus {
            campaign_id: campaign_id.to_string(),
            card_name: card.name.clone(),
            source_character_id: source.to_string(),
            analyzed_at: stored.as_ref().map(|stored| stored.analyzed_at.clone()),
            translation: stored.map(|stored| {
                let translation = stored.translation;
                MvuTranslationView {
                    variable_count: translation.variable_schema.len(),
                    ui_bindings: translation.ui_bindings,
                    interactions: translation.interactions,
                    update_rules: translation.update_rules,
                    fallback_fragments: translation
                        .fallback_fragments
                        .into_iter()
                        .map(|fragment| FallbackSummary {
                            description: fragment.description,
                            reason: fragment.reason,
                        })
                        .collect(),
                    routing: translation.routing,
                    analysis_confidence: translation.analysis_confidence,
                    notes: translation.notes,
                }
            }),
            definitions: card
                .character_definitions
                .iter()
                .map(|definition| MvuDefinition {
                    id: definition.id.to_string(),
                    name: definition.name.clone(),
                })
                .collect(),
            instances,
            campaign_variables: record.campaign.variables,
            pending_apply,
        })
    }

    /// 手动分析存档角色卡的 MVU（D44：不自动跑）。会调用一次底层模型；
    /// 已有翻译时重新分析并覆盖。
    pub async fn analyze_mvu(&self, campaign_id: &str) -> Result<MvuAnalyzed, BridgeError> {
        let _guard = self.acquire().await;
        let usage_before = self.llm.usage();
        let status = self.mvu_status(campaign_id)?;
        // 分析在活跃存档的角色上下文里找卡（与写作同口径）。
        if self.active_campaign().as_ref().map(Id::as_str) != Some(campaign_id) {
            set_active_campaign_for_host(&self.app, campaign_id.to_string())?;
        }
        let detail = analyze_mvu_card_for_host(&self.app, &status.source_character_id).await?;
        let translation = &detail.translation;
        Ok(MvuAnalyzed {
            empty: translation.ui_bindings.is_empty()
                && translation.interactions.is_empty()
                && translation.update_rules.is_empty(),
            status: self.mvu_status(campaign_id)?,
            usage: self.llm.usage().since(&usage_before),
        })
    }

    /// 把翻译出的变量定义应用到卡内每个有变化的角色定义（并给已有实例补默认值）。
    pub async fn apply_mvu(&self, campaign_id: &str) -> Result<MvuStatus, BridgeError> {
        let _guard = self.acquire().await;
        let status = self.mvu_status(campaign_id)?;
        if status.translation.is_none() {
            return Err(BridgeError::new(
                "validation",
                "这张卡还没有分析变量规则，先分析再应用",
            ));
        }
        let source = Id::from_str(&status.source_character_id);
        for pending in &status.pending_apply {
            apply_mvu_schema_for_backend(
                self.app.storage(),
                &source,
                &Id::from_str(&pending.definition_id),
            )
            .map_err(|error| BridgeError::new("storage", error))?;
        }
        self.mvu_status(campaign_id)
    }

    /// 按插件算好的计划写变量：逐条执行，单条失败不影响其它条，结果按顺序返回。
    pub async fn write_variables(
        &self,
        campaign_id: &str,
        writes: &[VariableWrite],
    ) -> Result<VariablesWritten, BridgeError> {
        if writes.len() > MAX_VARIABLE_WRITES {
            return Err(BridgeError::new(
                "invalid_params",
                format!(
                    "一次最多写 {MAX_VARIABLE_WRITES} 个变量（收到 {} 个）",
                    writes.len()
                ),
            ));
        }
        let _guard = self.acquire().await;
        let campaign = Id::from_str(campaign_id);
        let storage = self.app.storage();
        let results = writes
            .iter()
            .map(|write| {
                let outcome = match (write.scope, write.instance_id.as_deref()) {
                    (WriteScope::Campaign, _) => set_campaign_variable_for_host(
                        storage,
                        &campaign,
                        &write.key,
                        write.value.clone(),
                        0,
                    ),
                    (WriteScope::Instance, Some(instance_id)) => set_character_variable_for_host(
                        storage,
                        &campaign,
                        &Id::from_str(instance_id),
                        &write.key,
                        write.value.clone(),
                        0,
                    ),
                    (WriteScope::Instance, None) => Err(
                        storyforge_app_service::error::TauriCommandError::validation(
                            "instance scope requires instanceId",
                        ),
                    ),
                };
                WriteResult {
                    ok: outcome.is_ok(),
                    scope: write.scope,
                    instance_id: write.instance_id.clone(),
                    key: write.key.clone(),
                    error: outcome.err().map(|error| error.to_string()),
                }
            })
            .collect();
        Ok(VariablesWritten {
            results,
            status: self.mvu_status(campaign_id)?,
        })
    }
}
