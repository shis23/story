//! In-process prompt-hook / plugin audit helpers.
//!
//! This module stays free of SQLite / tauri-app storage. It provides:
//! - redacted audit records with no prompt bodies or secrets
//! - local integrity hashes + chain metadata (not a cryptographic seal)
//! - query / filter / pagination / retention
//! - correlation ids for asynchronous plugin operations

use crate::Permission;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Correlation identifier for asynchronous plugin operations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CorrelationId(pub String);

impl CorrelationId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Redacted audit record for plugin host surfaces.
///
/// Never stores raw prompts, messages, credentials, or stacks. Prefer
/// [`AuditRecord::redacted`] so free-form secret-bearing strings cannot be
/// stuffed into identity fields by accident. Fields are private and this type
/// deliberately does not implement `Deserialize`, so untrusted JSON cannot
/// bypass the redacting constructor.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditRecord {
    plugin_id: String,
    event: String,
    stage: String,
    status: String,
    duration_ms: u64,
    correlation_id: Option<CorrelationId>,
    generation_id: Option<String>,
    /// Summary only — never raw content.
    changed_keys: Vec<String>,
    recorded_at_ms: u64,
    /// Integrity hash over sanitized fields. Populated by [`chain_audit_records`].
    /// FNV-1a 32-bit: accidental-corruption detection only, not a crypto seal.
    record_hash: Option<String>,
    /// Previous record hash in the chain. Populated by [`chain_audit_records`].
    prev_hash: Option<String>,
}

fn sanitize_label(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let forbidden = [
        "api_key",
        "apikey",
        "authorization",
        "sf_secret_",
        "password",
        "credential",
        "bearer ",
        "sk-",
        "token=",
        "token:",
        "cookie=",
        "cookie:",
        "set-cookie",
        "secret=",
        "secret:",
        "prompt=",
        "prompt:",
        "private prompt",
        "stack",
    ];
    let lower = trimmed.to_ascii_lowercase();
    if forbidden.iter().any(|token| lower.contains(token)) {
        return format!("<redacted:{}>", fnv1a_hex(trimmed));
    }
    // Keep printable labels bounded; drop control characters.
    trimmed
        .chars()
        .filter(|ch| !ch.is_control())
        .take(96)
        .collect()
}

fn sanitize_status(value: &str) -> String {
    match value {
        "ok" | "no_change" | "timeout" | "cancelled" | "error" | "audit_error" | "missing_host"
        | "unloaded" | "revoked" | "budget_exceeded" => value.to_string(),
        _ => "audit_error".to_string(),
    }
}

/// Input for [`AuditRecord::redacted`]. Keeps the constructor arg count low.
#[derive(Debug, Clone, Default)]
pub struct AuditRecordInput {
    pub plugin_id: String,
    pub event: String,
    pub stage: String,
    pub status: String,
    pub duration_ms: u64,
    pub correlation_id: Option<CorrelationId>,
    pub generation_id: Option<String>,
    pub changed_keys: Vec<String>,
    pub recorded_at_ms: u64,
}

