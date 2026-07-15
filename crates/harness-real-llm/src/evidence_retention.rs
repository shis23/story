//! Durable, redacted M5 evidence retention.
//!
//! Provides a fail-closed evidence root policy, unique run IDs, atomic run
//! manifests with content hashes, offline verify, archive/restore, and bounded
//! retention cleanup. This module never calls models and never reads credentials.
//!
//! Default root resolution is **conservative**: an explicit env/CLI root is
//! required unless a test policy opts into ephemeral roots.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::endurance::{EnduranceEvidencePaths, read_latest_checkpoint};
use crate::evidence::{
    EVIDENCE_SCHEMA_VERSION, contains_forbidden_evidence_payload, now_unix_ms, read_evidence_lines,
};

/// Schema id for the atomic per-run retention manifest.
pub const RETENTION_SCHEMA_VERSION: &str = "m5-evidence-retention-v1";

/// File name of the atomic run manifest written by [`seal_run`].
pub const RUN_MANIFEST_FILE: &str = "run_manifest.json";

/// Controlled run directory prefix (namespace).
pub const RUN_DIR_PREFIX: &str = "run-";

/// Environment variable for the durable evidence root.
pub const EVIDENCE_ROOT_ENV: &str = "STORYFORGE_EVAL_EVIDENCE_ROOT";

/// Legacy evidence directory env (single-run path). Still accepted when it
/// resolves to a validated root or a child under a validated root.
pub const EVIDENCE_DIR_ENV: &str = "STORYFORGE_EVAL_EVIDENCE_DIR";

/// Required evidence file names sealed for endurance runs.
const REQUIRED_EVIDENCE_FILES: &[&str] = &[
    "endurance_calls.jsonl",
    "endurance_turns.jsonl",
    "endurance_checkpoint.jsonl",
];

/// Optional evidence files hashed when present.
const OPTIONAL_EVIDENCE_FILES: &[&str] = &["endurance_manifest.jsonl", "endurance_phase_b.jsonl"];

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum EvidenceRetentionError {
    MissingEvidenceRoot,
    IllegalEvidenceRoot { reason: String },
    PathTraversal { detail: String },
    PathOutsideRoot { detail: String },
    UncontrolledPath { detail: String },
    DuplicateRunId { run_id: String },
    MixedRunId { detail: String },
    RunIdMismatch { detail: String },
    MissingRequiredFile { relative_path: String },
    HashMismatch { relative_path: String },
    FileSetMismatch { detail: String },
    SchemaMismatch { detail: String },
    ForbiddenPayload { relative_path: String },
    InvalidRunId { detail: String },
    ResumeUnavailable { detail: String },
    ReparsePoint { detail: String },
    Io { detail: String },
}

impl std::fmt::Display for EvidenceRetentionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEvidenceRoot => write!(
                f,
                "missing evidence root: set {EVIDENCE_ROOT_ENV} (or {EVIDENCE_DIR_ENV}) to an explicit durable path outside the repo"
            ),
            Self::IllegalEvidenceRoot { reason } => {
                write!(f, "illegal evidence root: {reason}")
            }
            Self::PathTraversal { detail } => write!(f, "path traversal rejected: {detail}"),
            Self::PathOutsideRoot { detail } => write!(f, "path outside evidence root: {detail}"),
            Self::UncontrolledPath { detail } => {
                write!(f, "path is outside the controlled run namespace: {detail}")
            }
            Self::DuplicateRunId { run_id } => write!(f, "duplicate run id refused: {run_id}"),
            Self::MixedRunId { detail } => write!(f, "mixed run ids in evidence: {detail}"),
            Self::RunIdMismatch { detail } => write!(f, "run id mismatch: {detail}"),
            Self::MissingRequiredFile { relative_path } => {
                write!(f, "missing required evidence file: {relative_path}")
            }
            Self::HashMismatch { relative_path } => {
                write!(f, "content hash mismatch: {relative_path}")
            }
            Self::FileSetMismatch { detail } => {
                write!(f, "evidence file set mismatch: {detail}")
            }
            Self::SchemaMismatch { detail } => write!(f, "schema mismatch: {detail}"),
            Self::ForbiddenPayload { relative_path } => {
                write!(
                    f,
                    "forbidden (secret/prompt) payload detected under {relative_path}"
                )
            }
            Self::InvalidRunId { detail } => write!(f, "invalid run id: {detail}"),
            Self::ResumeUnavailable { detail } => write!(f, "resume unavailable: {detail}"),
            Self::ReparsePoint { detail } => {
                write!(f, "symlink/junction/reparse point rejected: {detail}")
            }
            Self::Io { detail } => write!(f, "evidence retention I/O error: {detail}"),
        }
    }
}

impl std::error::Error for EvidenceRetentionError {}

impl From<io::Error> for EvidenceRetentionError {
    fn from(value: io::Error) -> Self {
        Self::Io {
            detail: value.to_string(),
        }
    }
}

impl From<serde_json::Error> for EvidenceRetentionError {
    fn from(value: serde_json::Error) -> Self {
        Self::Io {
            detail: format!("json: {value}"),
        }
    }
}

// ── Root policy ─────────────────────────────────────────────────────────────

/// Policy for validating and resolving evidence roots.
#[derive(Debug, Clone)]
pub struct EvidenceRootPolicy {
    pub repo_root: PathBuf,
    /// When false, refuse temp/ephemeral roots (default production posture).
    pub allow_ephemeral: bool,
    /// When true, require an explicit env/CLI path (default production posture).
    pub require_explicit: bool,
}

impl EvidenceRootPolicy {
    /// Production-safe defaults: explicit durable root only.
    pub fn production(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            allow_ephemeral: false,
            require_explicit: true,
        }
    }

    /// Deterministic-test policy: allow unique temp roots under the OS temp dir.
    pub fn for_tests(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            allow_ephemeral: true,
            require_explicit: false,
        }
    }
}

