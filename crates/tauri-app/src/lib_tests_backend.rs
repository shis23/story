/// Gate 3 backend-flag whitelist: `.is_sqlite()` / `.is_json()` may appear
/// only in the bootstrap file (`lib.rs`), the facade (`storage_backend.rs`),
/// the SQLite runtime (`sqlite_runtime.rs`) and the named backend adapter
/// (`backend_workflows.rs`). Commands and ordinary application services must
/// never probe the storage backend — they consume the injected workflows and
/// the facade capability contract instead.
///
/// Entries are exact relative paths from `src/` (e.g. `commands/x.rs`); a
/// same-named file in a nested directory cannot match a top-level entry.
const BACKEND_FLAG_WHITELIST: &[&str] = &[
    "lib.rs",
    "storage_backend.rs",
    "sqlite_runtime.rs",
    "backend_workflows.rs",
];

fn contains_backend_flag(source: &str) -> bool {
    source.contains(".is_sqlite(") || source.contains(".is_json(")
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
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders: Vec<String> = Vec::new();
    let mut visited_whitelist: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    visit_rust_files(&src_dir, &mut |path| {
        let relative = path
            .strip_prefix(&src_dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if relative.starts_with("lib_tests") {
            return;
        }
        let source = std::fs::read_to_string(path).unwrap();
        // Whitelist matches the exact src/-relative path: a nested
        // `commands/storage_backend.rs` cannot alias the top-level entry.
        if contains_backend_flag(&source) && !BACKEND_FLAG_WHITELIST.contains(&relative.as_str()) {
            offenders.push(relative.clone());
        }
        if BACKEND_FLAG_WHITELIST.contains(&relative.as_str()) {
            visited_whitelist.insert(relative);
        }
    });

    assert!(
        offenders.is_empty(),
        "backend flags are only allowed in {BACKEND_FLAG_WHITELIST:?}, found in: {offenders:?}"
    );
    for expected in BACKEND_FLAG_WHITELIST {
        assert!(
            visited_whitelist.contains(*expected),
            "whitelist entry {expected} must still exist under src/"
        );
    }
}

/// Commands must not reach into the legacy JSON CharacterStore through the
/// test-only global `get_store()` — the facade's capability-guarded accessors
/// are the only sanctioned path.
#[test]
fn gate3_commands_never_use_the_ambient_character_store() {
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let commands_dir = src_dir.join("commands");
    let mut offenders: Vec<String> = Vec::new();
    visit_rust_files(&commands_dir, &mut |path| {
        let relative = path
            .strip_prefix(&src_dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(path).unwrap();
        if source.contains("get_store()") {
            offenders.push(relative);
        }
    });
    assert!(
        offenders.is_empty(),
        "commands must not use the ambient CharacterStore: {offenders:?}"
    );
}
