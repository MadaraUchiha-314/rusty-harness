//! The string catalog: every user-facing string from `strings/<locale>.json`, keyed, with
//! `en` as the source of truth, an ICU `MessageFormat` subset for arguments, and
//! per-key fallback to `en` (R33.6–9, Q18).

use core::fmt;
use std::collections::BTreeMap;

include!(concat!(env!("OUT_DIR"), "/strings.rs"));

/// A key of the catalog. Only the generated constants under [`keys`] make one, so a
/// missing key is a compile error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StringKey(&'static str);

impl StringKey {
    #[doc(hidden)]
    #[must_use]
    pub const fn __new(key: &'static str) -> Self {
        Self(key)
    }

    /// The dotted key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for StringKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// An argument value for a message.
#[derive(Clone, Debug, PartialEq)]
pub enum FmtValue {
    /// A number, for `plural` and `#`.
    Number(i64),
    /// Text, for `{name}` and `select`.
    Text(String),
}

impl fmt::Display for FmtValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{n}"),
            Self::Text(t) => f.write_str(t),
        }
    }
}

/// The arguments of a message.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FmtArgs(BTreeMap<String, FmtValue>);

impl FmtArgs {
    /// No arguments.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A text argument.
    #[must_use]
    pub fn text(mut self, name: &str, value: impl Into<String>) -> Self {
        self.0.insert(name.to_owned(), FmtValue::Text(value.into()));
        self
    }

    /// A number argument.
    #[must_use]
    pub fn number(mut self, name: &str, value: i64) -> Self {
        self.0.insert(name.to_owned(), FmtValue::Number(value));
        self
    }
}

/// A message pattern that is not in the supported subset.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("message pattern: {0}")]
pub struct PatternError(String);

/// One parsed piece of a pattern.
#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Text(String),
    Arg(String),
    Hash,
    Plural {
        name: String,
        cases: Vec<(String, Vec<Piece>)>,
    },
    Select {
        name: String,
        cases: Vec<(String, Vec<Piece>)>,
    },
}

fn parse(pattern: &str) -> Result<Vec<Piece>, PatternError> {
    let mut chars = pattern.chars().peekable();
    parse_pieces(&mut chars, 0)
}

fn parse_pieces(
    chars: &mut core::iter::Peekable<core::str::Chars<'_>>,
    depth: usize,
) -> Result<Vec<Piece>, PatternError> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    while let Some(&c) = chars.peek() {
        match c {
            '}' if depth > 0 => break,
            '{' => {
                chars.next();
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                pieces.push(parse_arg(chars)?);
            }
            '#' if depth > 0 => {
                chars.next();
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                pieces.push(Piece::Hash);
            }
            _ => {
                chars.next();
                text.push(c);
            }
        }
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    Ok(pieces)
}

fn parse_arg(
    chars: &mut core::iter::Peekable<core::str::Chars<'_>>,
) -> Result<Piece, PatternError> {
    let mut head = String::new();
    loop {
        match chars.next() {
            Some('}') => {
                let name = head.trim();
                if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return Err(PatternError(format!("bad argument name {name:?}")));
                }
                return Ok(Piece::Arg(name.to_owned()));
            }
            Some(',') => break,
            Some(c) => head.push(c),
            None => return Err(PatternError("unterminated argument".into())),
        }
    }
    let name = head.trim().to_owned();
    let mut kind = String::new();
    loop {
        match chars.next() {
            Some(',') => break,
            Some(c) => kind.push(c),
            None => return Err(PatternError("unterminated argument".into())),
        }
    }
    let kind = kind.trim().to_owned();
    if kind != "plural" && kind != "select" {
        return Err(PatternError(format!(
            "unsupported argument type {kind:?} (plural and select only)"
        )));
    }
    let mut cases = Vec::new();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        match chars.peek() {
            Some('}') => {
                chars.next();
                break;
            }
            Some(_) => {
                let mut selector = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '{' || c.is_whitespace() {
                        break;
                    }
                    selector.push(c);
                    chars.next();
                }
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
                if chars.next() != Some('{') {
                    return Err(PatternError(format!("case {selector:?} has no body")));
                }
                let body = parse_pieces(chars, 1)?;
                if chars.next() != Some('}') {
                    return Err(PatternError(format!("case {selector:?} is unterminated")));
                }
                cases.push((selector, body));
            }
            None => return Err(PatternError("unterminated plural/select".into())),
        }
    }
    if !cases.iter().any(|(s, _)| s == "other") {
        return Err(PatternError(format!("{kind} {name:?} has no `other` case")));
    }
    Ok(if kind == "plural" {
        Piece::Plural { name, cases }
    } else {
        Piece::Select { name, cases }
    })
}