/// Reject raw path strings that contain traversal markers before join/canonicalize.
pub fn reject_path_traversal(raw: &str) -> Result<(), EvidenceRetentionError> {
    let normalized = raw.replace('\\', "/");
    if normalized.split('/').any(|seg| seg == "..") {
        return Err(EvidenceRetentionError::PathTraversal {
            detail: "component '..' is not allowed".into(),
        });
    }
    if raw.contains('\0') {
        return Err(EvidenceRetentionError::PathTraversal {
            detail: "NUL byte in path".into(),
        });
    }
    Ok(())
}

fn path_contains_component(path: &Path, name: &str) -> bool {
    path.components().any(|c| match c {
        Component::Normal(os) => os == name,
        _ => false,
    })
}

fn is_under(parent: &Path, child: &Path) -> bool {
    let Ok(parent) = fs::canonicalize(parent).or_else(|_| Ok::<_, io::Error>(parent.to_path_buf()))
    else {
        return false;
    };
    let Ok(child) = fs::canonicalize(child).or_else(|_| Ok::<_, io::Error>(child.to_path_buf()))
    else {
        return false;
    };
    child == parent || child.starts_with(&parent)
}

fn is_temp_like(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    if is_under(&temp, path) {
        return true;
    }
    let s = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    s.contains("/tmp/")
        || s.contains("/temp/")
        || s.contains("/var/folders/")
        || s.starts_with("/tmp")
        || s.contains(":\\tmp\\")
        || s.contains("\\tmp\\")
        || s.contains(":\\temp\\")
        || s.contains("\\temp\\")
        || s.contains("/appdata/local/temp")
}

fn looks_like_live_campaign_data(path: &Path, repo_root: &Path) -> bool {
    if path_contains_component(path, "campaign_data") {
        return true;
    }
    // Repo `data/` is the live app data home — never an evidence root.
    let live = repo_root.join("data");
    is_under(&live, path) || path == live
}

/// Reject symlink / junction / reparse points (Windows reparse or Unix symlink).
pub fn assert_not_reparse_path(path: &Path) -> Result<(), EvidenceRetentionError> {
    if !path.exists() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(EvidenceRetentionError::ReparsePoint {
            detail: "symlink rejected".into(),
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(EvidenceRetentionError::ReparsePoint {
                detail: "reparse/junction rejected".into(),
            });
        }
    }
    Ok(())
}

fn canonicalize_existing(path: &Path) -> Result<PathBuf, EvidenceRetentionError> {
    assert_not_reparse_path(path)?;
    fs::canonicalize(path).map_err(|e| EvidenceRetentionError::Io {
        detail: format!("canonicalize failed: {e}"),
    })
}

/// Ensure `path` (existing or not) stays inside `root` after canonical resolution.
fn ensure_canonical_within(root: &Path, path: &Path) -> Result<PathBuf, EvidenceRetentionError> {
    let root_c = canonicalize_existing(root)?;
    if path.exists() {
        let path_c = canonicalize_existing(path)?;
        if path_c == root_c || path_c.starts_with(&root_c) {
            return Ok(path_c);
        }
        return Err(EvidenceRetentionError::PathOutsideRoot {
            detail: "resolved path escapes evidence root".into(),
        });
    }
    // Non-existent: canonicalize parent and re-join final component.
    let parent = path
        .parent()
        .ok_or_else(|| EvidenceRetentionError::PathOutsideRoot {
            detail: "path has no parent".into(),
        })?;
    let parent_c = if parent.exists() {
        canonicalize_existing(parent)?
    } else {
        // Walk up until an existing ancestor is found.
        let mut cur = parent.to_path_buf();
        while !cur.exists() {
            cur = cur
                .parent()
                .ok_or_else(|| EvidenceRetentionError::PathOutsideRoot {
                    detail: "no existing ancestor".into(),
                })?
                .to_path_buf();
        }
        let cur_c = canonicalize_existing(&cur)?;
        // Rebuild relative suffix from cur -> parent under cur_c.
        let suffix =
            parent
                .strip_prefix(&cur)
                .map_err(|_| EvidenceRetentionError::PathOutsideRoot {
                    detail: "cannot strip ancestor prefix".into(),
                })?;
        for c in suffix.components() {
            if matches!(c, Component::ParentDir | Component::CurDir) {
                return Err(EvidenceRetentionError::PathTraversal {
                    detail: "relative components in non-existent path".into(),
                });
            }
        }
        cur_c.join(suffix)
    };
    let joined = parent_c.join(path.file_name().unwrap_or_default());
    if joined == root_c || joined.starts_with(&root_c) {
        Ok(joined)
    } else {
        Err(EvidenceRetentionError::PathOutsideRoot {
            detail: "resolved path escapes evidence root".into(),
        })
    }
}

/// Validate that `candidate` may be used as an evidence root.
pub fn validate_evidence_root(
    candidate: &Path,
    policy: &EvidenceRootPolicy,
) -> Result<PathBuf, EvidenceRetentionError> {
    let raw = candidate.to_string_lossy();
    reject_path_traversal(raw.as_ref())?;

    if raw.trim().is_empty() {
        return Err(EvidenceRetentionError::IllegalEvidenceRoot {
            reason: "empty path".into(),
        });
    }

    // Create when missing so tests/runners can allocate a fresh durable root.
    if !candidate.exists() {
        fs::create_dir_all(candidate)?;
    }
    assert_not_reparse_path(candidate)?;
    if !candidate.is_dir() {
        return Err(EvidenceRetentionError::IllegalEvidenceRoot {
            reason: "path is not a directory".into(),
        });
    }

    let resolved = canonicalize_existing(candidate)?;

    let repo_c = fs::canonicalize(&policy.repo_root).unwrap_or_else(|_| policy.repo_root.clone());
    // Reject exact repo root and any descendant. A parent of the repo is allowed
    // (e.g. a sibling durable volume), which is intentionally not treated as "under".
    if resolved == repo_c || is_under(&repo_c, &resolved) {
        return Err(EvidenceRetentionError::IllegalEvidenceRoot {
            reason: "evidence root must not live inside the repository".into(),
        });
    }

    if looks_like_live_campaign_data(&resolved, &policy.repo_root) {
        return Err(EvidenceRetentionError::IllegalEvidenceRoot {
            reason: "live campaign/app data directories are not valid evidence roots".into(),
        });
    }

    if !policy.allow_ephemeral && is_temp_like(&resolved) {
        return Err(EvidenceRetentionError::IllegalEvidenceRoot {
            reason: "ephemeral/temp roots are rejected unless explicitly allowed for tests".into(),
        });
    }

    // Refuse roots that are clearly "tmp endurance cleanup" historical names when
    // production policy is on — they are disposable.
    let name = resolved
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !policy.allow_ephemeral
        && (name.starts_with("endurance-evidence") || name.starts_with("storyforge_endurance_"))
        && is_temp_like(&resolved)
    {
        return Err(EvidenceRetentionError::IllegalEvidenceRoot {
            reason: "historical disposable endurance-evidence temp directories are rejected".into(),
        });
    }

    Ok(resolved)
}

