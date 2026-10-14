//! 卡片界面（DSH 插件档 3）：存档角色卡的壳清单与可代取主机。
//!
//! 插件服务端据此决定消息里的卡片界面能不能自动挂载（桌面 `trustedMessageShellUrls` /
//! `inlineShellTriggers` 同口径：`.load(url)` 只认清单里的 URL，内联文档只认卡正则在原文里的
//! 命中），并用同一份主机白名单代取远程资源、钉壳文档的 CSP。清单不带内联正文——消息里的
//! 界面正文取自展示文本，插件不需要卡里的原始 HTML / JS。
//!
//! 壳自己的界面状态（TavernHelper 选择器桶）经 [`BridgeService::set_card_shell_variables`]
//! 写回存档，与桌面 `set_card_shell_variables` 同一入口。

use serde::Serialize;
use serde_json::Value;

use storyforge_app_service::services::variables::set_card_shell_variables_for_backend;
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
    pub source_character_id: String,
    pub shells: Vec<ShellEntry>,
    /// 卡里出现过的全部远程 URL（壳、依赖、酒馆助手 import）。
    pub remote_urls: Vec<String>,
    pub allowed_hosts: Vec<String>,
}

impl BridgeService {
    /// 存档角色卡的壳清单（只读，不占写作锁）。
    pub fn card_shell_manifest(&self, campaign_id: &str) -> Result<CardShellInfo, BridgeError> {
        let source = self.campaign_source_character(campaign_id)?;
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
            source_character_id: source,
            shells,
            remote_urls: manifest.remote_urls,
            allowed_hosts: DEFAULT_SHELL_HOSTS
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
        })
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

    /// 存档所用角色卡的源角色 id（`StoredCharacter.id` 或导入时记下的 `source_character_id`）。
    pub(crate) fn campaign_source_character(
        &self,
        campaign_id: &str,
    ) -> Result<String, BridgeError> {
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
        Ok(card.card.source_character_id.to_string())
    }
}