fn render(pieces: &[Piece], args: &FmtArgs, hash: Option<i64>) -> String {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Hash => {
                if let Some(n) = hash {
                    out.push_str(&n.to_string());
                }
            }
            Piece::Arg(name) => {
                if let Some(v) = args.0.get(name) {
                    out.push_str(&v.to_string());
                } else {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
            }
            Piece::Plural { name, cases } => {
                let n = match args.0.get(name) {
                    Some(FmtValue::Number(n)) => *n,
                    Some(FmtValue::Text(t)) => t.parse().unwrap_or(0),
                    None => 0,
                };
                let exact = format!("={n}");
                let category = if n == 1 { "one" } else { "other" };
                let body = cases
                    .iter()
                    .find(|(s, _)| *s == exact)
                    .or_else(|| cases.iter().find(|(s, _)| s == category))
                    .or_else(|| cases.iter().find(|(s, _)| s == "other"))
                    .map(|(_, b)| b.as_slice())
                    .unwrap_or_default();
                out.push_str(&render(body, args, Some(n)));
            }
            Piece::Select { name, cases } => {
                let value = args
                    .0
                    .get(name)
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let body = cases
                    .iter()
                    .find(|(s, _)| *s == value)
                    .or_else(|| cases.iter().find(|(s, _)| s == "other"))
                    .map(|(_, b)| b.as_slice())
                    .unwrap_or_default();
                out.push_str(&render(body, args, hash));
            }
        }
    }
    out
}

/// The argument names a pattern uses (for the cross-locale contract test).
///
/// # Errors
///
/// [`PatternError`] when the pattern is outside the subset.
pub fn placeholders(pattern: &str) -> Result<std::collections::BTreeSet<String>, PatternError> {
    fn walk(pieces: &[Piece], out: &mut std::collections::BTreeSet<String>) {
        for piece in pieces {
            match piece {
                Piece::Arg(n) => {
                    out.insert(n.clone());
                }
                Piece::Plural { name, cases } | Piece::Select { name, cases } => {
                    out.insert(name.clone());
                    for (_, body) in cases {
                        walk(body, out);
                    }
                }
                Piece::Text(_) | Piece::Hash => {}
            }
        }
    }
    let mut out = std::collections::BTreeSet::new();
    walk(&parse(pattern)?, &mut out);
    Ok(out)
}

/// Formats one pattern with arguments.
///
/// # Errors
///
/// [`PatternError`] when the pattern is outside the subset.
pub fn format(pattern: &str, args: &FmtArgs) -> Result<String, PatternError> {
    Ok(render(&parse(pattern)?, args, None))
}

/// The catalog: one map per locale, `en` always present.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    locales: BTreeMap<String, BTreeMap<String, String>>,
}

impl Catalog {
    /// The locales embedded at build time.
    ///
    /// # Panics
    ///
    /// When an embedded catalog is not a flat JSON object of strings; the contract tests
    /// catch that before release.
    #[must_use]
    pub fn builtin() -> Self {
        let mut catalog = Self::default();
        for (tag, json) in CATALOG_SOURCES {
            catalog
                .add_locale(tag, json)
                .unwrap_or_else(|e| panic!("strings/{tag}.json: {e}"));
        }
        catalog
    }

