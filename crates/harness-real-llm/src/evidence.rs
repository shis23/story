//! 脱敏评估证据（JSONL）。
//!
//! 写入字段只含角色、streaming、segment hash、usage、耗时与断言结果。
//! **禁止**写入 API key、完整 prompt、完整 secret 或供应商原始响应正文。

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 脱敏的工具调用/结果步骤（无正文、无参数原文）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EvidenceToolStep {
    /// `offered` | `call` | `result`
    pub kind: String,
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id16: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_hash16: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_len: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_hash16: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_len: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// 安全计数/枚举细节（如 summaries_count=3），禁止正文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// 单次 LLM 调用的脱敏证据行。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceCallRecord {
    pub schema_version: String,
    pub run_id: String,
    pub suite: String,
    pub turn_index: u32,
    pub role: String,
    pub tag: String,
    pub streaming: bool,
    pub request_fp16: String,
    pub system_hash16: String,
    pub history_hash16: String,
    pub tail_hash16: String,
    pub history_len: usize,
    pub tail_parts: usize,
    pub msg_count: usize,
    pub prompt_tokens: u32,
    pub cached_tokens: u32,
    pub cache_creation_tokens: u32,
    pub completion_tokens: u32,
    pub elapsed_ms: u128,
    /// `ok` / `client_error` / `timeout`；禁止落供应商原始错误正文。
    #[serde(default = "default_call_outcome")]
    pub outcome: String,
    /// 请求中声明的工具名列表（导演/子代理工具环）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_offered: Vec<String>,
    /// 本轮响应发出的 tool_calls + 请求 history 中已执行的 tool results。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_steps: Vec<EvidenceToolStep>,
    pub assertion_results: Vec<AssertionResult>,
    pub model_label: String,
    pub recorded_at_unix_ms: u128,
}

/// 单 turn 聚合的工具调用全过程（多 LLM 往返拼成一条时间线）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceToolTraceRecord {
    pub schema_version: String,
    pub run_id: String,
    pub suite: String,
    pub turn_index: u32,
    pub role: String,
    pub tag: String,
    /// 是否出现远记忆相关工具（get_recent_summary / search_chronicle / search_vectors）。
    pub remote_memory_tool_used: bool,
    pub tools_offered: Vec<String>,
    pub steps: Vec<EvidenceToolStep>,
    pub call_count: usize,
    pub result_count: usize,
    pub assertion_results: Vec<AssertionResult>,
    pub model_label: String,
    pub recorded_at_unix_ms: u128,
}

/// 断言结果（不落原文）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssertionResult {
    pub name: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// 一轮 Accept 闭环的脱敏摘要。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceTurnRecord {
    pub schema_version: String,
    pub run_id: String,
    pub suite: String,
    pub turn_index: u32,
    pub kind: String,
    /// 写作来源、Chronicle 来源与 Accept 路径必须拆开，防止 synthetic fixture 被误宣称。
    #[serde(default)]
    pub write_path: String,
    #[serde(default)]
    pub chronicle_path: String,
    #[serde(default)]
    pub accept_path: String,
    #[serde(default)]
    pub production_postprocess_complete: bool,
    pub draft_accepted: bool,
    pub force_accept: bool,
    pub quality_error_count: usize,
    pub quality_warning_count: usize,
    pub autofix_attempts: u32,
    pub campaign_revision_before: u64,
    pub campaign_revision_after: u64,
    pub chronicle_revision_before: u64,
    pub chronicle_revision_after: u64,
    pub summary_code: Option<String>,
    pub attempt_status: String,
    pub turn_status: String,
    pub draft_hash16: String,
    pub text_len: usize,
    pub text_sha16: String,
    pub early_fact_reachable: Option<bool>,
    /// ContextEpoch 仅落短指纹与计数，不落完整成员或正文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_epoch_id16: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_epoch_source_hash16: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_epoch_anchor_count: Option<usize>,
    pub assertion_results: Vec<AssertionResult>,
    pub elapsed_ms: u128,
    pub recorded_at_unix_ms: u128,
}

