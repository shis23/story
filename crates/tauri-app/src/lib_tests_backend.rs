/// Gate 3 backend-flag whitelist: `.is_sqlite()` / `.is_json()` may appear
/// only in the AppState bootstrap (`app-service/lib.rs`), the facade
/// (`storage_backend.rs`), the SQLite runtime (`sqlite_runtime.rs`) and the
/// named backend adapter (`backend_workflows.rs`). Commands and ordinary
/// application services must never probe the storage backend — they consume
/// the injected workflows and the facade capability contract instead.
///
/// The application layer spans two crates since the host-agnostic services
/// moved to `storyforge-app-service`; both `src/` trees are scanned. Entries are
/// exact `<crate>/<path from src/>` paths (e.g. `tauri-app/commands/x.rs`); a
/// same-named file in another crate or a nested directory cannot match.
/// `tauri-app` has no entry: the Tauri shell never probes the backend.
const BACKEND_FLAG_WHITELIST: &[&str] = &[
    "app-service/lib.rs",
    "app-service/storage_backend.rs",
    "app-service/sqlite_runtime.rs",
    "app-service/backend_workflows.rs",
    // 三审9：启动恢复协调模块，按 is_sqlite() 分派——与 backend_workflows 同性质。
    "app-service/startup_recovery.rs",
];

fn contains_backend_flag(source: &str) -> bool {
    source.contains(".is_sqlite(") || source.contains(".is_json(")
}

/// `(crate label, src/ dir)` for every crate that holds application-layer code.
fn application_source_roots() -> Vec<(&'static str, std::path::PathBuf)> {
    let tauri_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let service_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("app-service")
        .join("src");
    vec![("tauri-app", tauri_src), ("app-service", service_src)]
}

/// Recursively visit every `*.rs` file under `dir` (any nesting depth) so a
/// future nested module cannot bypass the whitelist scan.
fn visit_rust_files(dir: &std::path::Path, visit: &mut impl FnMut(&std::path::Path)) {
    for entry in std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read_dir {}: {error}", dir.display()))
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            visit_rust_files(&path, visit);
        } else if path.extension().is_some_and(|e| e == "rs") {
            visit(&path);
        }
    }
}

#[test]
fn gate3_backend_flag_whitelist_is_exactly_bootstrap_facade_and_adapter() {
    let mut offenders: Vec<String> = Vec::new();
    let mut visited_whitelist: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    for (label, src_dir) in application_source_roots() {
        visit_rust_files(&src_dir, &mut |path| {
            let in_crate = path
                .strip_prefix(&src_dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let file_name = path.file_name().unwrap().to_string_lossy();
            if file_name.starts_with("lib_tests") {
                return;
            }
            let relative = format!("{label}/{in_crate}");
            let source = std::fs::read_to_string(path).unwrap();
            // Whitelist matches the exact crate-qualified path: a nested
            // `commands/storage_backend.rs` cannot alias the top-level entry.
            if contains_backend_flag(&source)
                && !BACKEND_FLAG_WHITELIST.contains(&relative.as_str())
            {
                offenders.push(relative.clone());
            }
            if BACKEND_FLAG_WHITELIST.contains(&relative.as_str()) {
                visited_whitelist.insert(relative);
            }
        });
    }

    assert!(
        offenders.is_empty(),
        "backend flags are only allowed in {BACKEND_FLAG_WHITELIST:?}, found in: {offenders:?}"
    );
    for expected in BACKEND_FLAG_WHITELIST {
        assert!(
            visited_whitelist.contains(*expected),
            "whitelist entry {expected} must still exist"
        );
    }
}

/// Commands — the Tauri shells in `tauri-app/src/commands` and their
/// host-agnostic halves in `app-service/src/services` — must not reach into the
/// legacy JSON CharacterStore through the test-only global `get_store()`; the
/// facade's capability-guarded accessors are the only sanctioned path.
#[test]
fn gate3_commands_never_use_the_ambient_character_store() {
    let mut offenders: Vec<String> = Vec::new();
    let mut scanned = 0usize;
    for (label, src_dir) in application_source_roots() {
        let command_dir = match label {
            "tauri-app" => src_dir.join("commands"),
            _ => src_dir.join("services"),
        };
        visit_rust_files(&command_dir, &mut |path| {
            scanned += 1;
            let relative = path
                .strip_prefix(&src_dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let source = std::fs::read_to_string(path).unwrap();
            if source.contains("get_store()") {
                offenders.push(format!("{label}/{relative}"));
            }
        });
    }
    assert!(scanned > 0, "command sources must be found");
    assert!(
        offenders.is_empty(),
        "commands must not use the ambient CharacterStore: {offenders:?}"
    );
}