impl AuditRecord {
    /// Controlled constructor that redacts identity labels and rejects secret-
    /// like free text from entering the audit ring.
    pub fn redacted(input: AuditRecordInput) -> Self {
        Self {
            plugin_id: sanitize_label(&input.plugin_id),
            event: sanitize_label(&input.event),
            stage: sanitize_label(&input.stage),
            status: sanitize_status(&input.status),
            duration_ms: input.duration_ms,
            correlation_id: input
                .correlation_id
                .map(|id| CorrelationId::new(sanitize_label(id.as_str()))),
            generation_id: input.generation_id.map(|id| sanitize_label(&id)),
            changed_keys: input
                .changed_keys
                .into_iter()
                .take(128)
                .map(|key| sanitize_label(&key))
                .collect(),
            recorded_at_ms: input.recorded_at_ms,
            record_hash: None,
            prev_hash: None,
        }
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn event(&self) -> &str {
        &self.event
    }

    pub fn stage(&self) -> &str {
        &self.stage
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub fn correlation_id(&self) -> Option<&CorrelationId> {
        self.correlation_id.as_ref()
    }

    pub fn generation_id(&self) -> Option<&str> {
        self.generation_id.as_deref()
    }

    pub fn changed_keys(&self) -> &[String] {
        &self.changed_keys
    }

    pub fn recorded_at_ms(&self) -> u64 {
        self.recorded_at_ms
    }

    pub fn record_hash(&self) -> Option<&str> {
        self.record_hash.as_deref()
    }

    pub fn prev_hash(&self) -> Option<&str> {
        self.prev_hash.as_deref()
    }
}

/// Result of checking a stored local audit-chain segment.
///
/// FNV-1a only detects accidental mutation or reordering. This result must not
/// be presented as a cryptographic tamper-proof guarantee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditChainVerification {
    pub valid: bool,
    pub record_count: usize,
    pub invalid_index: Option<usize>,
    pub reason: Option<&'static str>,
    pub head_hash: Option<String>,
}

/// Query filters for audit records.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    pub plugin_id: Option<String>,
    pub event: Option<String>,
    pub stage: Option<String>,
    pub status: Option<String>,
    pub correlation_id: Option<String>,
    pub generation_id: Option<String>,
    pub min_duration_ms: Option<u64>,
    pub max_duration_ms: Option<u64>,
    pub since_ms: Option<u64>,
    pub until_ms: Option<u64>,
}

/// Pagination / ordering for audit records.
#[derive(Debug, Clone)]
pub struct AuditPage {
    pub limit: usize,
    pub offset: usize,
    pub order_by: AuditOrderBy,
    pub descending: bool,
}

