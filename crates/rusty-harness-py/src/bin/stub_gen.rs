//! Regenerates `rusty_harness.pyi` from the `#[gen_stub_*]` annotations, then appends the
//! `TypedDict`s: the configuration's, generated from its JSON schema (Q9), and the A2A
//! JSON shapes requests and events cross as.

use std::fmt::Write as _;

use serde_json::Value;

const STUB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/rusty_harness.pyi");

/// The A2A 1.0 JSON binding, as `dict`s (the same shapes as the npm package's `a2a.d.ts`).
const A2A: &str = r#"
# --- The A2A 1.0 JSON binding requests and events cross as.

Role = typing.Literal["ROLE_USER", "ROLE_AGENT", "ROLE_UNSPECIFIED"]
TaskState = typing.Literal[
    "TASK_STATE_UNSPECIFIED",
    "TASK_STATE_SUBMITTED",
    "TASK_STATE_WORKING",
    "TASK_STATE_COMPLETED",
    "TASK_STATE_FAILED",
    "TASK_STATE_CANCELED",
    "TASK_STATE_INPUT_REQUIRED",
    "TASK_STATE_REJECTED",
    "TASK_STATE_AUTH_REQUIRED",
]

class _PartCommon(typing.TypedDict, total=False):
    filename: str
    mediaType: str
    metadata: dict[str, typing.Any]

class TextPart(_PartCommon):
    text: str

class RawPart(_PartCommon):
    raw: str
    r"""Base64."""

class UrlPart(_PartCommon):
    url: str

class DataPart(_PartCommon):
    data: typing.Any

Part = TextPart | RawPart | UrlPart | DataPart

class Message(typing.TypedDict):
    messageId: str
    role: Role
    parts: list[Part]
    contextId: typing_extensions.NotRequired[str]
    taskId: typing_extensions.NotRequired[str]
    metadata: typing_extensions.NotRequired[dict[str, typing.Any]]
    extensions: typing_extensions.NotRequired[list[str]]
    referenceTaskIds: typing_extensions.NotRequired[list[str]]

class TaskStatus(typing.TypedDict):
    state: TaskState
    message: typing_extensions.NotRequired[Message]
    timestamp: typing_extensions.NotRequired[str]

class Artifact(typing.TypedDict):
    artifactId: str
    parts: list[Part]
    name: typing_extensions.NotRequired[str]
    description: typing_extensions.NotRequired[str]
    metadata: typing_extensions.NotRequired[dict[str, typing.Any]]
    extensions: typing_extensions.NotRequired[list[str]]

class Task(typing.TypedDict):
    id: str
    contextId: str
    status: TaskStatus
    artifacts: typing_extensions.NotRequired[list[Artifact]]
    history: typing_extensions.NotRequired[list[Message]]
    metadata: typing_extensions.NotRequired[dict[str, typing.Any]]

class TaskStatusUpdateEvent(typing.TypedDict):
    taskId: str
    contextId: str
    status: TaskStatus
    metadata: typing_extensions.NotRequired[dict[str, typing.Any]]

class TaskArtifactUpdateEvent(typing.TypedDict):
    taskId: str
    contextId: str
    artifact: Artifact
    append: typing_extensions.NotRequired[bool]
    lastChunk: typing_extensions.NotRequired[bool]
    metadata: typing_extensions.NotRequired[dict[str, typing.Any]]

class TaskEvent(typing.TypedDict):
    task: Task

class MessageEvent(typing.TypedDict):
    message: Message

class StatusUpdateEvent(typing.TypedDict):
    statusUpdate: TaskStatusUpdateEvent

class ArtifactUpdateEvent(typing.TypedDict):
    artifactUpdate: TaskArtifactUpdateEvent

StreamResponse = TaskEvent | MessageEvent | StatusUpdateEvent | ArtifactUpdateEvent

class SendMessageConfiguration(typing.TypedDict, total=False):
    acceptedOutputModes: list[str]
    historyLength: int
    returnImmediately: bool

class SendMessageRequest(typing.TypedDict):
    message: Message
    configuration: typing_extensions.NotRequired[SendMessageConfiguration]
    metadata: typing_extensions.NotRequired[dict[str, typing.Any]]
    tenant: typing_extensions.NotRequired[str]
"#;

