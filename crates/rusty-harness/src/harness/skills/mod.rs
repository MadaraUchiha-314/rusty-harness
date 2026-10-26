//! Agent Skills: the `SKILL.md` parser, progressive disclosure, activation (R6).
//!
//! A skill is a directory with a `SKILL.md` whose YAML front matter carries `name`,
//! `description` and the optional `license`, `compatibility`, `metadata` and
//! `allowed-tools`. The crate parses that front matter itself: it is flat scalars, one
//! one-level map and block scalars, which is less code than adapting a YAML crate
//! (design.md T11). Disclosure is progressive: the index line at startup, the body on
//! activation, resources on demand.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::harness::hooks::{HookCx, HookDispatcher, HookPoint, Operation};
use crate::host::FileSystem;
use crate::registry::{Entity, EntityId, EntityKind};

/// The front matter of a `SKILL.md`, validated per the specification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMeta {
    /// 1–64 chars of `a-z`, `0-9` and single hyphens, equal to the directory name.
    pub name: String,
    /// 1–1024 chars: what the skill does and when to use it.
    pub description: String,
    /// A licence name or a bundled licence file.
    pub license: Option<String>,
    /// Environment requirements, at most 500 chars.
    pub compatibility: Option<String>,
    /// Client-defined string pairs.
    pub metadata: BTreeMap<String, String>,
    /// Pre-approved tools (experimental in the specification), split on whitespace.
    pub allowed_tools: Vec<String>,
}

/// A skill that does not conform (R6.3): the error names the field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("skill {skill}: {field}: {reason}")]
pub struct SkillError {
    /// The skill directory.
    pub skill: String,
    /// The offending field.
    pub field: String,
    /// What is wrong.
    pub reason: String,
}

fn err(skill: &str, field: &str, reason: impl Into<String>) -> SkillError {
    SkillError {
        skill: skill.to_owned(),
        field: field.to_owned(),
        reason: reason.into(),
    }
}

/// Parses `SKILL.md` texts.
#[derive(Debug, Default, Clone, Copy)]
pub struct SkillParser;

impl SkillParser {
    /// Splits a `SKILL.md` into its front matter and body.
    ///
    /// # Errors
    ///
    /// [`SkillError`] when the front matter is missing or unterminated.
    pub fn split(dir_name: &str, text: &str) -> Result<(String, String), SkillError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let rest = text
            .strip_prefix("---\n")
            .or_else(|| text.strip_prefix("---\r\n"))
            .ok_or_else(|| err(dir_name, "front matter", "missing"))?;
        let end = rest
            .find("\n---")
            .ok_or_else(|| err(dir_name, "front matter", "unterminated"))?;
        let front = rest[..end].to_owned();
        let body = rest[end + 4..].trim_start_matches(['\r', '\n']).to_owned();
        Ok((front, body))
    }

    /// Parses and validates the front matter of a skill in directory `dir_name`.
    ///
    /// # Errors
    ///
    /// [`SkillError`] naming the field that is missing or invalid.
    pub fn parse(dir_name: &str, text: &str) -> Result<SkillMeta, SkillError> {
        let (front, _) = Self::split(dir_name, text)?;
        let fields = parse_front_matter(dir_name, &front)?;
        let scalar = |key: &str| -> Option<String> {
            match fields.get(key) {
                Some(Value::Scalar(s)) => Some(s.clone()),
                _ => None,
            }
        };
        let name = scalar("name").ok_or_else(|| err(dir_name, "name", "missing"))?;
        validate_name(dir_name, &name)?;
        let description =
            scalar("description").ok_or_else(|| err(dir_name, "description", "missing"))?;
        if description.is_empty() || description.chars().count() > 1024 {
            return Err(err(dir_name, "description", "must be 1–1024 characters"));
        }
        let compatibility = scalar("compatibility");
        if let Some(c) = &compatibility
            && (c.is_empty() || c.chars().count() > 500)
        {
            return Err(err(dir_name, "compatibility", "must be 1–500 characters"));
        }
        let metadata = match fields.get("metadata") {
            Some(Value::Map(m)) => m.clone(),
            Some(Value::Scalar(_)) => {
                return Err(err(dir_name, "metadata", "must be a map of strings"));
            }
            None => BTreeMap::new(),
        };
        Ok(SkillMeta {
            name,
            description,
            license: scalar("license"),
            compatibility,
            metadata,
            allowed_tools: scalar("allowed-tools")
                .map(|s| s.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
        })
    }
}