/// Phase B A/B 对照矩阵一行。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceAbRow {
    pub schema_version: String,
    pub run_id: String,
    pub suite: String,
    pub fixture_id: String,
    pub arm: String,
    pub seed: u64,
    pub model_label: String,
    pub leak_detected: bool,
    pub quality_error_count: usize,
    pub quality_warning_count: usize,
    pub autofix_count: u32,
    pub latency_ms: u128,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
    /// Continuity/agency marker: whether the draft maintained scene continuity
    /// (deterministic = true; real-model path overwrites).
    #[serde(default)]
    pub continuity_ok: bool,
    /// Output emptiness/truncation/failure indicator.
    /// `ok` / `empty` / `truncated` / `failed`
    #[serde(default = "default_ab_outcome")]
    pub output_status: String,
    pub assertion_results: Vec<AssertionResult>,
    pub recorded_at_unix_ms: u128,
}

pub const EVIDENCE_SCHEMA_VERSION: &str = "eval-m5-phaseb-v1";

fn default_call_outcome() -> String {
    "ok".into()
}

fn default_ab_outcome() -> String {
    "ok".into()
}

/// JSONL 证据写入器（自动创建父目录）。
pub struct EvidenceWriter {
    path: PathBuf,
    run_id: String,
}

impl EvidenceWriter {
    pub fn create(path: impl Into<PathBuf>, run_id: impl Into<String>) -> std::io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // truncate on create so each run is self-contained
        File::create(&path)?;
        Ok(Self {
            path,
            run_id: run_id.into(),
        })
    }

    pub fn open_append(
        path: impl Into<PathBuf>,
        run_id: impl Into<String>,
    ) -> std::io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            run_id: run_id.into(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn write_json_line<T: Serialize>(&self, value: &T) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = serde_json::to_string(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // refuse to write if raw secrets leak into serialized form
        if contains_forbidden_evidence_payload(&line) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "refusing to write evidence line that appears to contain secrets or full prompts",
            ));
        }
        writeln!(file, "{line}")?;
        Ok(())
    }

    pub fn write_call(&self, mut rec: EvidenceCallRecord) -> std::io::Result<()> {
        if rec.run_id.is_empty() {
            rec.run_id = self.run_id.clone();
        }
        if rec.schema_version.is_empty() {
            rec.schema_version = EVIDENCE_SCHEMA_VERSION.into();
        }
        if rec.recorded_at_unix_ms == 0 {
            rec.recorded_at_unix_ms = now_unix_ms();
        }
        sanitize_call_record(&mut rec);
        self.write_json_line(&rec)
    }

    pub fn write_tool_trace(&self, mut rec: EvidenceToolTraceRecord) -> std::io::Result<()> {
        if rec.run_id.is_empty() {
            rec.run_id = self.run_id.clone();
        }
        if rec.schema_version.is_empty() {
            rec.schema_version = EVIDENCE_SCHEMA_VERSION.into();
        }
        if rec.recorded_at_unix_ms == 0 {
            rec.recorded_at_unix_ms = now_unix_ms();
        }
        sanitize_tool_trace_record(&mut rec);
        self.write_json_line(&rec)
    }

    pub fn write_turn(&self, mut rec: EvidenceTurnRecord) -> std::io::Result<()> {
        if rec.run_id.is_empty() {
            rec.run_id = self.run_id.clone();
        }
        if rec.schema_version.is_empty() {
            rec.schema_version = EVIDENCE_SCHEMA_VERSION.into();
        }
        if rec.recorded_at_unix_ms == 0 {
            rec.recorded_at_unix_ms = now_unix_ms();
        }
        sanitize_turn_record(&mut rec);
        self.write_json_line(&rec)
    }

    pub fn write_ab_row(&self, mut rec: EvidenceAbRow) -> std::io::Result<()> {
        if rec.run_id.is_empty() {
            rec.run_id = self.run_id.clone();
        }
        if rec.schema_version.is_empty() {
            rec.schema_version = EVIDENCE_SCHEMA_VERSION.into();
        }
        if rec.recorded_at_unix_ms == 0 {
            rec.recorded_at_unix_ms = now_unix_ms();
        }
        self.write_json_line(&rec)
    }
}

