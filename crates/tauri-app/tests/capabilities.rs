use std::collections::BTreeSet;

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
