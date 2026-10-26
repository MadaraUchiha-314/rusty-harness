//! Embeds the crate's assets (R33): every `prompts/**/*.md` as a prompt source and every
//! `strings/<locale>.json` as a catalog, and generates the `StringKey` constants so a
//! missing key is a compile error.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    println!("cargo:rerun-if-changed=prompts");
    println!("cargo:rerun-if-changed=strings");

    let prompts_dir = manifest.join("prompts");
    let mut files = Vec::new();
    walk(&prompts_dir, &mut files);
    files.sort();
    let mut prompts = String::from(
        "/// Every prompt file, as `(crate-relative path, contents)`.\npub static PROMPT_SOURCES: &[(&str, &str)] = &[\n",
    );
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
        let rel = file
            .strip_prefix(&prompts_dir)
            .expect("under prompts/")
            .to_string_lossy()
            .replace('\\', "/");
        writeln!(
            prompts,
            "    ({rel:?}, include_str!({:?})),",
            file.display()
        )
        .expect("write");
    }
    prompts.push_str("];\n");
    fs::write(out.join("prompts.rs"), prompts).expect("write prompts.rs");

    let strings_dir = manifest.join("strings");
    let mut locales = BTreeMap::new();
    for entry in fs::read_dir(&strings_dir).expect("strings/ exists") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "json") {
            println!("cargo:rerun-if-changed={}", path.display());
            let tag = path
                .file_stem()
                .expect("stem")
                .to_string_lossy()
                .to_string();
            locales.insert(tag, path);
        }
    }
    let en = locales
        .get("en")
        .expect("strings/en.json is the source of truth");
    let text = fs::read_to_string(en).expect("read en.json");
    let map: BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&text).expect("en.json is a flat JSON object");

    let mut tree = Node::default();
    for key in map.keys() {
        let segments: Vec<&str> = key.split('.').collect();
        let (leaf, dirs) = segments
            .split_last()
            .expect("a key has at least one segment");
        let mut node = &mut tree;
        for dir in dirs {
            node = node.children.entry((*dir).to_owned()).or_default();
        }
        node.leaves.push(((*leaf).to_owned(), key.clone()));
    }
    let mut keys = String::from(
        "/// The string catalog's keys, generated from `strings/en.json` (R33.6).\n#[allow(unreachable_pub, clippy::module_inception, missing_docs)]\npub mod keys {\n    use super::StringKey;\n",
    );
    emit(&tree, 1, &mut keys);
    keys.push_str("}\n");
    let mut catalogs = String::from(
        "/// Every embedded locale, as `(tag, JSON)`.\npub static CATALOG_SOURCES: &[(&str, &str)] = &[\n",
    );
    for (tag, path) in &locales {
        writeln!(
            catalogs,
            "    ({tag:?}, include_str!({:?})),",
            path.display()
        )
        .expect("write");
    }
    catalogs.push_str("];\n");
    fs::write(out.join("strings.rs"), keys + &catalogs).expect("write strings.rs");
}

/// One module of the generated key tree.
#[derive(Default)]
struct Node {
    children: BTreeMap<String, Node>,
    leaves: Vec<(String, String)>,
}

fn emit(node: &Node, depth: usize, out: &mut String) {
    let pad = "    ".repeat(depth);
    for (leaf, full) in &node.leaves {
        writeln!(
            out,
            "{pad}/// `{full}`\n{pad}pub const {}: StringKey = StringKey::__new({full:?});",
            ident(leaf).to_uppercase()
        )
        .expect("write");
    }
    for (name, child) in &node.children {
        writeln!(
            out,
            "{pad}pub mod {} {{\n{pad}    use super::StringKey;",
            ident(name)
        )
        .expect("write");
        emit(child, depth + 1, out);
        writeln!(out, "{pad}}}").expect("write");
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

fn ident(segment: &str) -> String {
    let mut s: String = segment
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}