/// 读取 JSONL 证据行（测试/审计用）。
pub fn read_evidence_lines(path: &Path) -> std::io::Result<Vec<serde_json::Value>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(trimmed)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        out.push(v);
    }
    Ok(out)
}

pub fn short_hash16(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    hex.chars().take(16).collect()
}

pub fn draft_hash_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 自动脱敏：截断过长 detail，去掉疑似 key/secret 字段。
pub fn sanitize_call_record(rec: &mut EvidenceCallRecord) {
    for a in &mut rec.assertion_results {
        if let Some(detail) = a.detail.as_mut() {
            *detail = redact_detail(detail);
        }
    }
    rec.request_fp16 = truncate_hex16(&rec.request_fp16);
    rec.system_hash16 = truncate_hex16(&rec.system_hash16);
    rec.history_hash16 = truncate_hex16(&rec.history_hash16);
    rec.tail_hash16 = truncate_hex16(&rec.tail_hash16);
    rec.outcome = match rec.outcome.as_str() {
        "ok" | "client_error" | "timeout" => rec.outcome.clone(),
        _ => "client_error".into(),
    };
    if rec.model_label.len() > 64 {
        rec.model_label = rec.model_label.chars().take(64).collect();
    }
    sanitize_tool_steps(&mut rec.tool_steps);
    for name in &mut rec.tools_offered {
        if name.len() > 64 {
            *name = name.chars().take(64).collect();
        }
    }
}

fn sanitize_tool_steps(steps: &mut [EvidenceToolStep]) {
    for step in steps {
        if step.tool_name.len() > 64 {
            step.tool_name = step.tool_name.chars().take(64).collect();
        }
        if let Some(id) = step.call_id16.as_mut() {
            *id = truncate_hex16(id);
        }
        if let Some(h) = step.args_hash16.as_mut() {
            *h = truncate_hex16(h);
        }
        if let Some(h) = step.result_hash16.as_mut() {
            *h = truncate_hex16(h);
        }
        if let Some(detail) = step.detail.as_mut() {
            *detail = redact_detail(detail);
            if detail.len() > 120 {
                *detail = detail.chars().take(120).collect();
            }
        }
        match step.kind.as_str() {
            "offered" | "call" | "result" => {}
            _ => step.kind = "call".into(),
        }
    }
}

pub fn sanitize_tool_trace_record(rec: &mut EvidenceToolTraceRecord) {
    for a in &mut rec.assertion_results {
        if let Some(detail) = a.detail.as_mut() {
            *detail = redact_detail(detail);
        }
    }
    sanitize_tool_steps(&mut rec.steps);
    for name in &mut rec.tools_offered {
        if name.len() > 64 {
            *name = name.chars().take(64).collect();
        }
    }
    if rec.model_label.len() > 64 {
        rec.model_label = rec.model_label.chars().take(64).collect();
    }
}