/// Validate a resume `STORYFORGE_EVAL_EVIDENCE_DIR` run path:
/// controlled dirname, parent durable-root policy, no reparse, under parent.
pub fn resolve_resume_run_dir(
    run_dir: &Path,
    policy: &EvidenceRootPolicy,
) -> Result<PathBuf, EvidenceRetentionError> {
    let raw = run_dir.to_string_lossy();
    reject_path_traversal(raw.as_ref())?;
    if !run_dir.exists() {
        return Err(EvidenceRetentionError::ResumeUnavailable {
            detail: "resume run directory does not exist".into(),
        });
    }
    assert_not_reparse_path(run_dir)?;
    let name = run_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !is_controlled_run_dirname(name) {
        return Err(EvidenceRetentionError::UncontrolledPath {
            detail: "resume path is not a controlled run directory".into(),
        });
    }
    let parent = run_dir
        .parent()
        .ok_or_else(|| EvidenceRetentionError::IllegalEvidenceRoot {
            reason: "resume run dir has no parent root".into(),
        })?;
    let parent_c = validate_evidence_root(parent, policy)?;
    let run_c = ensure_canonical_within(&parent_c, run_dir)?;
    if !run_dir.join("endurance_checkpoint.jsonl").exists() {
        return Err(EvidenceRetentionError::ResumeUnavailable {
            detail: "resume run dir missing checkpoint".into(),
        });
    }
    assert_not_reparse_path(&run_dir.join("endurance_checkpoint.jsonl"))?;
    Ok(run_c)
}

/// Format a seal/verify failure as a hard fail-closed error (never a warning).
pub fn format_seal_hard_error(err: &EvidenceRetentionError) -> String {
    format!("fail-closed evidence seal/verify error: {err}")
}

/// Resolve the evidence root from optional CLI path / environment.
///
/// Production default (`require_explicit`): fails if neither CLI nor env is set.
pub fn resolve_evidence_root(
    cli_root: Option<&Path>,
    env_override: Option<&str>,
    policy: &EvidenceRootPolicy,
) -> Result<PathBuf, EvidenceRetentionError> {
    if let Some(p) = cli_root {
        return validate_evidence_root(p, policy);
    }
    if let Some(raw) = env_override {
        reject_path_traversal(raw)?;
        return validate_evidence_root(Path::new(raw), policy);
    }
    if let Ok(raw) = std::env::var(EVIDENCE_ROOT_ENV) {
        let raw = raw.trim();
        if !raw.is_empty() {
            reject_path_traversal(raw)?;
            return validate_evidence_root(Path::new(raw), policy);
        }
    }
    if let Ok(raw) = std::env::var(EVIDENCE_DIR_ENV) {
        let raw = raw.trim();
        if !raw.is_empty() {
            reject_path_traversal(raw)?;
            // Legacy: EVIDENCE_DIR may be a per-run directory; use its parent as
            // root when it looks like a controlled run dir, else treat as root.
            let path = PathBuf::from(raw);
            if let Some(name) = path.file_name().and_then(|s| s.to_str())
                && is_controlled_run_dirname(name)
                && let Some(parent) = path.parent()
            {
                return validate_evidence_root(parent, policy);
            }
            return validate_evidence_root(&path, policy);
        }
    }
    if policy.require_explicit {
        return Err(EvidenceRetentionError::MissingEvidenceRoot);
    }
    // Test-only fallback: unique ephemeral root.
    let fallback =
        std::env::temp_dir().join(format!("storyforge_evidence_root_{}", uuid::Uuid::new_v4()));
    validate_evidence_root(&fallback, policy)
}

// ── Run identity / layout ───────────────────────────────────────────────────

/// Whether a directory name is in the controlled `run-<stage>-<uuid>` namespace.
pub fn is_controlled_run_dirname(name: &str) -> bool {
    if !name.starts_with(RUN_DIR_PREFIX) {
        return false;
    }
    // run-<stage>-<uuid> — at least two hyphens after prefix content.
    let rest = &name[RUN_DIR_PREFIX.len()..];
    let mut parts = rest.split('-');
    let stage = parts.next().unwrap_or("");
    if stage.is_empty() || !stage.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    let uuid_part: String = parts.collect::<Vec<_>>().join("-");
    // Accept UUID hyphenated (36) or compact (32).
    (uuid_part.len() == 36 || uuid_part.len() == 32)
        && uuid_part.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Map a run id (`run-stage-uuid`) to its directory name (same string by design).
pub fn run_dirname_for_id(run_id: &str) -> String {
    run_id.to_string()
}

/// Allocate a unique run id of the form `run-<stage>-<uuid>`.
pub fn allocate_run_id(root: &Path, stage: &str) -> Result<String, EvidenceRetentionError> {
    let stage = stage.trim().to_ascii_lowercase();
    if stage.is_empty() || !stage.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(EvidenceRetentionError::InvalidRunId {
            detail: "stage must be non-empty alphanumeric/underscore".into(),
        });
    }
    for _ in 0..8 {
        let id = format!("run-{stage}-{}", uuid::Uuid::new_v4());
        let dir = root.join(run_dirname_for_id(&id));
        if !dir.exists() {
            return Ok(id);
        }
    }
    Err(EvidenceRetentionError::DuplicateRunId {
        run_id: format!("run-{stage}-<exhausted>"),
    })
}