    /// Adds or replaces a locale from its JSON text.
    ///
    /// # Errors
    ///
    /// When the text is not a flat JSON object whose values are strings.
    pub fn add_locale(&mut self, tag: &str, json: &str) -> Result<(), String> {
        let map: BTreeMap<String, serde_json::Value> =
            serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut strings = BTreeMap::new();
        for (key, value) in map {
            let serde_json::Value::String(s) = value else {
                return Err(format!("{key}: not a string"));
            };
            strings.insert(key, s);
        }
        self.locales.insert(tag.to_owned(), strings);
        Ok(())
    }

    /// The locale tags present.
    pub fn locales(&self) -> impl Iterator<Item = &str> {
        self.locales.keys().map(String::as_str)
    }

    /// The raw pattern for `key` in `locale`, without fallback.
    #[must_use]
    pub fn pattern(&self, locale: &str, key: &str) -> Option<&str> {
        self.locales.get(locale)?.get(key).map(String::as_str)
    }

    /// The keys of a locale.
    pub fn keys(&self, locale: &str) -> impl Iterator<Item = &str> {
        self.locales
            .get(locale)
            .into_iter()
            .flat_map(|m| m.keys().map(String::as_str))
    }

    /// The string for `key` in `locale`, falling back to `en` per key (R33.8), formatted
    /// with `args`. A pattern outside the subset renders as its key, which the contract
    /// tests make impossible for shipped locales.
    #[must_use]
    pub fn get(&self, key: StringKey, locale: &str, args: &FmtArgs) -> String {
        let pattern = self
            .pattern(locale, key.0)
            .or_else(|| self.pattern("en", key.0));
        match pattern {
            Some(p) => format(p, args).unwrap_or_else(|_| key.0.to_owned()),
            None => key.0.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Catalog, FmtArgs, format, keys, placeholders};

    #[test]
    fn the_subset_formats_arguments_plurals_and_selects() {
        assert_eq!(
            format("hello {name}", &FmtArgs::new().text("name", "Sam")).unwrap(),
            "hello Sam"
        );
        let p = "{n, plural, =0 {none} one {# task} other {# tasks}}";
        assert_eq!(format(p, &FmtArgs::new().number("n", 0)).unwrap(), "none");
        assert_eq!(format(p, &FmtArgs::new().number("n", 1)).unwrap(), "1 task");
        assert_eq!(
            format(p, &FmtArgs::new().number("n", 7)).unwrap(),
            "7 tasks"
        );
        let s = "{role, select, watcher {no, {name}} other {maybe}}";
        assert_eq!(
            format(s, &FmtArgs::new().text("role", "watcher").text("name", "x")).unwrap(),
            "no, x"
        );
        assert_eq!(
            format(s, &FmtArgs::new().text("role", "admin")).unwrap(),
            "maybe"
        );
        assert_eq!(
            placeholders(s).unwrap().into_iter().collect::<Vec<_>>(),
            ["name", "role"]
        );
        assert!(
            format("{n, number}", &FmtArgs::new()).is_err(),
            "only plural and select"
        );
        assert!(
            format("{n, plural, one {x}}", &FmtArgs::new()).is_err(),
            "other is required"
        );
        assert!(format("{unterminated", &FmtArgs::new()).is_err());
    }

    #[test]
    fn the_builtin_catalog_falls_back_per_key() {
        let mut catalog = Catalog::builtin();
        catalog
            .add_locale("xx", include_str!("../../tests/fixtures/strings/xx.json"))
            .expect("fixture");
        assert_eq!(
            catalog.get(keys::tui::status::WORKING, "xx", &FmtArgs::new()),
            "[xx] working"
        );
        assert_eq!(
            catalog.get(keys::tui::status::COMPLETED, "xx", &FmtArgs::new()),
            "completed",
            "missing in xx: en"
        );
        assert_eq!(
            catalog.get(
                keys::tui::task::SUB_TASKS,
                "en",
                &FmtArgs::new().number("n", 2)
            ),
            "2 sub-tasks"
        );
        assert_eq!(
            catalog.get(
                keys::command::UNKNOWN,
                "de",
                &FmtArgs::new()
                    .text("name", "/modle")
                    .text("suggestion", "/model")
            ),
            "Unknown command /modle. Did you mean /model?"
        );
    }
}
