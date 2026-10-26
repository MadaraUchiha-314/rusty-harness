//! Contracts over the crate's assets (R33): every prompt fragment is a file and every
//! file a fragment, every locale matches `en`, and no prompt is a literal in code.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusty_harness::assets::{Catalog, PROMPT_SOURCES, PromptRegistry, placeholders};
use rusty_harness::registry::Entity;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            walk(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// Feature: Prompts as assets
/// Requirement: docs/specs/issue-3/requirements.md#R33
///
/// Scenario: Every registered prompt fragment resolves to a markdown file
///     Given the prompt registry built at compile time
///     When every fragment's source is looked up under prompts/ and every file under prompts/ is parsed
///     Then the two sets are equal and every fragment parses with a matching id and placeholder list
#[test]
fn assets_every_prompt_fragment_resolves_to_a_file() {
    let registry = PromptRegistry::builtin();
    let prompts = crate_dir().join("prompts");
    let mut files = Vec::new();
    walk(&prompts, "md", &mut files);
    let on_disk: BTreeSet<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(&prompts)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    let embedded: BTreeSet<String> = PROMPT_SOURCES
        .iter()
        .map(|(p, _)| (*p).to_owned())
        .collect();
    assert_eq!(
        on_disk, embedded,
        "prompts/ and the embedded sources differ: rebuild"
    );
    let registered: BTreeSet<String> = registry.all().map(|f| f.source().to_owned()).collect();
    assert_eq!(
        registered, embedded,
        "every embedded file is a registered fragment"
    );
    for fragment in registry.all() {
        assert!(
            prompts.join(fragment.source()).is_file(),
            "{} has no file",
            fragment.entity_id()
        );
        assert!(
            !fragment.description().is_empty(),
            "{} has no description",
            fragment.entity_id()
        );
    }
}

/// Scenario: A missing key in a locale falls back to en and a missing key in en fails the build
///     Given strings/en.json and every other locale (shipped or a test fixture)
///     Then every key of every other locale exists in en with the same placeholder set
///     And every pattern is inside the supported subset
///     (a key missing in en cannot be referenced: `StringKey` constants are generated from en)
#[test]
fn assets_placeholder_sets_match_en() {
    let mut catalog = Catalog::builtin();
    let fixture = crate_dir().join("tests/fixtures/strings/xx.json");
    catalog
        .add_locale("xx", &std::fs::read_to_string(&fixture).expect("fixture"))
        .expect("fixture parses");
    let en_keys: BTreeSet<&str> = catalog.keys("en").collect();
    assert!(!en_keys.is_empty());
    for key in &en_keys {
        placeholders(catalog.pattern("en", key).unwrap())
            .unwrap_or_else(|e| panic!("en {key}: {e}"));
    }
    let locales: Vec<String> = catalog.locales().map(str::to_owned).collect();
    for locale in locales.iter().filter(|l| *l != "en") {
        for key in catalog.keys(locale) {
            assert!(
                en_keys.contains(key),
                "{locale}.json has {key}, which en.json lacks"
            );
            let en = placeholders(catalog.pattern("en", key).unwrap()).unwrap();
            let other = placeholders(catalog.pattern(locale, key).unwrap())
                .unwrap_or_else(|e| panic!("{locale} {key}: {e}"));
            assert_eq!(
                en, other,
                "{locale}.json {key}: placeholders differ from en"
            );
        }
    }
}

/// Scenario: No prompt is a string literal in code
///     Given every Rust source of the crate outside its tests
///     Then no call builds a `Prompt` from a literal, nor hands one to `Prompt::data` as authored text
#[test]
fn assets_no_raw_prompt_literals() {
    let mut files = Vec::new();
    walk(&crate_dir().join("src"), "rs", &mut files);
    let mut offenders = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("source");
        let mut in_tests = false;
        for (n, line) in text.lines().enumerate() {
            if line.contains("#[cfg(test)]") {
                in_tests = true;
            }
            if in_tests {
                continue;
            }
            let call = line.find("Prompt::data(").map(|i| &line[i..]);
            if let Some(call) = call
                && let Some(after_role) = call.split_once(',').map(|(_, rest)| rest.trim_start())
                && after_role.starts_with('"')
            {
                offenders.push(format!("{}:{}", file.display(), n + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "Prompt::data with a literal (authored text belongs in prompts/*.md): {offenders:#?}"
    );
}