/// Create an exclusive run directory for `run_id` under a validated root.
pub fn prepare_run_dir(root: &Path, run_id: &str) -> Result<PathBuf, EvidenceRetentionError> {
    if !is_controlled_run_dirname(run_id) {
        return Err(EvidenceRetentionError::InvalidRunId {
            detail: "run id must match controlled namespace run-<stage>-<uuid>".into(),
        });
    }
    assert_not_reparse_path(root)?;
    let dir = root.join(run_dirname_for_id(run_id));
    ensure_within_root(root, &dir)?;
    if dir.exists() {
        return Err(EvidenceRetentionError::DuplicateRunId {
            run_id: run_id.into(),
        });
    }
    fs::create_dir_all(&dir)?;
    assert_not_reparse_path(&dir)?;
    ensure_canonical_within(root, &dir)?;
    Ok(dir)
}

fn ensure_within_root(root: &Path, path: &Path) -> Result<(), EvidenceRetentionError> {
    ensure_canonical_within(root, path).map(|_| ())
}

// ── Manifest schema ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Interrupted,
    Completed,
    Failed,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetSummary {
    pub max_calls: u32,
    pub max_turns: u32,
    pub timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDigest {
    /// Path relative to the run directory, using `/` separators only.
    pub relative_path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunManifest {
    pub schema_version: String,
    pub run_id: String,
    pub status: RunStatus,
    pub stage: String,
    pub model_label: String,
    pub budget: BudgetSummary,
    pub commit: String,
    pub branch: String,
    pub evidence_schema_version: String,
    pub sealed_at_unix_ms: u128,
    pub files: Vec<FileDigest>,
    /// Relative-only snapshot dirs (e.g. `campaign_snapshot`), never absolute host paths.
    #[serde(default)]
    pub snapshot_dirs: Vec<String>,
    pub accepted_turn_number: u32,
    pub calls_used: u32,
}

#[derive(Debug, Clone)]
pub struct SealOptions {
    pub run_id: String,
    pub status: RunStatus,
    pub stage: String,
    pub model_label: String,
    pub budget: BudgetSummary,
    pub commit: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResumeContext {
    pub run_id: String,
    pub status: RunStatus,
    pub accepted_turn_number: u32,
    pub next_turn: u32,
    pub calls_used: u32,
    pub data_dir_rel: Option<String>,
    pub campaign_id: Option<String>,
    pub conversation_id: Option<String>,
}

// ── Hashing ─────────────────────────────────────────────────────────────────

pub fn sha256_file(path: &Path) -> Result<(String, u64), EvidenceRetentionError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

fn to_relative_posix(path: &str) -> String {
    path.replace('\\', "/")
}

fn rel_path_under(run_dir: &Path, file: &Path) -> Result<String, EvidenceRetentionError> {
    let rel = file
        .strip_prefix(run_dir)
        .map_err(|_| EvidenceRetentionError::PathOutsideRoot {
            detail: "file is not under run directory".into(),
        })?;
    let s = to_relative_posix(&rel.to_string_lossy());
    if Path::new(&s).is_absolute() || s.contains("..") || s.contains(':') {
        return Err(EvidenceRetentionError::PathOutsideRoot {
            detail: "absolute or escaped relative path refused".into(),
        });
    }
    Ok(s)
}

// ── Seal / verify ───────────────────────────────────────────────────────────

fn is_live_working_dir(name: &str) -> bool {
    // Live mutable campaign state is not part of the sealed evidence envelope.
    // Still scanned for secrets when present, but never hashed/archived.
    name == "campaign_data" || name == "archives" || name == "restore"
}

/// Files that participate in the sealed digest / archive set.
///
/// Exact-set verification must include every evidence-like artifact under the run
/// dir so untracked JSONL / snapshot leaves cannot hide outside the digest table.
fn is_sealed_evidence_file(rel: &str) -> bool {
    if rel == RUN_MANIFEST_FILE {
        return false; // envelope, not a subject digest
    }
    let first = rel.split('/').next().unwrap_or("");
    if first == "campaign_data" || first == "archives" || first == "restore" {
        return false;
    }
    // Probe / temp names are scanned for secrets but not sealed subjects.
    let base = rel.rsplit('/').next().unwrap_or(rel);
    if base.starts_with('.') || base.ends_with(".tmp") {
        return false;
    }
    if REQUIRED_EVIDENCE_FILES.contains(&rel) || OPTIONAL_EVIDENCE_FILES.contains(&rel) {
        return true;
    }
    // Any other JSONL under the run tree is controlled evidence and must be listed.
    if rel.ends_with(".jsonl") {
        return true;
    }
    // Snapshot tree files (never campaign_data).
    first.contains("snapshot") || first.starts_with("campaign_")
}

fn scan_for_forbidden(run_dir: &Path) -> Result<(), EvidenceRetentionError> {
    fn walk(dir: &Path, run_dir: &Path) -> Result<(), EvidenceRetentionError> {
        assert_not_reparse_path(dir)?;
        ensure_canonical_within(run_dir, dir)?;
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            assert_not_reparse_path(&path)?;
            ensure_canonical_within(run_dir, &path)?;
            let name = entry.file_name().to_string_lossy().to_string();
            let ft = entry.file_type()?;
            if ft.is_symlink() {
                return Err(EvidenceRetentionError::ReparsePoint {
                    detail: "symlink entry rejected during scan".into(),
                });
            }
            if path.is_dir() {
                // Always recurse, including live working dirs: secrets must not hide there.
                walk(&path, run_dir)?;
                continue;
            }
            // Scan ALL files, including hidden and .tmp — digest/archive sets are separate.
            // Prefer full read when UTF-8; also scan lossy bytes for marker strings.
            let bytes = fs::read(&path)?;
            let text = String::from_utf8_lossy(&bytes);
            if contains_forbidden_evidence_payload(&text) {
                let rel = rel_path_under(run_dir, &path).unwrap_or_else(|_| "unknown".into());
                return Err(EvidenceRetentionError::ForbiddenPayload { relative_path: rel });
            }
            let _ = name;
        }
        Ok(())
    }
    assert_not_reparse_path(run_dir)?;
    walk(run_dir, run_dir)
}

/// Collect the exact set of sealed subject files currently on disk.
fn collect_actual_sealed_files(run_dir: &Path) -> Result<BTreeSet<String>, EvidenceRetentionError> {
    let mut out = BTreeSet::new();
    fn walk(
        dir: &Path,
        run_dir: &Path,
        out: &mut BTreeSet<String>,
    ) -> Result<(), EvidenceRetentionError> {
        assert_not_reparse_path(dir)?;
        ensure_canonical_within(run_dir, dir)?;
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            assert_not_reparse_path(&path)?;
            ensure_canonical_within(run_dir, &path)?;
            if entry.file_type()?.is_symlink() {
                return Err(EvidenceRetentionError::ReparsePoint {
                    detail: "symlink rejected while collecting sealed files".into(),
                });
            }
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if is_live_working_dir(&name) {
                    continue;
                }
                walk(&path, run_dir, out)?;
                continue;
            }
            let rel = rel_path_under(run_dir, &path)?;
            if is_sealed_evidence_file(&rel) {
                out.insert(rel);
            }
        }
        Ok(())
    }
    walk(run_dir, run_dir, &mut out)?;
    Ok(out)
}

