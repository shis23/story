//! 卡片界面（DSH 插件档 3）：存档角色卡的壳清单与可代取主机。
//!
//! 插件服务端据此决定消息里的卡片界面能不能自动挂载（桌面 `trustedMessageShellUrls` /
//! `inlineShellTriggers` 同口径：`.load(url)` 只认清单里的 URL，内联文档只认卡正则在原文里的
//! 命中），并用同一份主机白名单代取远程资源、钉壳文档的 CSP。清单不带内联正文——消息里的
//! 界面正文取自展示文本，插件不需要卡里的原始 HTML / JS。
//!
//! 壳自己的界面状态（TavernHelper 选择器桶）经 [`BridgeService::set_card_shell_variables`]
//! 写回存档，与桌面 `set_card_shell_variables` 同一入口。
//!
//! 开场界面（首页 / 自定义开局）：清单带卡的问候语（壳的 `SillyTavern.chat[0].swipes`），
//! 壳选定开场后经 [`BridgeService::apply_opening`] 改写开场白（桌面 `apply_campaign_opening`）；
//! 壳经 TavernHelper 世界书接口只读条目名与启用状态、只改启用状态（桌面
//! `list_campaign_world_info` / `set_campaign_world_info_enabled`）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use storyforge_app_service::services::campaigns::apply_campaign_opening_for_backend;
use storyforge_app_service::services::variables::set_card_shell_variables_for_backend;
use storyforge_app_service::services::world_info::{
    CampaignWorldInfoDto, list_campaign_world_info_for_host,
    set_campaign_world_info_enabled_for_host,
};
use storyforge_app_service::startup_support::stored_info_to_character;
use storyforge_domain::Id;
use storyforge_domain::card_shell::{
    CardShellEntry, CardShellKind, DEFAULT_SHELL_HOSTS, extract_card_shell_manifest,
};

