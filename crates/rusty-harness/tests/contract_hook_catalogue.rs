//! Contract: the hook catalogue covers every point of sherma's twenty, as the design's
//! mapping table says (R4.11), and every point the design's catalogue table lists exists.

use std::collections::BTreeSet;
use std::path::Path;

use rusty_harness::harness::hooks::HookPoint;

fn design() -> String {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(crate_dir.join("../../docs/specs/issue-3/design.md"))
        .expect("design.md is readable")
}

/// Dotted `area.operation` names inside backticks in `text`.
fn dotted_names(text: &str) -> BTreeSet<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|tok| {
            tok.contains('.')
                && !tok.contains(' ')
                && !tok.contains('(')
                && tok
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_')
        })
        .map(str::to_owned)
        .collect()
}

#[test]
fn contract_hook_catalogue_covers_sherma_points() {
    let design = design();
    let table = design
        .split("**sherma's twenty points mapped**")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").nth(1))
        .expect("the sherma mapping table follows its heading");
    let mapped: BTreeSet<String> = table
        .lines()
        .filter(|l| l.starts_with("| `"))
        .flat_map(|l| dotted_names(l.rsplit('|').nth(1).unwrap_or("")))
        .collect();
    assert!(mapped.len() >= 10, "parsed only {mapped:?}");
    let missing: Vec<_> = mapped
        .iter()
        .filter(|name| HookPoint::from_name(name).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "points the design maps sherma onto but the catalogue lacks: {missing:?}"
    );
}

#[test]
fn contract_hook_catalogue_lists_every_point_of_the_design_table() {
    let design = design();
    let table = design
        .split("**The catalogue**")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").nth(1))
        .expect("the catalogue table follows its heading");
    let listed: BTreeSet<String> = table
        .lines()
        .filter(|l| l.starts_with("| `"))
        .flat_map(|l| dotted_names(l.split('|').nth(1).unwrap_or("")))
        .collect();
    assert!(listed.len() >= 30, "parsed only {} points", listed.len());
    let missing: Vec<_> = listed
        .iter()
        .filter(|name| HookPoint::from_name(name).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "points the design lists but the catalogue lacks: {missing:?}"
    );
    let extra: Vec<_> = HookPoint::ALL
        .iter()
        .map(|p| p.name().to_owned())
        .filter(|n| !listed.contains(n))
        .collect();
    assert!(
        extra.is_empty(),
        "points in the catalogue the design does not list: {extra:?}"
    );
}