fn validate_name(dir_name: &str, name: &str) -> Result<(), SkillError> {
    let ok_chars = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if name.is_empty()
        || name.len() > 64
        || !ok_chars
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
    {
        return Err(err(
            dir_name,
            "name",
            "must be 1–64 lowercase letters, digits and single hyphens",
        ));
    }
    if name != dir_name {
        return Err(err(
            dir_name,
            "name",
            format!("{name:?} does not match the directory name"),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Scalar(String),
    Map(BTreeMap<String, String>),
}

fn unquote(raw: &str) -> String {
    let s = raw.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].to_owned()
    } else {
        s.to_owned()
    }
}

/// The YAML subset of a skill's front matter: `key: scalar`, `key: |` / `key: >` block
/// scalars, and `key:` followed by an indented map of scalars.
fn parse_front_matter(dir_name: &str, front: &str) -> Result<BTreeMap<String, Value>, SkillError> {
    let lines: Vec<&str> = front.lines().collect();
    let mut fields = BTreeMap::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with(' ') {
            return Err(err(
                dir_name,
                "front matter",
                format!("unexpected indentation at {line:?}"),
            ));
        }
        let (key, rest) = line.split_once(':').ok_or_else(|| {
            err(
                dir_name,
                "front matter",
                format!("line without a key: {line:?}"),
            )
        })?;
        let key = key.trim().to_owned();
        let rest = rest.trim();
        let value = if rest == "|" || rest == ">" {
            let mut block = Vec::new();
            while i < lines.len() && (lines[i].starts_with(' ') || lines[i].trim().is_empty()) {
                block.push(lines[i].trim_start());
                i += 1;
            }
            let text = if rest == "|" {
                block.join("\n")
            } else {
                block.join(" ")
            };
            Value::Scalar(text.trim().to_owned())
        } else if rest.is_empty() {
            let mut map = BTreeMap::new();
            while i < lines.len() && lines[i].starts_with(' ') {
                let (k, v) = lines[i]
                    .trim()
                    .split_once(':')
                    .ok_or_else(|| err(dir_name, &key, format!("bad map entry {:?}", lines[i])))?;
                map.insert(k.trim().to_owned(), unquote(v));
                i += 1;
            }
            Value::Map(map)
        } else {
            Value::Scalar(unquote(rest))
        };
        fields.insert(key, value);
    }
    Ok(fields)
}

/// Where a skill's files live.
#[derive(Clone, Debug)]
pub enum SkillRoot {
    /// A directory, read through the host's file system.
    Dir(PathBuf),
    /// In memory (a programmatic plugin): paths relative to the skill root.
    InMemory(BTreeMap<PathBuf, Bytes>),
}

/// A registered skill with its three disclosure levels.
pub struct Skill {
    id: EntityId,
    meta: SkillMeta,
    root: SkillRoot,
    body: Mutex<Option<String>>,
}

impl core::fmt::Debug for Skill {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Skill")
            .field("id", &self.id)
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl Skill {
    /// A skill from its parsed metadata and root; `version` is the plugin's, when any.
    #[must_use]
    pub fn new(meta: SkillMeta, root: SkillRoot, version: Option<semver::Version>) -> Self {
        let id = match version {
            Some(v) => EntityId::versioned(meta.name.clone(), v),
            None => EntityId::new(meta.name.clone()),
        };
        Self {
            id,
            meta,
            root,
            body: Mutex::new(None),
        }
    }

    /// Level 1: the metadata, loaded at startup.
    #[must_use]
    pub fn meta(&self) -> &SkillMeta {
        &self.meta
    }

    /// Level 1 as one line for the skills index position of the context.
    #[must_use]
    pub fn index_line(&self) -> String {
        format!("{}: {}", self.meta.name, self.meta.description)
    }

    /// Where the files live.
    #[must_use]
    pub fn root(&self) -> &SkillRoot {
        &self.root
    }

    /// Whether the body has been loaded yet.
    #[must_use]
    pub fn body_loaded(&self) -> bool {
        self.body
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// Level 2: the instructions, read on activation and kept.
    ///
    /// # Errors
    ///
    /// [`SkillError`] when `SKILL.md` cannot be read or no longer parses.
    pub async fn body(&self, fs: &dyn FileSystem) -> Result<String, SkillError> {
        if let Some(body) = self
            .body
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Ok(body);
        }
        let text = self.read(Path::new("SKILL.md"), fs).await?;
        let text = String::from_utf8(text.to_vec())
            .map_err(|_| err(&self.meta.name, "SKILL.md", "not UTF-8"))?;
        let (_, body) = SkillParser::split(&self.meta.name, &text)?;
        *self
            .body
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(body.clone());
        Ok(body)
    }

    /// Level 3: a file under the skill root (`scripts/…`, `references/…`, `assets/…`),
    /// read on demand. The path may not leave the root.
    ///
    /// # Errors
    ///
    /// [`SkillError`] for a path outside the root or one that cannot be read.
    pub async fn resource(&self, rel: &Path, fs: &dyn FileSystem) -> Result<Bytes, SkillError> {
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(err(
                &self.meta.name,
                "resource",
                format!("{} leaves the skill directory", rel.display()),
            ));
        }
        self.read(rel, fs).await
    }

