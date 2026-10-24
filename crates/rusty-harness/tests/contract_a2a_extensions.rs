//! Feature: A2A extensions
//! Requirement: docs/specs/issue-3/requirements.md#R19
//!
//! Scenario: The six extension schemas match the checked-in files
//!     Given the payload types of task/v1, participants/v1, plan/v1, channels/v1,
//!     commands/v1 and voice/v1
//!     When their JSON schemas are generated
//!     Then each equals `specs/a2a-ext/<name>.v1.schema.json` (`UPDATE_SNAPSHOTS=1` rewrites)

use std::path::Path;

use rusty_harness::harness::core_loop::task::ext;

#[test]
fn contract_a2a_extension_schemas() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../specs/a2a-ext");
    let all = ext::all();
    assert_eq!(all.len(), 6);
    let mut stale = Vec::new();
    for extension in &all {
        assert_eq!(
            extension.uri,
            format!(
                "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/{}/v1",
                extension.name
            )
        );
        let path = dir.join(format!("{}.v1.schema.json", extension.name));
        let mut json = serde_json::to_string_pretty(&extension.schema).expect("serializes");
        json.push('\n');
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &json).expect("writes the schema");
        }
        let published: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
                .unwrap_or(serde_json::Value::Null);
        let generated: serde_json::Value = serde_json::from_str(&json).unwrap();
        if published != generated {
            stale.push(path.display().to_string());
        }
    }
    assert!(
        stale.is_empty(),
        "stale extension schemas, rerun with UPDATE_SNAPSHOTS=1: {stale:#?}"
    );
    let uris = ext::uris();
    assert!(
        uris.iter()
            .all(|u| u.starts_with(rusty_harness::harness::core_loop::task::EXT_PREFIX))
    );
}
