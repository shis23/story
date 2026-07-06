use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn default_capability_permissions() -> BTreeSet<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("capabilities")
        .join("default.json");
    let raw = std::fs::read_to_string(path).expect("read default capability");
    let value: Value = serde_json::from_str(&raw).expect("parse default capability");
    value["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .filter_map(|entry| entry.as_str().map(str::to_owned))
        .collect()
}

fn workspace_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read_frontend_sources() -> String {
    fn visit(dir: &Path, out: &mut String) {
        for entry in std::fs::read_dir(dir).expect("read frontend src") {
            let entry = entry.expect("read dir entry");
            let path = entry.path();
            if path.is_dir() {
                visit(&path, out);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("js" | "vue")
            ) {
                out.push_str(&std::fs::read_to_string(&path).expect("read frontend source"));
                out.push('\n');
            }
        }
    }

    let mut out = String::new();
    visit(&workspace_root().join("frontend").join("src"), &mut out);
    out
}

fn frontend_imports_helper(frontend: &str, plugin: &str, helper: &str) -> bool {
    frontend
        .lines()
        .any(|line| line.contains(plugin) && line.contains(helper))
}

#[test]
fn default_capability_uses_narrow_dialog_and_file_permissions() {
    let permissions = default_capability_permissions();

    assert!(permissions.contains("core:default"));
    assert!(permissions.contains("dialog:allow-open"));
    assert!(permissions.contains("dialog:allow-save"));
    assert!(permissions.contains("dialog:allow-message"));
    assert!(permissions.contains("dialog:allow-ask"));
    assert!(permissions.contains("fs:allow-read-file"));
    assert!(permissions.contains("fs:allow-write-file"));

    assert!(!permissions.contains("dialog:default"));
    assert!(!permissions.contains("fs:default"));
}

#[test]
fn frontend_file_dialog_usage_has_matching_capabilities() {
    let permissions = default_capability_permissions();
    let frontend = read_frontend_sources();

    let required = [
        ("@tauri-apps/plugin-dialog", "open", "dialog:allow-open"),
        ("@tauri-apps/plugin-dialog", "save", "dialog:allow-save"),
        (
            "@tauri-apps/plugin-dialog",
            "message",
            "dialog:allow-message",
        ),
        ("@tauri-apps/plugin-dialog", "ask", "dialog:allow-ask"),
        ("@tauri-apps/plugin-fs", "readFile", "fs:allow-read-file"),
        (
            "@tauri-apps/plugin-fs",
            "writeBinaryFile",
            "fs:allow-write-file",
        ),
    ];

    for (plugin, helper, permission) in required {
        assert!(
            !frontend_imports_helper(&frontend, plugin, helper) || permissions.contains(permission),
            "frontend imports `{helper}` from `{plugin}` but default capability lacks `{permission}`"
        );
    }

    for unsupported in [
        "readDir",
        "mkdir",
        "exists",
        "remove",
        "rename",
        "copyFile",
        "stat",
        "lstat",
    ] {
        assert!(
            !frontend_imports_helper(&frontend, "@tauri-apps/plugin-fs", unsupported),
            "frontend uses unsupported fs helper `{unsupported}`; add the narrow capability deliberately"
        );
    }
}
