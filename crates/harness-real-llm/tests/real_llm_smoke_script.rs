//! `run-real-llm-smoke.ps1` 的无网络回归测试。

#[cfg(windows)]
#[test]
fn eval_dry_run_does_not_require_credentials_or_paid_authorization() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let script = repo_root.join("scripts").join("run-real-llm-smoke.ps1");
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .args(["-Suite", "eval", "-DryRun"])
        .current_dir(&repo_root)
        .env_remove("LLM_BASE_URL")
        .env_remove("LLM_API_KEY")
        .env_remove("LLM_MODEL")
        .env_remove("STORYFORGE_EVAL_REAL_LLM")
        .env_remove("STORYFORGE_EVAL_FIXTURE_CARD")
        .output()
        .expect("launch PowerShell dry-run");

    assert!(
        output.status.success(),
        "DryRun must succeed without credentials. stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("DRY RUN:"));
    assert!(
        stdout.contains("eval_real_llm_pipeline_write_synthetic_chronicle_accept_across_epoch")
    );
}

#[cfg(windows)]
#[test]
fn eval_real_run_fails_before_cargo_when_fixture_override_is_missing() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let script = repo_root.join("scripts").join("run-real-llm-smoke.ps1");
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .args(["-Suite", "eval"])
        .current_dir(&repo_root)
        .env("LLM_BASE_URL", "https://example.invalid/v1")
        .env("LLM_API_KEY", "test-only")
        .env("LLM_MODEL", "test-model")
        .env("STORYFORGE_EVAL_REAL_LLM", "1")
        .env("STORYFORGE_EVAL_MAX_CALLS", "96")
        .env("STORYFORGE_EVAL_MAX_TURNS", "16")
        .env("STORYFORGE_EVAL_TIMEOUT_SECS", "180")
        .env_remove("STORYFORGE_EVAL_FIXTURE_CARD")
        .output()
        .expect("launch PowerShell fixture preflight");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("STORYFORGE_EVAL_FIXTURE_CARD"));
    assert!(!stdout.contains("RUN: cargo test"));
}