use super::{BridgeError, BridgeService};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellSource {
    RemoteUrl,
    InlineHtml,
    InlineJs,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShellEntry {
    pub kind: CardShellKind,
    pub source: ShellSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub label: String,
    /// 正则壳是卡的 `find_regex`，酒馆助手脚本是 `th-script`。
    pub trigger: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardShellInfo {
    pub campaign_id: String,
    /// 存档名（开场界面 `chat[0].name`，桌面同样取活动名）。
    pub campaign_name: String,
    pub source_character_id: String,
    pub shells: Vec<ShellEntry>,
    /// 卡里出现过的全部远程 URL（壳、依赖、酒馆助手 import）。
    pub remote_urls: Vec<String>,
    pub allowed_hosts: Vec<String>,
    /// 卡的问候语（与桌面 `get_card` 同源：导入时的角色原文）。开场界面按它们切换开场。
    pub first_mes: String,
    pub alternate_greetings: Vec<String>,
}

/// 开场界面选定后存进对话的开场白。
#[derive(Debug, Clone, Serialize)]
pub struct OpeningApplied {
    pub node_id: String,
    /// 存进去的原文（身份宏已展开）。
    pub content: String,
    /// 跑过展示正则之后的文本。
    pub display_content: String,
}

/// 世界书一条的启用状态改动（条目按本局世界书里的位置）。
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct WorldInfoToggle {
    pub entry_index: usize,
    pub enabled: bool,
}

/// 一次最多改的世界书条目数。
pub const MAX_WORLD_INFO_TOGGLES: usize = 500;

impl BridgeService {
    /// 存档角色卡的壳清单（只读，不占写作锁）。
    pub fn card_shell_manifest(&self, campaign_id: &str) -> Result<CardShellInfo, BridgeError> {
        let (source, campaign_name) = self.campaign_source_character(campaign_id)?;
        let stored = self
            .app
            .storage()
            .get_character(&source)
            .map_err(|error| BridgeError::new("storage", error))?
            .ok_or_else(|| BridgeError::new("not_found", format!("找不到角色卡 {source}")))?;
        let manifest = extract_card_shell_manifest(&stored_info_to_character(&stored));
        let shells = manifest
            .shells
            .into_iter()
            .map(|shell| {
                let (source, url) = match shell.entry {
                    CardShellEntry::RemoteUrl { url } => (ShellSource::RemoteUrl, Some(url)),
                    CardShellEntry::InlineHtml { .. } => (ShellSource::InlineHtml, None),
                    CardShellEntry::InlineJs { .. } => (ShellSource::InlineJs, None),
                };
                ShellEntry {
                    kind: shell.kind,
                    source,
                    url,
                    label: shell.label,
                    trigger: shell.trigger,
                }
            })
            .collect();
        Ok(CardShellInfo {
            campaign_id: campaign_id.to_string(),
            campaign_name,
            source_character_id: source,
            shells,
            remote_urls: manifest.remote_urls,
            allowed_hosts: DEFAULT_SHELL_HOSTS
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
            first_mes: stored.info.first_mes.clone(),
            alternate_greetings: stored.info.alternate_greetings.clone(),
        })
    }

    /// 开场界面选定开场：改写还在开场态（对话只有开场白一个节点）的开场白，返回新的
    /// 原文与展示文本。故事开始之后拒绝（validation），与桌面同一个 app-service 入口。
    pub async fn apply_opening(
        &self,
        campaign_id: &str,
        content: String,
    ) -> Result<OpeningApplied, BridgeError> {
        let _guard = self.acquire().await;
        let id = Id::from_str(campaign_id);
        apply_campaign_opening_for_backend(
            self.app.storage(),
            self.app.conv_store.as_ref(),
            &id,
            content,
        )?;
        let conversation = self.campaign_conversation(&id)?;
        let node = conversation
            .nodes
            .first()
            .ok_or_else(|| BridgeError::new("internal", "改写后的对话没有开场白"))?;
        let content = node
            .active()
            .map(|variant| variant.content.clone())
            .unwrap_or_default();
        let display_content = self
            .node_display(&conversation, 0)
            .unwrap_or_else(|| content.clone());
        Ok(OpeningApplied {
            node_id: node.id.to_string(),
            content,
            display_content,
        })
    }

    /// 本局世界书（只读，不占写作锁；本局书还空着时按卡模板拷贝一份）。
    pub fn world_info(&self, campaign_id: &str) -> Result<CampaignWorldInfoDto, BridgeError> {
        Ok(list_campaign_world_info_for_host(
            &self.app,
            &Id::from_str(campaign_id),
        )?)
    }

    /// 逐条改本局世界书的启用状态（不动正文与路由）。占写作锁；遇到第一条失败即停，
    /// 前面的已落盘。返回改了几条。
    pub async fn set_world_info_enabled(
        &self,
        campaign_id: &str,
        toggles: &[WorldInfoToggle],
    ) -> Result<usize, BridgeError> {
        if toggles.len() > MAX_WORLD_INFO_TOGGLES {
            return Err(BridgeError::new(
                "invalid_params",
                format!(
                    "一次最多改 {MAX_WORLD_INFO_TOGGLES} 条世界书（收到 {} 条）",
                    toggles.len()
                ),
            ));
        }
        let _guard = self.acquire().await;
        let id = Id::from_str(campaign_id);
        for toggle in toggles {
            set_campaign_world_info_enabled_for_host(
                &self.app,
                &id,
                toggle.entry_index,
                toggle.enabled,
            )?;
        }
        Ok(toggles.len())
    }

    /// 整体替换存档的卡壳选择器桶（`__storyforge_card_shell_variables`）。插件按存档串行
    /// 读改写后把整份快照交给这里；形状与 2 MiB 上限由 app-service 校验。占写作锁：
    /// 写作进行中会排到这一轮收尾之后。
    pub async fn set_card_shell_variables(
        &self,
        campaign_id: &str,
        variables: Value,
    ) -> Result<(), BridgeError> {
        let _guard = self.acquire().await;
        set_card_shell_variables_for_backend(
            self.app.storage(),
            &Id::from_str(campaign_id),
            variables,
        )?;
        Ok(())
    }

    /// 存档所用角色卡的源角色 id（`StoredCharacter.id` 或导入时记下的 `source_character_id`）
    /// 与存档名。
    pub(crate) fn campaign_source_character(
        &self,
        campaign_id: &str,
    ) -> Result<(String, String), BridgeError> {
        let storage = self.app.storage();
        let record = storage
            .get_campaign(&Id::from_str(campaign_id))
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
            })?;
        Ok((
            card.card.source_character_id.to_string(),
            record.campaign.name,
        ))
    }
}