impl Default for AuditPage {
    fn default() -> Self {
        Self {
            limit: 100,
            offset: 0,
            order_by: AuditOrderBy::RecordedAt,
            descending: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOrderBy {
    RecordedAt,
    DurationMs,
    PluginId,
    Event,
    Status,
}

/// FNV-1a 32-bit over a string. Deterministic and free of crypto deps.
fn fnv1a_hex(input: &str) -> String {
    let mut hash: u32 = 2_166_136_261;
    for byte in input.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    format!("{hash:08x}")
}

/// Compute an integrity hash over sanitized record fields.
///
/// Does not include secrets or raw prompt bodies because those fields are
/// never present on [`AuditRecord`] when built via [`AuditRecord::redacted`].
/// FNV-1a 32-bit: local accidental-corruption detection only (no trusted head).
pub fn compute_audit_record_hash(record: &AuditRecord, prev_hash: Option<&str>) -> String {
    let mut material = String::new();
    material.push_str(prev_hash.unwrap_or(""));
    material.push('|');
    material.push_str(&record.plugin_id);
    material.push('|');
    material.push_str(&record.event);
    material.push('|');
    material.push_str(&record.stage);
    material.push('|');
    material.push_str(&record.status);
    material.push('|');
    material.push_str(&record.duration_ms.to_string());
    material.push('|');
    material.push_str(
        record
            .correlation_id
            .as_ref()
            .map(CorrelationId::as_str)
            .unwrap_or(""),
    );
    material.push('|');
    material.push_str(record.generation_id.as_deref().unwrap_or(""));
    material.push('|');
    material.push_str(&record.changed_keys.join(","));
    material.push('|');
    material.push_str(&record.recorded_at_ms.to_string());
    fnv1a_hex(&material)
}

/// Chain records with `record_hash` / `prev_hash`. Deterministic.
pub fn chain_audit_records(records: &[AuditRecord]) -> Vec<AuditRecord> {
    let mut prev: Option<String> = None;
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let mut next = record.clone();
        let hash = compute_audit_record_hash(&next, prev.as_deref());
        next.prev_hash = prev.clone();
        next.record_hash = Some(hash.clone());
        prev = Some(hash);
        out.push(next);
    }
    out
}

/// Verify the hashes and links already stored in an audit-chain segment.
///
/// The first record may retain a `prev_hash` from an older entry removed by
/// bounded retention; its own hash is still verifiable against that anchor.
/// This function intentionally never re-chains or mutates its input.
pub fn verify_audit_record_chain(records: &[AuditRecord]) -> AuditChainVerification {
    let mut previous_hash: Option<String> = None;
    for (index, record) in records.iter().enumerate() {
        let Some(record_hash) = record.record_hash.as_deref() else {
            return AuditChainVerification {
                valid: false,
                record_count: records.len(),
                invalid_index: Some(index),
                reason: Some("missing_record_hash"),
                head_hash: previous_hash,
            };
        };
        if index > 0 && record.prev_hash.as_deref() != previous_hash.as_deref() {
            return AuditChainVerification {
                valid: false,
                record_count: records.len(),
                invalid_index: Some(index),
                reason: Some("previous_hash_mismatch"),
                head_hash: previous_hash,
            };
        }
        let expected = compute_audit_record_hash(record, record.prev_hash.as_deref());
        if record_hash != expected {
            return AuditChainVerification {
                valid: false,
                record_count: records.len(),
                invalid_index: Some(index),
                reason: Some("record_hash_mismatch"),
                head_hash: previous_hash,
            };
        }
        previous_hash = Some(record_hash.to_string());
    }
    AuditChainVerification {
        valid: true,
        record_count: records.len(),
        invalid_index: None,
        reason: None,
        head_hash: previous_hash,
    }
}

/// Filter audit records.
pub fn query_audit_records(records: &[AuditRecord], query: &AuditQuery) -> Vec<AuditRecord> {
    records
        .iter()
        .filter(|record| matches_query(record, query))
        .cloned()
        .collect()
}

fn matches_query(record: &AuditRecord, query: &AuditQuery) -> bool {
    if query
        .plugin_id
        .as_ref()
        .is_some_and(|plugin_id| &record.plugin_id != plugin_id)
    {
        return false;
    }
    if query
        .event
        .as_ref()
        .is_some_and(|event| &record.event != event)
    {
        return false;
    }
    if query
        .stage
        .as_ref()
        .is_some_and(|stage| &record.stage != stage)
    {
        return false;
    }
    if query
        .status
        .as_ref()
        .is_some_and(|status| &record.status != status)
    {
        return false;
    }
    if let Some(correlation_id) = &query.correlation_id {
        let matches = record
            .correlation_id
            .as_ref()
            .is_some_and(|id| id.as_str() == correlation_id);
        if !matches {
            return false;
        }
    }
    if let Some(generation_id) = &query.generation_id {
        let matches = record
            .generation_id
            .as_ref()
            .is_some_and(|id| id == generation_id);
        if !matches {
            return false;
        }
    }
    if query
        .min_duration_ms
        .is_some_and(|min| record.duration_ms < min)
    {
        return false;
    }
    if query
        .max_duration_ms
        .is_some_and(|max| record.duration_ms > max)
    {
        return false;
    }
    if query
        .since_ms
        .is_some_and(|since| record.recorded_at_ms < since)
    {
        return false;
    }
    if query
        .until_ms
        .is_some_and(|until| record.recorded_at_ms > until)
    {
        return false;
    }
    true
}

/// Deterministic ordering + bounded pagination.
pub fn paginate_audit_records(records: &[AuditRecord], page: &AuditPage) -> Vec<AuditRecord> {
    let mut sorted = records.to_vec();
    sorted.sort_by(|a, b| {
        let ord = match page.order_by {
            AuditOrderBy::RecordedAt => a.recorded_at_ms.cmp(&b.recorded_at_ms),
            AuditOrderBy::DurationMs => a.duration_ms.cmp(&b.duration_ms),
            AuditOrderBy::PluginId => a.plugin_id.cmp(&b.plugin_id),
            AuditOrderBy::Event => a.event.cmp(&b.event),
            AuditOrderBy::Status => a.status.cmp(&b.status),
        };
        if page.descending { ord.reverse() } else { ord }
    });
    sorted
        .into_iter()
        .skip(page.offset)
        .take(page.limit)
        .collect()
}

/// Keep the most recent `limit` records (by recorded_at_ms), preserving
/// insertion order among the retained set.
pub fn retain_audit_records(records: &[AuditRecord], limit: usize) -> Vec<AuditRecord> {
    if records.len() <= limit {
        return records.to_vec();
    }
    let mut indexed: Vec<(usize, &AuditRecord)> = records.iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        b.1.recorded_at_ms
            .cmp(&a.1.recorded_at_ms)
            .then_with(|| b.0.cmp(&a.0))
    });
    let keep: std::collections::HashSet<usize> = indexed
        .into_iter()
        .take(limit)
        .map(|(idx, _)| idx)
        .collect();
    records
        .iter()
        .enumerate()
        .filter(|(idx, _)| keep.contains(idx))
        .map(|(_, record)| record.clone())
        .collect()
}

