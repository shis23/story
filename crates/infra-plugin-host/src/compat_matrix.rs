//! Deterministic plugin compatibility inventory for StoryForge host surfaces.
//!
//! This module freezes the backend-visible permission / ST mapping claims so
//! frontend matrix tests and backend permission checks stay aligned.

use crate::Permission;

/// Compatibility support labels shared with the frontend matrix vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatStatus {
    Implemented,
    Alias,
    Derived,
    Shim,
    Degraded,
    IntentionallyUnsupported,
    Noop,
}

/// One matrix row describing a host-facing plugin surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatEntry {
    pub id: &'static str,
    pub surface: &'static str,
    pub name: &'static str,
    pub status: CompatStatus,
    pub reason: Option<&'static str>,
    pub requires_permissions: &'static [Permission],
}

/// Backend permission matrix for plugin host APIs.
pub const PERMISSION_COMPAT_MATRIX: &[CompatEntry] = &[
    CompatEntry {
        id: "perm:read_characters",
        surface: "permission",
        name: "ReadCharacters",
        status: CompatStatus::Implemented,
        reason: None,
        requires_permissions: &[Permission::ReadCharacters],
    },
    CompatEntry {
        id: "perm:read_world_info",
        surface: "permission",
        name: "ReadWorldInfo",
        status: CompatStatus::Implemented,
        reason: None,
        requires_permissions: &[Permission::ReadWorldInfo],
    },
    CompatEntry {
        id: "perm:read_memory",
        surface: "permission",
        name: "ReadMemory",
        status: CompatStatus::Implemented,
        reason: Some("event body redaction depends on this grant"),
        requires_permissions: &[Permission::ReadMemory],
    },
    CompatEntry {
        id: "perm:read_variables",
        surface: "permission",
        name: "ReadVariables",
        status: CompatStatus::Implemented,
        reason: None,
        requires_permissions: &[Permission::ReadVariables],
    },
    CompatEntry {
        id: "perm:write_variables",
        surface: "permission",
        name: "WriteVariables",
        status: CompatStatus::Implemented,
        reason: Some("legacy direct write path"),
        requires_permissions: &[Permission::WriteVariables],
    },
    CompatEntry {
        id: "perm:propose_variable_update",
        surface: "permission",
        name: "ProposeVariableUpdate",
        status: CompatStatus::Implemented,
        reason: Some("preferred over WriteVariables for new plugins"),
        requires_permissions: &[Permission::ProposeVariableUpdate],
    },
    CompatEntry {
        id: "perm:modify_prompt",
        surface: "permission",
        name: "ModifyPrompt",
        status: CompatStatus::Implemented,
        reason: Some("required for prompt hooks"),
        requires_permissions: &[Permission::ModifyPrompt],
    },
    CompatEntry {
        id: "perm:call_llm",
        surface: "permission",
        name: "CallLlm",
        status: CompatStatus::Implemented,
        reason: None,
        requires_permissions: &[Permission::CallLlm],
    },
    CompatEntry {
        id: "perm:network",
        surface: "permission",
        name: "Network",
        status: CompatStatus::Implemented,
        reason: Some("default deny unless declared"),
        requires_permissions: &[Permission::Network],
    },
    CompatEntry {
        id: "perm:notifications",
        surface: "permission",
        name: "Notifications",
        status: CompatStatus::Implemented,
        reason: None,
        requires_permissions: &[Permission::Notifications],
    },
];