/// 从一组 call 样本聚合 turn 级工具时间线。
pub fn aggregate_tool_trace(
    run_id: impl Into<String>,
    suite: impl Into<String>,
    turn_index: u32,
    model_label: impl Into<String>,
    calls: &[EvidenceCallRecord],
) -> Option<EvidenceToolTraceRecord> {
    let mut tools_offered = Vec::new();
    let mut steps = Vec::new();
    let mut role = "pipeline".to_string();
    let mut tag = format!("sqlite-turn{turn_index}");
    for c in calls {
        if c.turn_index != turn_index {
            continue;
        }
        role = c.role.clone();
        tag = c.tag.clone();
        for name in &c.tools_offered {
            if !tools_offered.iter().any(|n| n == name) {
                tools_offered.push(name.clone());
            }
        }
        steps.extend(c.tool_steps.iter().cloned());
    }
    if tools_offered.is_empty() && steps.is_empty() {
        return None;
    }
    let call_count = steps.iter().filter(|s| s.kind == "call").count();
    let result_count = steps.iter().filter(|s| s.kind == "result").count();
    let remote_memory_tool_used = steps.iter().any(|s| {
        matches!(
            s.tool_name.as_str(),
            "get_recent_summary" | "search_chronicle" | "search_vectors" | "get_chronicle"
        )
    }) || tools_offered.iter().any(|n| {
        matches!(
            n.as_str(),
            "get_recent_summary" | "search_chronicle" | "search_vectors" | "get_chronicle"
        )
    });
    Some(EvidenceToolTraceRecord {
        schema_version: EVIDENCE_SCHEMA_VERSION.into(),
        run_id: run_id.into(),
        suite: suite.into(),
        turn_index,
        role,
        tag,
        remote_memory_tool_used,
        tools_offered,
        steps,
        call_count,
        result_count,
        assertion_results: vec![AssertionResult {
            name: "tool_trace_recorded".into(),
            passed: true,
            detail: Some(format!("calls={call_count};results={result_count}")),
        }],
        model_label: model_label.into(),
        recorded_at_unix_ms: 0,
    })
}

pub fn sanitize_turn_record(rec: &mut EvidenceTurnRecord) {
    for a in &mut rec.assertion_results {
        if let Some(detail) = a.detail.as_mut() {
            *detail = redact_detail(detail);
        }
    }
    rec.draft_hash16 = truncate_hex16(&rec.draft_hash16);
    rec.text_sha16 = truncate_hex16(&rec.text_sha16);
    rec.context_epoch_id16 = rec.context_epoch_id16.as_deref().map(truncate_hex16);
    rec.context_epoch_source_hash16 = rec
        .context_epoch_source_hash16
        .as_deref()
        .map(truncate_hex16);
}

fn truncate_hex16(s: &str) -> String {
    s.chars().take(16).collect()
}

fn redact_detail(s: &str) -> String {
    let mut out = s.to_string();
    // strip common secret-like tokens
    for needle in [
        "sk-",
        "api_key",
        "API_KEY",
        "Bearer ",
        "SF_SECRET_",
        "-----BEGIN",
    ] {
        if out.contains(needle) {
            out = format!("<redacted detail containing {needle}>");
            break;
        }
    }
    if out.chars().count() > 240 {
        out = out.chars().take(240).collect::<String>() + "…";
    }
    out
}

/// 证据序列化守卫：命中则拒绝写盘。
pub fn contains_forbidden_evidence_payload(serialized: &str) -> bool {
    let lower = serialized.to_ascii_lowercase();
    if lower.contains("\"api_key\"")
        || lower.contains("\"apikey\"")
        || contains_secret_key_token(&lower)
        || lower.contains("bearer ")
        || lower.contains("-----begin")
    {
        return true;
    }
    // full prompt bodies should never appear as long free text fields
    if lower.contains("\"prompt\":") || lower.contains("\"messages\":") {
        return true;
    }
    // production private probe tokens must not be stored raw
    if serialized.contains("SF_SECRET_") {
        return true;
    }
    false
}

