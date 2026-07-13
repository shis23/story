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

pub fn find_permission_entry(name: &str) -> Option<&'static CompatEntry> {
    PERMISSION_COMPAT_MATRIX
        .iter()
        .find(|entry| entry.name == name)
}

pub fn unsupported_event_names() -> &'static [&'static str] {
    INTENTIONALLY_UNSUPPORTED_ST_EVENTS
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
}