fn collect_run_ids_from_jsonl(path: &Path) -> Result<BTreeSet<String>, EvidenceRetentionError> {
    let mut ids = BTreeSet::new();
    if !path.exists() {
        return Ok(ids);
    }
    for line in read_evidence_lines(path)? {
        if let Some(id) = line.get("run_id").and_then(|v| v.as_str()) {
            ids.insert(id.to_string());
        }
    }
    Ok(ids)
}

fn assert_single_run_id(run_dir: &Path, expected: &str) -> Result<(), EvidenceRetentionError> {
    let paths = EnduranceEvidencePaths::new(run_dir.to_path_buf());
    let mut all = BTreeSet::new();
    for p in [
        &paths.calls_jsonl,
        &paths.turns_jsonl,
        &paths.checkpoint_jsonl,
        &paths.manifest_jsonl,
        &paths.phase_b_jsonl,
    ] {
        for id in collect_run_ids_from_jsonl(p)? {
            all.insert(id);
        }
    }
    if all.is_empty() {
        return Ok(());
    }
    if all.len() > 1 || !all.contains(expected) {
        return Err(EvidenceRetentionError::MixedRunId {
            detail: format!("expected only {expected}"),
        });
    }
    Ok(())
}

fn list_snapshot_dirs(run_dir: &Path) -> Result<Vec<String>, EvidenceRetentionError> {
    let mut out = Vec::new();
    for entry in fs::read_dir(run_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        // Only allow relative snapshot-like dirs; never store absolute paths.
        if name.contains("..") || name.contains(':') || name.contains('/') || name.contains('\\') {
            return Err(EvidenceRetentionError::PathTraversal {
                detail: "illegal snapshot directory name".into(),
            });
        }
        // campaign_data is live mutable state — only seal explicit snapshot dirs.
        if name == "campaign_data" {
            continue;
        }
        if name.contains("snapshot") || name.starts_with("campaign_") {
            out.push(to_relative_posix(&name));
        }
    }
    out.sort();
    Ok(out)
}