    async fn read(&self, rel: &Path, fs: &dyn FileSystem) -> Result<Bytes, SkillError> {
        match &self.root {
            SkillRoot::Dir(dir) => fs
                .read(&dir.join(rel))
                .await
                .map_err(|e| err(&self.meta.name, &rel.display().to_string(), e.to_string())),
            SkillRoot::InMemory(files) => files
                .get(rel)
                .cloned()
                .ok_or_else(|| err(&self.meta.name, &rel.display().to_string(), "not found")),
        }
    }
}

impl Entity for Skill {
    fn kind(&self) -> EntityKind {
        EntityKind::Skill
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

/// The `skill.invoke` operation: activating a skill loads its body (R6.4).
#[derive(Debug)]
pub struct SkillInvoke;

impl Operation for SkillInvoke {
    const POINT: HookPoint = HookPoint::SkillInvoke;
    type Input = EntityId;
    type Output = String;
    fn identity(input: &EntityId) -> String {
        input.to_string()
    }
}

/// Activates `skill` through the `skill.invoke` hooks and returns its body.
///
/// # Errors
///
/// A veto or hook failure, or [`SkillError`] wrapped as an internal error.
pub async fn activate(
    skill: Arc<Skill>,
    dispatcher: &HookDispatcher,
    cx: &HookCx,
    fs: Arc<dyn FileSystem>,
) -> Result<String, crate::HarnessError> {
    let id = skill.entity_id().clone();
    dispatcher
        .run::<SkillInvoke, _, _>(cx, id, move |_| async move {
            skill
                .body(fs.as_ref())
                .await
                .map_err(|e| crate::HarnessError::Internal(e.to_string()))
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::{SkillMeta, SkillParser};
    use std::collections::BTreeMap;

    const FULL: &str = "---\nname: pdf-processing\ndescription: >\n  Extract PDF text, fill forms, merge files.\n  Use when handling PDFs.\nlicense: Apache-2.0\ncompatibility: \"Requires Python 3.14+\"\nmetadata:\n  author: example-org\n  version: \"1.0\"\nallowed-tools: Bash(git:*) Read\n---\n# Body\nDo the thing.\n";

    #[test]
    fn the_front_matter_subset_parses_every_specified_field() {
        let meta = SkillParser::parse("pdf-processing", FULL).expect("parses");
        assert_eq!(
            meta,
            SkillMeta {
                name: "pdf-processing".into(),
                description: "Extract PDF text, fill forms, merge files. Use when handling PDFs."
                    .into(),
                license: Some("Apache-2.0".into()),
                compatibility: Some("Requires Python 3.14+".into()),
                metadata: BTreeMap::from([
                    ("author".to_owned(), "example-org".to_owned()),
                    ("version".to_owned(), "1.0".to_owned())
                ]),
                allowed_tools: vec!["Bash(git:*)".into(), "Read".into()],
            }
        );
        let (_, body) = SkillParser::split("pdf-processing", FULL).unwrap();
        assert_eq!(body, "# Body\nDo the thing.\n");
    }

    #[test]
    fn invalid_skills_are_refused_naming_the_field() {
        let minimal = |name: &str| format!("---\nname: {name}\ndescription: x\n---\n");
        assert_eq!(
            SkillParser::parse("other", &minimal("pdf"))
                .unwrap_err()
                .field,
            "name"
        );
        for bad in ["PDF", "-pdf", "pdf-", "pdf--x", ""] {
            assert_eq!(
                SkillParser::parse(bad, &minimal(bad)).unwrap_err().field,
                "name",
                "{bad:?}"
            );
        }
        assert_eq!(
            SkillParser::parse("x", "---\nname: x\n---\n")
                .unwrap_err()
                .field,
            "description"
        );
        assert_eq!(
            SkillParser::parse(
                "x",
                &format!("---\nname: x\ndescription: {}\n---\n", "d".repeat(1025))
            )
            .unwrap_err()
            .field,
            "description"
        );
        assert_eq!(
            SkillParser::parse("x", "no front matter")
                .unwrap_err()
                .field,
            "front matter"
        );
        assert_eq!(
            SkillParser::parse("x", "---\nname: x\ndescription: d\nmetadata: oops\n---\n")
                .unwrap_err()
                .field,
            "metadata"
        );
    }
}
