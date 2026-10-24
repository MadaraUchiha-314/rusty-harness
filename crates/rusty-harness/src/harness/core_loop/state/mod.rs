//! `AgentState` with schema-checked keys (R14.1).
//!
//! Values are JSON under named keys. A key may carry a JSON schema; a write to such a key
//! is validated and a violation is a typed [`StateError::SchemaViolation`]. The validator
//! covers the subset agents declare in practice (`type`, `enum`, `const`, `properties`,
//! `required`, `additionalProperties`, `items`, `minimum`/`maximum`,
//! `minLength`/`maxLength`, `minItems`/`maxItems`); anything else in a schema is
//! accepted unchecked, and the schema itself may be produced with `schemars`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::harness::core_loop::task::TaskId;
use crate::harness::hooks::{HookPoint, Operation};

/// A state key.
pub type StateKey = String;

/// A typed state error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// The value does not satisfy the key's schema (R14.1).
    #[error("state key {key}: schema violation at {path}: {reason}")]
    SchemaViolation {
        /// The key.
        key: StateKey,
        /// A JSON pointer to the offending value.
        path: String,
        /// What was violated.
        reason: String,
    },
    /// The value could not be (de)serialized.
    #[error("state key {key}: {reason}")]
    Codec {
        /// The key.
        key: StateKey,
        /// The serde error.
        reason: String,
    },
}

/// One write, for hooks and the trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDelta {
    /// The key.
    pub key: StateKey,
    /// The value before, if any.
    pub before: Option<Value>,
    /// The value after.
    pub after: Value,
}

/// The agent's state (R14.1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentState {
    entries: BTreeMap<StateKey, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    schemas: BTreeMap<StateKey, Value>,
}

impl AgentState {
    /// Empty state with no schemas.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares the schema `key` must satisfy from now on. An existing value is not
    /// re-checked.
    pub fn declare(&mut self, key: impl Into<String>, schema: Value) {
        self.schemas.insert(key.into(), schema);
    }

    /// The schema for `key`, if declared.
    #[must_use]
    pub fn schema(&self, key: &str) -> Option<&Value> {
        self.schemas.get(key)
    }

    /// Writes `value` under `key`, validating it against the key's schema when one is
    /// declared.
    ///
    /// # Errors
    ///
    /// [`StateError::SchemaViolation`] (the state is unchanged) or [`StateError::Codec`].
    pub fn set<T: Serialize>(
        &mut self,
        key: impl Into<String>,
        value: T,
    ) -> Result<StateDelta, StateError> {
        let key = key.into();
        let after = serde_json::to_value(value).map_err(|e| StateError::Codec {
            key: key.clone(),
            reason: e.to_string(),
        })?;
        if let Some(schema) = self.schemas.get(&key) {
            let mut path = String::new();
            validate(schema, &after, &mut path).map_err(|reason| StateError::SchemaViolation {
                key: key.clone(),
                path: if path.is_empty() { "/".into() } else { path },
                reason,
            })?;
        }
        let before = self.entries.insert(key.clone(), after.clone());
        Ok(StateDelta { key, before, after })
    }

    /// Reads `key` as `T`; `None` when unset.
    ///
    /// # Errors
    ///
    /// [`StateError::Codec`] when the stored value is not a `T`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, StateError> {
        self.entries
            .get(key)
            .map(|v| {
                serde_json::from_value(v.clone()).map_err(|e| StateError::Codec {
                    key: key.to_owned(),
                    reason: e.to_string(),
                })
            })
            .transpose()
    }

    /// Removes `key`, returning the old value.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.entries.remove(key)
    }

    /// The keys, sorted.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The raw entries, for the context window's `StateExcerpt` position.
    #[must_use]
    pub fn entries(&self) -> &BTreeMap<StateKey, Value> {
        &self.entries
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_matches(expected: &str, v: &Value) -> bool {
    match expected {
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        other => type_name(v) == other,
    }
}

/// Validates `value` against the supported subset of `schema`; `path` is the JSON pointer
/// of the offending value on error.
fn validate(schema: &Value, value: &Value, path: &mut String) -> Result<(), String> {
    let Some(schema) = schema.as_object() else {
        return Ok(()); // `true`/unknown: accept
    };
    if let Some(t) = schema.get("type") {
        let ok = match t {
            Value::String(s) => type_matches(s, value),
            Value::Array(ts) => ts
                .iter()
                .any(|t| t.as_str().is_some_and(|s| type_matches(s, value))),
            _ => true,
        };
        if !ok {
            return Err(format!("expected type {t}, found {}", type_name(value)));
        }
    }
    if let Some(Value::Array(allowed)) = schema.get("enum")
        && !allowed.contains(value)
    {
        return Err(format!(
            "{value} is not one of {}",
            Value::Array(allowed.clone())
        ));
    }
    if let Some(c) = schema.get("const")
        && c != value
    {
        return Err(format!("expected {c}"));
    }
    if let Some(n) = value.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(Value::as_f64)
            && n < min
        {
            return Err(format!("{n} < minimum {min}"));
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_f64)
            && n > max
        {
            return Err(format!("{n} > maximum {max}"));
        }
    }
    if let Some(s) = value.as_str() {
        let len = s.chars().count();
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64)
            && (len as u64) < min
        {
            return Err(format!("length {len} < minLength {min}"));
        }
        if let Some(max) = schema.get("maxLength").and_then(Value::as_u64)
            && (len as u64) > max
        {
            return Err(format!("length {len} > maxLength {max}"));
        }
    }
    if let Some(items) = value.as_array() {
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64)
            && (items.len() as u64) < min
        {
            return Err(format!("{} items < minItems {min}", items.len()));
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64)
            && (items.len() as u64) > max
        {
            return Err(format!("{} items > maxItems {max}", items.len()));
        }
        if let Some(item_schema) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                let mark = path.len();
                let _ = write!(path, "/{i}");
                validate(item_schema, item, path)?;
                path.truncate(mark);
            }
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(Value::Array(required)) = schema.get("required") {
            for r in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(r) {
                    return Err(format!("missing required property {r}"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (k, v) in object {
            let mark = path.len();
            let _ = write!(path, "/{k}");
            match properties.and_then(|p| p.get(k)) {
                Some(s) => validate(s, v, path)?,
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => return Err(format!("unexpected property {k}")),
                    Some(s @ Value::Object(_)) => validate(s, v, path)?,
                    _ => {}
                },
            }
            path.truncate(mark);
        }
    }
    Ok(())
}

/// The input of `state.persist`: whose state, and the state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateWrite {
    /// The task.
    pub task: TaskId,
    /// The whole state after the write.
    pub state: AgentState,
    /// The write that triggered it, when a single key changed.
    pub delta: Option<StateDelta>,
}