fn collect_file_digests(run_dir: &Path) -> Result<Vec<FileDigest>, EvidenceRetentionError> {
    // Digest subjects must equal the actual sealed file set (exact-set).
    let actual = collect_actual_sealed_files(run_dir)?;
    for name in REQUIRED_EVIDENCE_FILES {
        if !actual.contains(*name) {
            return Err(EvidenceRetentionError::MissingRequiredFile {
                relative_path: (*name).into(),
            });
        }
    }
    let mut files = Vec::new();
    for rel in actual {
        let path = run_dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        assert_not_reparse_path(&path)?;
        ensure_canonical_within(run_dir, &path)?;
        let (sha, size) = sha256_file(&path)?;
        files.push(FileDigest {
            relative_path: rel,
            sha256: sha,
            size_bytes: size,
        });
    }
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

fn assert_exact_digest_set(
    run_dir: &Path,
    manifest: &RunManifest,
) -> Result<(), EvidenceRetentionError> {
    let actual = collect_actual_sealed_files(run_dir)?;
    let mut listed = BTreeSet::new();
    for digest in &manifest.files {
        if !listed.insert(digest.relative_path.clone()) {
            return Err(EvidenceRetentionError::FileSetMismatch {
                detail: format!("duplicate digest entry: {}", digest.relative_path),
            });
        }
    }
    if listed != actual {
        let missing: Vec<_> = actual.difference(&listed).cloned().collect();
        let extra: Vec<_> = listed.difference(&actual).cloned().collect();
        return Err(EvidenceRetentionError::FileSetMismatch {
            detail: format!("missing_in_manifest={missing:?} extra_in_manifest={extra:?}"),
        });
    }
    for name in REQUIRED_EVIDENCE_FILES {
        if !listed.contains(*name) {
            return Err(EvidenceRetentionError::MissingRequiredFile {
                relative_path: (*name).into(),
            });
        }
    }
    Ok(())
}

/// Atomically seal a run directory: redact scan, single run-id check, hash files,
/// write `run_manifest.json` via temp+rename.
pub fn seal_run(run_dir: &Path, opts: SealOptions) -> Result<RunManifest, EvidenceRetentionError> {
    if !run_dir.is_dir() {
        return Err(EvidenceRetentionError::IllegalEvidenceRoot {
            reason: "run directory does not exist".into(),
        });
    }
    assert_not_reparse_path(run_dir)?;
    if !is_controlled_run_dirname(&opts.run_id) {
        return Err(EvidenceRetentionError::InvalidRunId {
            detail: "run id not in controlled namespace".into(),
        });
    }

    // Secret scan covers the full tree (including hidden/.tmp/nested/live dirs).
    scan_for_forbidden(run_dir)?;
    assert_single_run_id(run_dir, &opts.run_id)?;

    let paths = EnduranceEvidencePaths::new(run_dir.to_path_buf());
    if !paths.checkpoint_jsonl.exists() {
        return Err(EvidenceRetentionError::MissingRequiredFile {
            relative_path: "endurance_checkpoint.jsonl".into(),
        });
    }
    assert_not_reparse_path(&paths.checkpoint_jsonl)?;

    let (accepted_turn_number, calls_used) =
        if let Some(cp) = read_latest_checkpoint(&paths.checkpoint_jsonl) {
            if cp.run_id != opts.run_id {
                return Err(EvidenceRetentionError::RunIdMismatch {
                    detail: "checkpoint run_id does not match seal options".into(),
                });
            }
            if cp.schema_version != crate::endurance::EnduranceCheckpoint::schema_version() {
                return Err(EvidenceRetentionError::SchemaMismatch {
                    detail: "checkpoint schema drift".into(),
                });
            }
            (cp.accepted_turn_number, cp.calls_used)
        } else {
            return Err(EvidenceRetentionError::ResumeUnavailable {
                detail: "checkpoint present but unreadable or schema-invalid".into(),
            });
        };

    // Truncate model_label / commit / branch to non-sensitive short fields.
    let model_label = opts.model_label.chars().take(64).collect::<String>();
    let commit = opts.commit.chars().take(64).collect::<String>();
    let branch = opts.branch.chars().take(128).collect::<String>();

    let snapshot_dirs = list_snapshot_dirs(run_dir)?;
    let files = collect_file_digests(run_dir)?;

    let manifest = RunManifest {
        schema_version: RETENTION_SCHEMA_VERSION.into(),
        run_id: opts.run_id,
        status: opts.status,
        stage: opts.stage,
        model_label,
        budget: opts.budget,
        commit,
        branch,
        evidence_schema_version: EVIDENCE_SCHEMA_VERSION.into(),
        sealed_at_unix_ms: now_unix_ms(),
        files,
        snapshot_dirs,
        accepted_turn_number,
        calls_used,
    };

    // Digest set must match on-disk sealed subjects before writing the envelope.
    assert_exact_digest_set(run_dir, &manifest)?;

    let serialized = serde_json::to_string_pretty(&manifest)?;
    if contains_forbidden_evidence_payload(&serialized) {
        return Err(EvidenceRetentionError::ForbiddenPayload {
            relative_path: RUN_MANIFEST_FILE.into(),
        });
    }
    for f in &manifest.files {
        if Path::new(&f.relative_path).is_absolute()
            || f.relative_path.contains("..")
            || f.relative_path.contains(':')
        {
            return Err(EvidenceRetentionError::PathOutsideRoot {
                detail: "manifest must not record absolute or escaped paths".into(),
            });
        }
    }

    let dest = run_dir.join(RUN_MANIFEST_FILE);
    storyforge_infra_util::atomic_write_json_str(&dest, &serialized)?;
    Ok(manifest)
}

/// Offline verification: schema, single run id, exact file set, re-hash.
pub fn verify_run(run_dir: &Path) -> Result<RunManifest, EvidenceRetentionError> {
    assert_not_reparse_path(run_dir)?;
    let manifest_path = run_dir.join(RUN_MANIFEST_FILE);
    if !manifest_path.exists() {
        return Err(EvidenceRetentionError::MissingRequiredFile {
            relative_path: RUN_MANIFEST_FILE.into(),
        });
    }
    assert_not_reparse_path(&manifest_path)?;
    let raw = fs::read_to_string(&manifest_path)?;
    if contains_forbidden_evidence_payload(&raw) {
        return Err(EvidenceRetentionError::ForbiddenPayload {
            relative_path: RUN_MANIFEST_FILE.into(),
        });
    }
    let manifest: RunManifest = serde_json::from_str(&raw)?;
    if manifest.schema_version != RETENTION_SCHEMA_VERSION {
        return Err(EvidenceRetentionError::SchemaMismatch {
            detail: format!(
                "expected {RETENTION_SCHEMA_VERSION}, got {}",
                manifest.schema_version
            ),
        });
    }
    if !is_controlled_run_dirname(&manifest.run_id) {
        return Err(EvidenceRetentionError::InvalidRunId {
            detail: "manifest run_id not controlled".into(),
        });
    }
    // Full-tree secret scan must match the seal path (hidden/.tmp/nested included).
    scan_for_forbidden(run_dir)?;
    assert_single_run_id(run_dir, &manifest.run_id)?;
    assert_exact_digest_set(run_dir, &manifest)?;

    // Re-hash every listed file with containment checks.
    for digest in &manifest.files {
        if digest.relative_path.contains("..")
            || Path::new(&digest.relative_path).is_absolute()
            || digest.relative_path.contains(':')
        {
            return Err(EvidenceRetentionError::PathTraversal {
                detail: "manifest relative_path invalid".into(),
            });
        }
        let path = run_dir.join(
            digest
                .relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        if !path.exists() {
            return Err(EvidenceRetentionError::MissingRequiredFile {
                relative_path: digest.relative_path.clone(),
            });
        }
        assert_not_reparse_path(&path)?;
        ensure_canonical_within(run_dir, &path)?;
        let (sha, size) = sha256_file(&path)?;
        if sha != digest.sha256 || size != digest.size_bytes {
            return Err(EvidenceRetentionError::HashMismatch {
                relative_path: digest.relative_path.clone(),
            });
        }
    }
    Ok(manifest)
}

/// Fail-closed resume loader: verifies seal (when present) and extracts next turn.
pub fn load_resume_context(
    run_dir: &Path,
    expected_run_id: &str,
) -> Result<ResumeContext, EvidenceRetentionError> {
    // Prefer mismatch over invalid-format when a sealed run is present so callers
    // cannot resume a different id into an existing run directory.
    let manifest_path = run_dir.join(RUN_MANIFEST_FILE);
    let status = if manifest_path.exists() {
        let m = verify_run(run_dir)?;
        if m.run_id != expected_run_id {
            return Err(EvidenceRetentionError::RunIdMismatch {
                detail: "manifest run_id does not match expected".into(),
            });
        }
        m.status
    } else {
        if !is_controlled_run_dirname(expected_run_id) {
            return Err(EvidenceRetentionError::InvalidRunId {
                detail: "expected run id not controlled".into(),
            });
        }
        // Allow resume from unsealed interrupted runs, but still check ids.
        assert_single_run_id(run_dir, expected_run_id)?;
        RunStatus::Interrupted
    };

    let paths = EnduranceEvidencePaths::new(run_dir.to_path_buf());
    let cp = read_latest_checkpoint(&paths.checkpoint_jsonl).ok_or_else(|| {
        EvidenceRetentionError::ResumeUnavailable {
            detail: "missing readable checkpoint".into(),
        }
    })?;
    if cp.run_id != expected_run_id {
        return Err(EvidenceRetentionError::RunIdMismatch {
            detail: "checkpoint run_id does not match expected".into(),
        });
    }
    if cp.schema_version != crate::endurance::EnduranceCheckpoint::schema_version() {
        return Err(EvidenceRetentionError::SchemaMismatch {
            detail: "checkpoint schema drift on resume".into(),
        });
    }

    Ok(ResumeContext {
        run_id: expected_run_id.into(),
        status,
        accepted_turn_number: cp.accepted_turn_number,
        next_turn: cp.accepted_turn_number.saturating_add(1),
        calls_used: cp.calls_used,
        data_dir_rel: cp.data_dir_rel,
        campaign_id: cp.campaign_id,
        conversation_id: cp.conversation_id,
    })
}

// ── Archive / restore ───────────────────────────────────────────────────────

/// Copy a sealed run into `archive_root/<run_id>/` and re-verify.
///
/// **Audit archive only for completed (or already archived) runs.**
/// Interrupted/running/failed statuses are refused: this pack is not a resume
/// vehicle for live `campaign_data`. Only digest-listed relative files + the
/// run manifest are copied — never live campaign state.
pub fn archive_run(run_dir: &Path, archive_root: &Path) -> Result<PathBuf, EvidenceRetentionError> {
    assert_not_reparse_path(run_dir)?;
    let manifest = verify_run(run_dir)?;
    match manifest.status {
        RunStatus::Completed | RunStatus::Archived => {}
        RunStatus::Running | RunStatus::Interrupted | RunStatus::Failed => {
            return Err(EvidenceRetentionError::ResumeUnavailable {
                detail: "archive is completed-audit only; refuse interrupted/running/failed packs (campaign_data is not portable resume state)".into(),
            });
        }
    }
    assert_not_reparse_path(archive_root)?;
    if !archive_root.exists() {
        fs::create_dir_all(archive_root)?;
    }
    let archive_root_c = canonicalize_existing(archive_root)?;
    let dest = archive_root_c.join(run_dirname_for_id(&manifest.run_id));
    ensure_canonical_within(&archive_root_c, &dest)?;
    if dest.exists() {
        return Err(EvidenceRetentionError::DuplicateRunId {
            run_id: manifest.run_id,
        });
    }
    fs::create_dir_all(&dest)?;
    for digest in &manifest.files {
        if digest.relative_path.contains("..")
            || Path::new(&digest.relative_path).is_absolute()
            || digest.relative_path.contains(':')
        {
            return Err(EvidenceRetentionError::PathTraversal {
                detail: "archive refused invalid relative path".into(),
            });
        }
        let from = run_dir.join(
            digest
                .relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        assert_not_reparse_path(&from)?;
        ensure_canonical_within(run_dir, &from)?;
        let to = dest.join(
            digest
                .relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        ensure_canonical_within(&dest, &to)?;
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&from, &to)?;
        assert_not_reparse_path(&to)?;
    }
    // Rewrite status to Archived without changing subject file hashes.
    let mut archived = manifest.clone();
    archived.status = RunStatus::Archived;
    archived.sealed_at_unix_ms = now_unix_ms();
    let serialized = serde_json::to_string_pretty(&archived)?;
    if contains_forbidden_evidence_payload(&serialized) {
        return Err(EvidenceRetentionError::ForbiddenPayload {
            relative_path: RUN_MANIFEST_FILE.into(),
        });
    }
    storyforge_infra_util::atomic_write_json_str(&dest.join(RUN_MANIFEST_FILE), &serialized)?;
    verify_run(&dest)?;
    Ok(dest)
}

/// Restore an archived run into `restore_root/<run_id>/` and re-verify hashes.
pub fn restore_run(
    archive_dir: &Path,
    restore_root: &Path,
) -> Result<PathBuf, EvidenceRetentionError> {
    assert_not_reparse_path(archive_dir)?;
    let manifest = verify_run(archive_dir)?;
    if !restore_root.exists() {
        fs::create_dir_all(restore_root)?;
    }
    assert_not_reparse_path(restore_root)?;
    let restore_root_c = canonicalize_existing(restore_root)?;
    let dest = restore_root_c.join(run_dirname_for_id(&manifest.run_id));
    ensure_canonical_within(&restore_root_c, &dest)?;
    if dest.exists() {
        return Err(EvidenceRetentionError::DuplicateRunId {
            run_id: manifest.run_id,
        });
    }
    // Copy only digest subjects + manifest (never follow unexpected links).
    fs::create_dir_all(&dest)?;
    for digest in &manifest.files {
        let from = archive_dir.join(
            digest
                .relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        assert_not_reparse_path(&from)?;
        ensure_canonical_within(archive_dir, &from)?;
        let to = dest.join(
            digest
                .relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        ensure_canonical_within(&dest, &to)?;
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&from, &to)?;
    }
    let man_from = archive_dir.join(RUN_MANIFEST_FILE);
    assert_not_reparse_path(&man_from)?;
    fs::copy(&man_from, dest.join(RUN_MANIFEST_FILE))?;
    verify_run(&dest)?;
    Ok(dest)
}

// ── Retention / cleanup ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RetentionOptions {
    pub keep: usize,
    pub protect_run_ids: Vec<String>,
    pub only_statuses: Vec<RunStatus>,
}

#[derive(Debug, Clone)]
pub struct RetentionPlan {
    pub root: PathBuf,
    pub targets: Vec<PathBuf>,
}

/// Plan cleanup: only controlled namespace dirs, only listed statuses (default completed),
/// never the protected / active run, never unknown directories.
pub fn plan_retention_cleanup(
    root: &Path,
    opts: RetentionOptions,
) -> Result<RetentionPlan, EvidenceRetentionError> {
    // Root must not contain traversal markers in the requested form.
    let raw = root.to_string_lossy();
    reject_path_traversal(raw.as_ref())?;
    // Also reject path components after normalization attempts that still include `..`.
    if root
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(EvidenceRetentionError::PathTraversal {
            detail: "retention root contains relative '.'/'..' components".into(),
        });
    }
    // Defend against OS-specific PathBuf display forms that hide `..` after join.
    let display = format!("{}", root.display());
    if display
        .split(['/', '\\'])
        .any(|seg| seg == ".." || seg == ".")
    {
        return Err(EvidenceRetentionError::PathTraversal {
            detail: "retention root display form contains relative segments".into(),
        });
    }
    if !root.is_dir() {
        return Err(EvidenceRetentionError::IllegalEvidenceRoot {
            reason: "retention root is not a directory".into(),
        });
    }
    assert_not_reparse_path(root)?;
    let root_c = canonicalize_existing(root)?;

    let protect: BTreeSet<String> = opts.protect_run_ids.into_iter().collect();
    let only: BTreeSet<RunStatus> = if opts.only_statuses.is_empty() {
        BTreeSet::from([RunStatus::Completed, RunStatus::Archived])
    } else {
        opts.only_statuses.into_iter().collect()
    };

    let mut candidates: Vec<(std::time::SystemTime, PathBuf, String)> = Vec::new();
    for entry in fs::read_dir(&root_c)? {
        let entry = entry?;
        let path = entry.path();
        // Skip / reject reparse candidates — never select junctions for deletion.
        if assert_not_reparse_path(&path).is_err() {
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_controlled_run_dirname(&name) {
            continue;
        }
        if ensure_canonical_within(&root_c, &path).is_err() {
            continue;
        }
        if protect.contains(&name) {
            continue;
        }
        // Skip active running runs always.
        let status = match read_status_quiet(&path) {
            Some(s) => s,
            None => continue, // unknown/unsealed — never auto-delete
        };
        if matches!(status, RunStatus::Running | RunStatus::Interrupted) {
            continue;
        }
        if !only.contains(&status) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        candidates.push((modified, path, name));
    }

    candidates.sort_by_key(|b| std::cmp::Reverse(b.0)); // newest first
    let targets = if candidates.len() > opts.keep {
        candidates
            .into_iter()
            .skip(opts.keep)
            .map(|(_, p, _)| p)
            .collect()
    } else {
        Vec::new()
    };

    Ok(RetentionPlan {
        root: root_c,
        targets,
    })
}

fn read_status_quiet(run_dir: &Path) -> Option<RunStatus> {
    let raw = fs::read_to_string(run_dir.join(RUN_MANIFEST_FILE)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let s = v.get("status")?.as_str()?;
    match s {
        "running" => Some(RunStatus::Running),
        "interrupted" => Some(RunStatus::Interrupted),
        "completed" => Some(RunStatus::Completed),
        "failed" => Some(RunStatus::Failed),
        "archived" => Some(RunStatus::Archived),
        _ => None,
    }
}

/// Apply a retention plan with path-boundary checks. Never deletes outside root
/// or non-controlled directory names.
pub fn apply_retention_cleanup(
    root: &Path,
    plan: &RetentionPlan,
) -> Result<Vec<PathBuf>, EvidenceRetentionError> {
    assert_not_reparse_path(root)?;
    let root_c = canonicalize_existing(root)?;
    if canonicalize_existing(&plan.root)? != root_c {
        return Err(EvidenceRetentionError::PathOutsideRoot {
            detail: "plan root does not match apply root".into(),
        });
    }

    let mut removed = Vec::new();
    for target in &plan.targets {
        let name = target.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !is_controlled_run_dirname(name) {
            return Err(EvidenceRetentionError::UncontrolledPath {
                detail: "target is not a controlled run directory".into(),
            });
        }
        assert_not_reparse_path(target)?;
        ensure_canonical_within(&root_c, target)?;
        if !target.exists() {
            continue;
        }
        // Final status guard.
        if let Some(status) = read_status_quiet(target)
            && matches!(status, RunStatus::Running | RunStatus::Interrupted)
        {
            return Err(EvidenceRetentionError::UncontrolledPath {
                detail: "refusing to delete active/interrupted run".into(),
            });
        }
        fs::remove_dir_all(target)?;
        removed.push(target.clone());
    }
    Ok(removed)
}

// ── Endurance integration helpers ───────────────────────────────────────────

/// Build endurance evidence paths under a validated root + exclusive run dir.
pub fn open_endurance_run_paths(
    root: &Path,
    stage: &str,
) -> Result<(String, EnduranceEvidencePaths), EvidenceRetentionError> {
    let run_id = allocate_run_id(root, stage)?;
    let run_dir = prepare_run_dir(root, &run_id)?;
    Ok((run_id, EnduranceEvidencePaths::new(run_dir)))
}

/// Preflight addition for dry-run: root policy + writable controlled namespace.
pub fn preflight_evidence_root(
    root: &Path,
    policy: &EvidenceRootPolicy,
) -> Result<PathBuf, EvidenceRetentionError> {
    let validated = validate_evidence_root(root, policy)?;
    // Probe write within namespace only.
    let probe_id = allocate_run_id(&validated, "preflight")?;
    let probe_dir = prepare_run_dir(&validated, &probe_id)?;
    let probe_file = probe_dir.join(".write_probe");
    fs::write(&probe_file, b"ok")?;
    // Cleanup probe run dir entirely.
    let _ = fs::remove_dir_all(&probe_dir);
    Ok(validated)
}

/// Redact a path for logs/errors: only basename-ish relative fragments, never env secrets.
pub fn redact_path_for_display(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<path>".into())
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn controlled_dirname_parser() {
        assert!(is_controlled_run_dirname(
            "run-canary-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        ));
        assert!(!is_controlled_run_dirname("scratch-notes"));
        assert!(!is_controlled_run_dirname("run-"));
        assert!(!is_controlled_run_dirname("../run-canary-x"));
    }

    #[test]
    fn traversal_reject() {
        assert!(reject_path_traversal(r"C:\tmp\..\Windows").is_err());
        assert!(reject_path_traversal("/var/evidence/ok").is_ok());
    }
}