/// ST API claims that are intentionally unsupported or degraded on the host.
pub const ST_API_COMPAT_MATRIX: &[CompatEntry] = &[
    CompatEntry {
        id: "stapi:get_context_split",
        surface: "tavernHelper",
        name: "SillyTavern.getContext()",
        status: CompatStatus::Shim,
        reason: Some("split into character/worldInfo/memory helpers"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "stapi:replace_variables",
        surface: "tavernHelper",
        name: "replaceVariables(msg)",
        status: CompatStatus::IntentionallyUnsupported,
        reason: Some("host performs variable substitution centrally"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "stapi:trigger_slash_genraw",
        surface: "slash",
        name: "triggerSlash('/genraw ...')",
        status: CompatStatus::Shim,
        reason: Some("routes through storyforge.llm.generate / CallLlm"),
        requires_permissions: &[Permission::CallLlm],
    },
    CompatEntry {
        id: "stapi:dom_append",
        surface: "pluginHost",
        name: "$('#chat').append(html)",
        status: CompatStatus::IntentionallyUnsupported,
        reason: Some("DOM writes must use ui.mountToSlot"),
        requires_permissions: &[],
    },
];

/// Event names that must never be treated as fully supported ST 99 coverage.
pub const INTENTIONALLY_UNSUPPORTED_ST_EVENTS: &[&str] = &[
    "GENERATE_AFTER_COMBINE_PROMPTS",
    "WORLDINFO_FORCE_ACTIVATE",
    "TOOL_CALLS_PERFORMED",
    "TOOL_CALLS_RENDERED",
    "GROUP_UPDATED",
    "GROUP_MEMBER_DRAFTED",
    "GROUP_WRAPPER_FINISHED",
];

/// Expanded ST event inventory with explicit status + fallback reason.
/// Every non-supported row must carry a reason so reports stay honest.
pub const ST_EVENT_COMPAT_MATRIX: &[CompatEntry] = &[
    CompatEntry {
        id: "evt:APP_READY",
        surface: "events",
        name: "APP_READY",
        status: CompatStatus::Implemented,
        reason: None,
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:CHAT_LOADED",
        surface: "events",
        name: "CHAT_LOADED",
        status: CompatStatus::Implemented,
        reason: None,
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:MESSAGE_RECEIVED",
        surface: "events",
        name: "MESSAGE_RECEIVED",
        status: CompatStatus::Implemented,
        reason: Some("body redacted without ReadMemory"),
        requires_permissions: &[Permission::ReadMemory],
    },
    CompatEntry {
        id: "evt:GENERATION_STARTED",
        surface: "events",
        name: "GENERATION_STARTED",
        status: CompatStatus::Alias,
        reason: Some("maps from pipeline.started"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:COMMITTED_ALIAS",
        surface: "events",
        name: "state_changed→committed",
        status: CompatStatus::Alias,
        reason: Some("closes StateChanged{Committed} ambiguity"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:GENERATE_AFTER_COMBINE_PROMPTS",
        surface: "events",
        name: "GENERATE_AFTER_COMBINE_PROMPTS",
        status: CompatStatus::IntentionallyUnsupported,
        reason: Some("hook_chain_before_and_ready_only"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:WORLDINFO_FORCE_ACTIVATE",
        surface: "events",
        name: "WORLDINFO_FORCE_ACTIVATE",
        status: CompatStatus::IntentionallyUnsupported,
        reason: Some("no_force_activate_ui"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:TOOL_CALLS_PERFORMED",
        surface: "events",
        name: "TOOL_CALLS_PERFORMED",
        status: CompatStatus::IntentionallyUnsupported,
        reason: Some("agent_tools_not_exposed_to_st_plugins"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:GROUP_UPDATED",
        surface: "events",
        name: "GROUP_UPDATED",
        status: CompatStatus::IntentionallyUnsupported,
        reason: Some("no_group_model"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:SETTINGS_LOADED",
        surface: "events",
        name: "SETTINGS_LOADED",
        status: CompatStatus::Noop,
        reason: Some("settings_not_broadcast_as_st_events"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:SAVE_CHAT",
        surface: "tavernHelper",
        name: "saveChat",
        status: CompatStatus::Degraded,
        reason: Some("host adapter injectable; default local_mirror_only_no_host_persist"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:CALL_GENERIC_POPUP",
        surface: "tavernHelper",
        name: "callGenericPopup",
        status: CompatStatus::Degraded,
        reason: Some("no_ui_returns_default_or_null"),
        requires_permissions: &[],
    },
    CompatEntry {
        id: "evt:GET_REQUEST_HEADERS",
        surface: "tavernHelper",
        name: "getRequestHeaders",
        status: CompatStatus::Degraded,
        reason: Some("static_json_content_type_only_no_auth"),
        requires_permissions: &[],
    },
];

pub fn find_permission_entry(name: &str) -> Option<&'static CompatEntry> {
    PERMISSION_COMPAT_MATRIX
        .iter()
        .find(|entry| entry.name == name)
}

pub fn unsupported_event_names() -> &'static [&'static str] {
    INTENTIONALLY_UNSUPPORTED_ST_EVENTS
}

/// Every non-supported ST event / API row must declare a reason.
pub fn non_supported_entries_with_reasons() -> Vec<&'static CompatEntry> {
    ST_EVENT_COMPAT_MATRIX
        .iter()
        .chain(ST_API_COMPAT_MATRIX.iter())
        .filter(|entry| {
            !matches!(
                entry.status,
                CompatStatus::Implemented | CompatStatus::Alias | CompatStatus::Derived
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ST_API_MAPPING;

    #[test]
    fn permission_matrix_covers_every_permission_variant() {
        let names: Vec<&str> = PERMISSION_COMPAT_MATRIX.iter().map(|e| e.name).collect();
        for permission in [
            Permission::ReadCharacters,
            Permission::ReadWorldInfo,
            Permission::ReadMemory,
            Permission::ReadVariables,
            Permission::WriteVariables,
            Permission::ProposeVariableUpdate,
            Permission::ModifyPrompt,
            Permission::CallLlm,
            Permission::Network,
            Permission::Notifications,
        ] {
            let label = format!("{permission:?}");
            assert!(
                names.iter().any(|name| *name == label),
                "missing permission matrix row for {label}"
            );
        }
    }

    #[test]
    fn st_api_mapping_rows_are_non_empty_and_matrix_references_exist() {
        assert!(!ST_API_MAPPING.is_empty());
        assert!(!ST_API_COMPAT_MATRIX.is_empty());
        assert!(
            ST_API_COMPAT_MATRIX
                .iter()
                .any(|entry| entry.status == CompatStatus::IntentionallyUnsupported)
        );
        assert!(
            ST_API_COMPAT_MATRIX
                .iter()
                .any(|entry| entry.status == CompatStatus::Shim)
        );
    }

    #[test]
    fn unsupported_event_inventory_is_explicit() {
        for name in INTENTIONALLY_UNSUPPORTED_ST_EVENTS {
            assert!(!name.is_empty());
            assert!(name.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_'));
        }
        assert!(find_permission_entry("ModifyPrompt").is_some());
        assert_eq!(
            unsupported_event_names().len(),
            INTENTIONALLY_UNSUPPORTED_ST_EVENTS.len()
        );
    }

    #[test]
    fn every_non_supported_event_row_has_a_reason() {
        let rows = non_supported_entries_with_reasons();
        assert!(!rows.is_empty());
        for entry in rows {
            assert!(
                entry.reason.is_some(),
                "non-supported entry {} missing reason",
                entry.id
            );
            assert!(!entry.reason.unwrap().is_empty());
        }
        // Keep the four classifications distinct in the inventory.
        let statuses: Vec<CompatStatus> = ST_EVENT_COMPAT_MATRIX.iter().map(|e| e.status).collect();
        assert!(statuses.contains(&CompatStatus::Implemented));
        assert!(statuses.contains(&CompatStatus::Alias));
        assert!(statuses.contains(&CompatStatus::Degraded));
        assert!(statuses.contains(&CompatStatus::IntentionallyUnsupported));
        assert!(statuses.contains(&CompatStatus::Noop));
    }

    #[test]
    fn committed_alias_row_is_documented() {
        assert!(
            ST_EVENT_COMPAT_MATRIX
                .iter()
                .any(|entry| entry.id == "evt:COMMITTED_ALIAS"
                    && entry.status == CompatStatus::Alias)
        );
    }
}
