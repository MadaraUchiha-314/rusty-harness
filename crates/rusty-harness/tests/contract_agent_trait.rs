//! Feature: The core loop
//! Requirement: docs/specs/issue-3/requirements.md#R11
//!
//! Scenario: The Agent trait matches sherma's base agent
//!     Given design.md's mapping table from sherma's `Agent` to rusty-harness's
//!     When the trait's source is read
//!     Then every rusty-harness operation the table names is a method of the trait

use std::path::Path;

#[test]
fn contract_agent_trait_matches_sherma_shape() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/harness/core_loop/agent.rs")).unwrap();
    let design = std::fs::read_to_string(root.join("../../docs/specs/issue-3/design.md")).unwrap();
    let trait_body = source
        .split("pub trait Agent: Entity {")
        .nth(1)
        .and_then(|rest| rest.split("\n    }\n}").next())
        .expect("the Agent trait");
    let table: Vec<&str> = design
        .lines()
        .skip_while(|l| !l.starts_with("| sherma `Agent`"))
        .skip(2)
        .take_while(|l| l.starts_with('|'))
        .collect();
    assert_eq!(table.len(), 4, "four sherma operations: {table:#?}");
    for row in &table {
        // `\|` inside the table's code spans is an escaped pipe, not a column break.
        let row = row.replace("\\|", "\u{1}");
        let ours = row.split('|').nth(2).unwrap().trim().replace('\u{1}', "|");
        let ours = ours.as_str();
        let method = ours.trim_matches('`').split('(').next().unwrap();
        assert!(
            trait_body.contains(&format!("fn {method}(")),
            "design maps a sherma operation to `{ours}`, which the Agent trait lacks"
        );
    }
    for required in [
        "fn card(",
        "fn schemas(",
        "fn send_message(",
        "fn cancel_task(",
    ] {
        assert!(trait_body.contains(required), "{required}");
    }
    assert!(
        trait_body.contains("a2a::AgentCard")
            && trait_body.contains("a2a::StreamResponse")
            && trait_body.contains("a2a::Task"),
        "typed with the official A2A types"
    );
}