/// Runtime permission re-check helper. Returns `Ok(())` when the live grant
/// still includes the required permission; `Err` when revoked/disabled.
pub fn ensure_live_permission(
    declared: &[Permission],
    live: &[Permission],
    required: &Permission,
) -> Result<(), String> {
    if !declared.contains(required) {
        return Err(format!("permission not declared: {required:?}"));
    }
    if !live.contains(required) {
        return Err(format!("permission revoked: {required:?}"));
    }
    Ok(())
}

/// Next correlation id from a monotonic counter map keyed by plugin id.
pub fn next_correlation_id(counters: &mut HashMap<String, u64>, plugin_id: &str) -> CorrelationId {
    let entry = counters.entry(plugin_id.to_string()).or_insert(0);
    *entry = entry.saturating_add(1);
    CorrelationId::new(format!("{plugin_id}:{entry}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(plugin_id: &str, recorded_at_ms: u64, status: &str) -> AuditRecord {
        AuditRecord::redacted(AuditRecordInput {
            plugin_id: plugin_id.into(),
            event: "CHAT_COMPLETION_PROMPT_READY".into(),
            stage: "frontend_intent".into(),
            status: status.into(),
            duration_ms: 10,
            correlation_id: Some(CorrelationId::new("corr-1")),
            generation_id: Some("gen-1".into()),
            changed_keys: vec!["prompt".into()],
            recorded_at_ms,
        })
    }

    #[test]
    fn redacted_constructor_strips_secret_like_labels() {
        let record = AuditRecord::redacted(AuditRecordInput {
            plugin_id: "token=plugin-secret".into(),
            event: "cookie=session-secret".into(),
            stage: "prompt=private-prompt-body".into(),
            status: "ok".into(),
            duration_ms: 1,
            correlation_id: Some(CorrelationId::new("corr with SF_SECRET_x")),
            generation_id: Some("secret=must-not-serialize".into()),
            changed_keys: vec!["Authorization".into(), "cookie=do-not-leak".into()],
            recorded_at_ms: 1,
        });
        let json = serde_json::to_string(&record).unwrap();
        assert!(!json.contains("SF_SECRET_"));
        assert!(!json.contains("plugin-secret"));
        assert!(!json.contains("session-secret"));
        assert!(!json.contains("private-prompt-body"));
        assert!(!json.contains("must-not-serialize"));
        assert!(!json.contains("do-not-leak"));
        assert!(!json.contains("Authorization"));
        assert!(
            record
                .changed_keys
                .iter()
                .all(|key| !key.contains("Authorization"))
        );
    }

    #[test]
    fn chain_is_deterministic_and_detects_tampering() {
        let records = vec![
            sample("p1", 100, "ok"),
            sample("p2", 200, "ok"),
            sample("p3", 300, "ok"),
        ];
        let chained = chain_audit_records(&records);
        let re_chained = chain_audit_records(&records);
        assert_eq!(
            chained
                .iter()
                .map(|r| r.record_hash.clone())
                .collect::<Vec<_>>(),
            re_chained
                .iter()
                .map(|r| r.record_hash.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(chained[0].prev_hash, None);
        assert_eq!(chained[1].prev_hash, chained[0].record_hash);
        assert_eq!(chained[2].prev_hash, chained[1].record_hash);

        let mut tampered = records.clone();
        tampered[1].status = "error".into();
        let tampered_chain = chain_audit_records(&tampered);
        assert_ne!(chained[1].record_hash, tampered_chain[1].record_hash);
        assert_ne!(chained[2].record_hash, tampered_chain[2].record_hash);
    }

    #[test]
    fn verifier_rejects_a_mutated_stored_chain_without_rechaining_it() {
        let records = chain_audit_records(&[sample("p1", 100, "ok"), sample("p2", 200, "ok")]);
        let mut tampered = records.clone();
        tampered[0].status = "error".into();

        let verification = verify_audit_record_chain(&tampered);
        assert!(!verification.valid);
        assert_eq!(verification.invalid_index, Some(0));
    }

    #[test]
    fn query_and_pagination_are_deterministic() {
        let records = vec![
            sample("zeta", 300, "ok"),
            sample("alpha", 100, "error"),
            sample("mid", 200, "timeout"),
        ];
        let filtered = query_audit_records(
            &records,
            &AuditQuery {
                status: Some("ok".into()),
                ..Default::default()
            },
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].plugin_id, "zeta");

        let page = paginate_audit_records(
            &records,
            &AuditPage {
                limit: 2,
                offset: 0,
                order_by: AuditOrderBy::RecordedAt,
                descending: false,
            },
        );
        assert_eq!(
            page.iter()
                .map(|r| r.plugin_id.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "mid"]
        );
    }

    #[test]
    fn retention_keeps_most_recent_in_insertion_order() {
        let records: Vec<_> = (0..10)
            .map(|i| sample(&format!("p{i}"), 100 + i, "ok"))
            .collect();
        let retained = retain_audit_records(&records, 3);
        assert_eq!(
            retained
                .iter()
                .map(|r| r.plugin_id.as_str())
                .collect::<Vec<_>>(),
            vec!["p7", "p8", "p9"]
        );
    }

    #[test]
    fn permission_recheck_detects_revocation() {
        let declared = vec![Permission::ModifyPrompt, Permission::ReadMemory];
        let live = vec![Permission::ReadMemory];
        assert!(ensure_live_permission(&declared, &declared, &Permission::ModifyPrompt).is_ok());
        assert!(ensure_live_permission(&declared, &live, &Permission::ModifyPrompt).is_err());
    }

    #[test]
    fn correlation_ids_are_monotonic_per_plugin() {
        let mut counters = HashMap::new();
        let a1 = next_correlation_id(&mut counters, "plugin-a");
        let a2 = next_correlation_id(&mut counters, "plugin-a");
        let b1 = next_correlation_id(&mut counters, "plugin-b");
        assert_eq!(a1.as_str(), "plugin-a:1");
        assert_eq!(a2.as_str(), "plugin-a:2");
        assert_eq!(b1.as_str(), "plugin-b:1");
    }

    #[test]
    fn audit_records_never_hold_secret_fields() {
        let record = sample("p1", 1, "ok");
        let json = serde_json::to_string(&record).unwrap();
        for banned in [
            "api_key",
            "Authorization",
            "SF_SECRET_",
            "private prompt",
            "stack",
        ] {
            assert!(!json.contains(banned), "audit record leaked {banned}");
        }
    }
}
