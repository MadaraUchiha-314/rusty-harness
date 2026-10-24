//! Contract: the crate's module tree is the one `design.md` draws (R2.3).
//!
//! The design's tree block is the source of truth; this test parses it and expects one
//! file per entry under `src/`. A module the design does not list may exist (nested
//! sub-modules the tree mentions only in comments), but one it lists may not be missing.

use std::path::{Path, PathBuf};

/// Entries under `src/` of the design's first fenced `text` block, as crate-relative paths.
fn expected_modules(design: &str) -> Vec<PathBuf> {
    let block = design
        .split("```text\n")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("design.md has a text block with the module tree");
    let mut stack: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for line in block.lines() {
        let body = line.split('#').next().unwrap_or("").trim_end();
        if body.trim().is_empty() {
            continue;
        }
        let depth = (body.len() - body.trim_start().len()) / 2;
        stack.truncate(depth);
        let names = body
            .split_whitespace()
            .filter(|tok| !tok.starts_with('['))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let Some(first) = names.first() else { continue };
        for name in &names {
            let mut path = stack.clone();
            path.push(name.trim_end_matches('/').to_owned());
            if path.first().map(String::as_str) == Some("crates/rusty-harness")
                && path.get(1).map(String::as_str) == Some("src")
                && path.len() > 2
                && !path[2].starts_with("bin")
            // the binary lands with task 25 of tasks.md
            {
                let rel: PathBuf = path[1..].iter().collect();
                if name.ends_with('/') {
                    out.push(rel.join("mod.rs"));
                } else if Path::new(name).extension().is_some_and(|ext| ext == "rs") {
                    out.push(rel);
                }
            }
        }
        if first.ends_with('/') {
            stack.push(first.trim_end_matches('/').to_owned());
        }
    }
    out
}

#[test]
fn contract_module_tree_matches_design() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let design = std::fs::read_to_string(crate_dir.join("../../docs/specs/issue-3/design.md"))
        .expect("design.md is readable");
    let expected = expected_modules(&design);
    assert!(
        expected.len() > 30,
        "the tree block parsed into only {} entries",
        expected.len()
    );
    let missing: Vec<_> = expected
        .iter()
        .filter(|p| !crate_dir.join(p).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "modules the design lists but the crate lacks: {missing:#?}"
    );
}