fn contains_secret_key_token(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices("sk-").any(|(index, _)| {
        let has_token_prefix =
            index > 0 && matches!(bytes[index - 1], b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-');
        if has_token_prefix {
            return false;
        }
        text[index + 3..]
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            .count()
            >= 1
    })
}

/// 真实模型运行开关（默认关闭；无凭证时测试应 ignored，而不是假通过）。
#[derive(Debug, Clone)]
pub struct RealLlmRunBudget {
    pub enabled: bool,
    pub max_calls: u32,
    pub max_turns: u32,
    pub timeout_secs: u64,
    /// Optional request-level output ceiling. `None` delegates to the provider/model default.
    pub max_tokens: Option<u32>,
}

impl Default for RealLlmRunBudget {
    fn default() -> Self {
        Self {
            enabled: false,
            max_calls: 0,
            max_turns: 0,
            timeout_secs: 120,
            max_tokens: None,
        }
    }
}

impl RealLlmRunBudget {
    /// 从环境变量解析。
    ///
    /// - `STORYFORGE_EVAL_REAL_LLM=1|true|yes` 才允许真实调用
    /// - `STORYFORGE_EVAL_MAX_CALLS`（默认 40）
    /// - `STORYFORGE_EVAL_MAX_TURNS`（默认 24）
    /// - `STORYFORGE_EVAL_TIMEOUT_SECS`（默认 180）
    /// - `STORYFORGE_EVAL_MAX_TOKENS`（默认不发送；正整数时覆盖每次评估请求）
    pub fn from_env() -> Self {
        let enabled = std::env::var("STORYFORGE_EVAL_REAL_LLM")
            .ok()
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "1" | "true" | "yes" | "on")
            })
            .unwrap_or(false);
        let max_calls = parse_u32_env("STORYFORGE_EVAL_MAX_CALLS", 40);
        let max_turns = parse_u32_env("STORYFORGE_EVAL_MAX_TURNS", 24);
        let timeout_secs = parse_u64_env("STORYFORGE_EVAL_TIMEOUT_SECS", 180);
        let max_tokens = std::env::var("STORYFORGE_EVAL_MAX_TOKENS")
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .filter(|value| *value > 0);
        Self {
            enabled,
            max_calls,
            max_turns,
            timeout_secs,
            max_tokens,
        }
    }

    pub fn assert_enabled_or_skip(reason: &str) {
        let budget = Self::from_env();
        if !budget.enabled {
            eprintln!(
                "skip real LLM eval: STORYFORGE_EVAL_REAL_LLM not enabled ({reason}); \
                 set STORYFORGE_EVAL_REAL_LLM=1 plus LLM_BASE_URL/API_KEY/MODEL to run"
            );
            // panic with ignore-friendly message when used inside #[ignore] tests via require
            panic!("STORYFORGE_EVAL_REAL_LLM disabled: {reason}");
        }
    }
}