fn main() -> pyo3_stub_gen::Result<()> {
    rusty_harness_py::stub_info()?.generate()?;
    let schema =
        serde_json::to_value(rusty_harness::config::Config::schema()).expect("the schema is JSON");
    let mut stub = std::fs::read_to_string(STUB)?;
    stub = stub.replacen(
        "import typing\n",
        "import typing\nimport typing_extensions\n",
        1,
    );
    stub.push_str(A2A);
    stub.push_str(&typed_dicts(&schema));
    std::fs::write(STUB, stub)?;
    Ok(())
}

/// The configuration's `TypedDict`s: one per object in `$defs`, `ConfigDict` for the root.
fn typed_dicts(schema: &Value) -> String {
    let mut out = String::from(
        "\n# --- The configuration as dicts, generated from rusty-harness.schema.json.\n",
    );
    let empty = serde_json::Map::new();
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    for (name, def) in defs {
        definition(&mut out, name, def);
    }
    definition(&mut out, "ConfigDict", schema);
    out
}

fn definition(out: &mut String, name: &str, def: &Value) {
    if def.get("properties").is_some() {
        class(out, name, def);
        return;
    }
    let variants = def
        .get("oneOf")
        .or_else(|| def.get("anyOf"))
        .and_then(Value::as_array);
    if let Some(variants) = variants
        && variants.iter().any(|v| v.get("properties").is_some())
    {
        let mut names = Vec::new();
        for (i, variant) in variants.iter().enumerate() {
            let tag = variant
                .pointer("/properties/scheme/const")
                .or_else(|| variant.pointer("/properties/type/const"))
                .and_then(Value::as_str)
                .map_or_else(|| i.to_string(), pascal);
            let variant_name = format!("{name}{tag}");
            class(out, &variant_name, variant);
            names.push(variant_name);
        }
        let _ = writeln!(out, "\n{name} = {}", names.join(" | "));
        return;
    }
    let _ = writeln!(out, "\n{name} = {}", type_of(def));
}

fn class(out: &mut String, name: &str, def: &Value) {
    let _ = writeln!(out, "\nclass {name}(typing.TypedDict):");
    if let Some(doc) = def.get("description").and_then(Value::as_str) {
        let _ = writeln!(out, "    r\"\"\"{}\"\"\"", doc.replace("\"\"\"", "'''"));
    }
    let required: Vec<&str> = def
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(properties) = def.get("properties").and_then(Value::as_object) else {
        out.push_str("    pass\n");
        return;
    };
    for (field, schema) in properties {
        let ty = type_of(schema);
        if required.contains(&field.as_str()) {
            let _ = writeln!(out, "    {field}: {ty}");
        } else {
            let _ = writeln!(out, "    {field}: typing_extensions.NotRequired[{ty}]");
        }
        if let Some(doc) = schema.get("description").and_then(Value::as_str) {
            let _ = writeln!(out, "    r\"\"\"{}\"\"\"", doc.replace("\"\"\"", "'''"));
        }
    }
}

fn type_of(schema: &Value) -> String {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.rsplit('/').next().unwrap_or("typing.Any");
        return if name == "Config" {
            "ConfigDict".into()
        } else {
            name.into()
        };
    }
    if let Some(value) = schema.get("const") {
        return format!("typing.Literal[{value}]");
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let literals: Vec<String> = values.iter().map(Value::to_string).collect();
        return format!("typing.Literal[{}]", literals.join(", "));
    }
    if let Some(variants) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
    {
        let mut types: Vec<String> = variants.iter().map(type_of).collect();
        types.dedup();
        return types.join(" | ");
    }
    match schema.get("type") {
        Some(Value::Array(types)) => {
            let types: Vec<String> = types
                .iter()
                .map(|t| {
                    let mut one = schema.clone();
                    one["type"] = t.clone();
                    type_of(&one)
                })
                .collect();
            types.join(" | ")
        }
        Some(Value::String(t)) => match t.as_str() {
            "string" => "builtins.str".into(),
            "integer" => "builtins.int".into(),
            "number" => "builtins.float".into(),
            "boolean" => "builtins.bool".into(),
            "null" => "None".into(),
            "array" => format!(
                "builtins.list[{}]",
                schema
                    .get("items")
                    .map_or_else(|| "typing.Any".into(), type_of)
            ),
            "object" => match schema.get("additionalProperties") {
                Some(Value::Bool(_)) | None => "builtins.dict[builtins.str, typing.Any]".into(),
                Some(values) => format!("builtins.dict[builtins.str, {}]", type_of(values)),
            },
            _ => "typing.Any".into(),
        },
        _ => "typing.Any".into(),
    }
}

fn pascal(text: &str) -> String {
    text.split(['-', '_'])
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}