/// The `state.persist` operation (R14.2): a security-relevant point, its policy locked.
#[derive(Debug)]
pub struct StatePersist;

impl Operation for StatePersist {
    const POINT: HookPoint = HookPoint::StatePersist;
    type Input = StateWrite;
    type Output = ();
    fn identity(input: &StateWrite) -> String {
        input.task.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentState, StateError};
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Prefs {
        theme: String,
        retries: u32,
    }

    #[test]
    fn typed_values_round_trip_under_named_keys() {
        let mut state = AgentState::new();
        let delta = state
            .set(
                "prefs",
                Prefs {
                    theme: "dark".into(),
                    retries: 2,
                },
            )
            .unwrap();
        assert_eq!(delta.before, None);
        assert_eq!(
            state.get::<Prefs>("prefs").unwrap(),
            Some(Prefs {
                theme: "dark".into(),
                retries: 2
            })
        );
        assert_eq!(state.get::<Prefs>("nothing").unwrap(), None);
        assert!(matches!(
            state.get::<u32>("prefs"),
            Err(StateError::Codec { .. })
        ));
        let delta = state
            .set(
                "prefs",
                Prefs {
                    theme: "light".into(),
                    retries: 2,
                },
            )
            .unwrap();
        assert_eq!(delta.before.unwrap()["theme"], "dark");
        assert_eq!(state.keys().collect::<Vec<_>>(), ["prefs"]);
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<AgentState>(&json).unwrap(), state);
    }

    #[test]
    fn a_schema_violation_is_a_typed_error_and_the_state_is_unchanged() {
        let mut state = AgentState::new();
        state.declare(
            "prefs",
            json!({
                "type": "object",
                "required": ["theme"],
                "additionalProperties": false,
                "properties": {
                    "theme": { "type": "string", "enum": ["dark", "light"] },
                    "retries": { "type": "integer", "minimum": 0, "maximum": 5 },
                    "tags": { "type": "array", "maxItems": 2, "items": { "type": "string", "minLength": 1 } }
                }
            }),
        );
        state
            .set(
                "prefs",
                json!({ "theme": "dark", "retries": 3, "tags": ["a"] }),
            )
            .unwrap();
        let cases = [
            (json!({ "theme": "blue" }), "/theme", "not one of"),
            (
                json!({ "retries": 1 }),
                "/",
                "missing required property theme",
            ),
            (
                json!({ "theme": "dark", "retries": 9 }),
                "/retries",
                "> maximum",
            ),
            (
                json!({ "theme": "dark", "retries": 1.5 }),
                "/retries",
                "expected type",
            ),
            (
                json!({ "theme": "dark", "extra": 1 }),
                "/extra",
                "unexpected property",
            ),
            (
                json!({ "theme": "dark", "tags": ["a", "b", "c"] }),
                "/tags",
                "maxItems",
            ),
            (
                json!({ "theme": "dark", "tags": ["a", ""] }),
                "/tags/1",
                "minLength",
            ),
            (json!("dark"), "/", "expected type"),
        ];
        for (value, path, reason) in cases {
            let err = state
                .set("prefs", value.clone())
                .expect_err(&value.to_string());
            match err {
                StateError::SchemaViolation {
                    key,
                    path: p,
                    reason: r,
                } => {
                    assert_eq!(key, "prefs");
                    assert_eq!(p, path, "{value}");
                    assert!(r.contains(reason), "{value}: {r}");
                }
                other @ StateError::Codec { .. } => panic!("{other}"),
            }
        }
        assert_eq!(
            state.get::<serde_json::Value>("prefs").unwrap().unwrap()["retries"],
            3,
            "unchanged"
        );
        state.set("free", json!({ "anything": [1, 2, 3] })).unwrap();
    }
}