fn parse_u32_env(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

fn parse_u64_env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_writer_roundtrips_call_record_without_secrets() {
        let dir = std::env::temp_dir().join(format!("sf_eval_ev_{}", uuid::Uuid::new_v4()));
        let path = dir.join("calls.jsonl");
        let writer = EvidenceWriter::create(&path, "run-1").unwrap();
        let rec = EvidenceCallRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: "run-1".into(),
            suite: "unit".into(),
            turn_index: 1,
            role: "editor".into(),
            tag: "turn1".into(),
            streaming: true,
            request_fp16: "abcdef0123456789".into(),
            system_hash16: "1111111111111111".into(),
            history_hash16: "2222222222222222".into(),
            tail_hash16: "3333333333333333".into(),
            history_len: 4,
            tail_parts: 1,
            msg_count: 6,
            prompt_tokens: 100,
            cached_tokens: 20,
            cache_creation_tokens: 0,
            completion_tokens: 30,
            elapsed_ms: 12,
            outcome: "ok".into(),
            tools_offered: vec!["get_recent_summary".into()],
            tool_steps: vec![EvidenceToolStep {
                kind: "call".into(),
                tool_name: "get_recent_summary".into(),
                call_id16: Some("aaaaaaaaaaaaaaaa".into()),
                args_hash16: Some("bbbbbbbbbbbbbbbb".into()),
                args_len: Some(12),
                result_hash16: None,
                result_len: None,
                ok: None,
                detail: Some("limit=3".into()),
            }],
            assertion_results: vec![AssertionResult {
                name: "non_empty".into(),
                passed: true,
                detail: None,
            }],
            model_label: "mock".into(),
            recorded_at_unix_ms: 1,
        };
        writer.write_call(rec.clone()).unwrap();
        let lines = read_evidence_lines(&path).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["tag"], "turn1");
        assert_eq!(lines[0]["cached_tokens"], 20);
        assert!(lines[0].get("api_key").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn forbids_secret_payload_in_serialized_line() {
        assert!(contains_forbidden_evidence_payload(
            r#"{"api_key":"sk-abc"}"#
        ));
        assert!(contains_forbidden_evidence_payload(
            r#"{"text":"SF_SECRET_CHEN_BADGE_X91"}"#
        ));
        assert!(!contains_forbidden_evidence_payload(
            r#"{"request_fp16":"abc","role":"director"}"#
        ));
        assert!(contains_forbidden_evidence_payload(&format!(
            r#"{{"token":"{}{}"}}"#,
            "s", "k-1234567890abcdef"
        )));
        assert!(!contains_forbidden_evidence_payload(
            r#"{"task_id":"task-private-probe"}"#
        ));
    }

    #[test]
    fn redact_detail_strips_secret_markers() {
        let s = redact_detail("leak SF_SECRET_FOO in body");
        assert!(s.contains("redacted"));
        assert!(!s.contains("SF_SECRET_FOO"));
    }

    #[test]
    fn real_llm_budget_defaults_disabled() {
        // cannot safely mutate process env in parallel tests; just check Default
        let b = RealLlmRunBudget::default();
        assert!(!b.enabled);
        assert_eq!(b.max_calls, 0);
    }

    #[test]
    fn aggregate_tool_trace_marks_remote_memory_without_payloads() {
        let call = EvidenceCallRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: "run-t".into(),
            suite: "endurance_sqlite".into(),
            turn_index: 7,
            role: "pipeline".into(),
            tag: "sqlite-turn7".into(),
            streaming: true,
            request_fp16: "aaaaaaaaaaaaaaaa".into(),
            system_hash16: "bbbbbbbbbbbbbbbb".into(),
            history_hash16: "cccccccccccccccc".into(),
            tail_hash16: "dddddddddddddddd".into(),
            history_len: 3,
            tail_parts: 1,
            msg_count: 5,
            prompt_tokens: 10,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            completion_tokens: 2,
            elapsed_ms: 3,
            outcome: "ok".into(),
            tools_offered: vec!["get_recent_summary".into(), "search_chronicle".into()],
            tool_steps: vec![
                EvidenceToolStep {
                    kind: "call".into(),
                    tool_name: "get_recent_summary".into(),
                    call_id16: Some("1111111111111111".into()),
                    args_hash16: Some("2222222222222222".into()),
                    args_len: Some(11),
                    result_hash16: None,
                    result_len: None,
                    ok: None,
                    detail: Some("limit=3".into()),
                },
                EvidenceToolStep {
                    kind: "result".into(),
                    tool_name: "get_recent_summary".into(),
                    call_id16: Some("1111111111111111".into()),
                    args_hash16: None,
                    args_len: None,
                    result_hash16: Some("3333333333333333".into()),
                    result_len: Some(40),
                    ok: Some(true),
                    detail: Some("summaries_count=2".into()),
                },
            ],
            assertion_results: vec![],
            model_label: "mock".into(),
            recorded_at_unix_ms: 1,
        };
        let trace = aggregate_tool_trace("run-t", "endurance_sqlite", 7, "mock", &[call])
            .expect("trace present");
        assert!(trace.remote_memory_tool_used);
        assert_eq!(trace.call_count, 1);
        assert_eq!(trace.result_count, 1);
        let line = serde_json::to_string(&trace).unwrap();
        assert!(!line.contains("sk-"));
        assert!(!contains_forbidden_evidence_payload(&line));
        assert!(line.contains("get_recent_summary"));
        assert!(line.contains("summaries_count=2"));
    }
}
