//! A2UI 0.9.1 message and basic-catalog component types carried as A2A data parts (R25.4).
//!
//! The shapes follow the published specification (`specification/v0_9_1` in
//! <https://github.com/google/A2UI>): four server-to-client messages in a versioned
//! envelope ([`A2uiMessage`]), the basic catalog's eighteen components ([`Component`]
//! with a [`ComponentKind`] per catalog entry), the client's `action` and `error`
//! messages ([`ClientMessage`]), and the capability objects both sides exchange in A2A
//! message metadata ([`ClientCapabilities`], [`ServerCapabilities`]). On the wire an
//! A2UI payload is a `DataPart` whose `data` is the list of messages and whose metadata
//! names the media type [`MEDIA_TYPE`] ([`part`], [`parse_part`]).
//!
//! Only the basic catalog ([`BASIC_CATALOG`]) is produced here; [`text_alternative`]
//! derives the plain-text rendering a surface without A2UI support receives instead
//! (R25.2).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocol version written on every message.
pub const VERSION: &str = "v0.9.1";
/// The versions a receiver accepts (`v0.9` payloads are shape-compatible).
pub const ACCEPTED_VERSIONS: [&str; 2] = ["v0.9", "v0.9.1"];
/// The A2A extension URI carrying A2UI.
pub const EXTENSION_URI: &str = "https://a2ui.org/a2a-extension/a2ui/v0.9.1";
/// The media type of an A2UI data part (also its `metadata.mimeType`).
pub const MEDIA_TYPE: &str = "application/a2ui+json";
/// The basic catalog's id, the one catalog the harness generates.
pub const BASIC_CATALOG: &str = "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json";
/// The A2A message metadata key of the client's capabilities.
pub const CLIENT_CAPABILITIES_KEY: &str = "a2uiClientCapabilities";
/// The A2A message metadata key of the client's synced data model.
pub const CLIENT_DATA_MODEL_KEY: &str = "a2uiClientDataModel";
/// The part metadata key naming the media type (the extension's own convention).
pub const MIME_TYPE_KEY: &str = "mimeType";
/// The part metadata key on a text part that stands in for a part a surface cannot
/// render: its value is the media type it replaces.
pub const ALTERNATIVE_FOR_KEY: &str = "alternativeFor";

/// A component id, unique within a surface; `root` is the tree's root.
pub type ComponentId = String;

// ---------------------------------------------------------------------------
// Server → client
// ---------------------------------------------------------------------------

/// One server-to-client message: the envelope's `version` is [`VERSION`] when written
/// and any of [`ACCEPTED_VERSIONS`] when read (design.md § Surfaces and renderers).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(into = "Wire<ServerBody>", try_from = "Wire<ServerBody>")]
pub enum A2uiMessage {
    /// Create a surface and start rendering it.
    CreateSurface(CreateSurface),
    /// Add or replace components of a surface; one of them has the id `root`.
    UpdateComponents(UpdateComponents),
    /// Insert into or replace (a path of) a surface's data model.
    UpdateDataModel(UpdateDataModel),
    /// Remove a surface.
    DeleteSurface(DeleteSurface),
}

impl A2uiMessage {
    /// The surface the message addresses.
    #[must_use]
    pub fn surface_id(&self) -> &str {
        match self {
            Self::CreateSurface(m) => &m.surface_id,
            Self::UpdateComponents(m) => &m.surface_id,
            Self::UpdateDataModel(m) => &m.surface_id,
            Self::DeleteSurface(m) => &m.surface_id,
        }
    }
}

/// `createSurface`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSurface {
    /// The surface.
    pub surface_id: String,
    /// The catalog every component of the surface comes from.
    pub catalog_id: String,
    /// Theme parameters, validated against the catalog's theme schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<BTreeMap<String, Value>>,
    /// Whether the client sends the surface's data model on every message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_data_model: Option<bool>,
}

/// `updateComponents`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateComponents {
    /// The surface.
    pub surface_id: String,
    /// The components, by id; the tree starts at `root`.
    pub components: Vec<Component>,
}

/// `updateDataModel`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDataModel {
    /// The surface.
    pub surface_id: String,
    /// A JSON Pointer into the model; absent or `/` is the whole model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The value put at `path`; absent removes the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// `deleteSurface`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSurface {
    /// The surface.
    pub surface_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ServerBody {
    CreateSurface(CreateSurface),
    UpdateComponents(UpdateComponents),
    UpdateDataModel(UpdateDataModel),
    DeleteSurface(DeleteSurface),
}

/// The envelope: `{ "version": "v0.9.1", "<kind>": { … } }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Wire<B> {
    version: String,
    #[serde(flatten)]
    body: B,
}

/// A message whose `version` the harness does not speak.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unsupported A2UI version {0:?}; accepted: v0.9, v0.9.1")]
pub struct UnsupportedVersion(pub String);

fn check_version(version: &str) -> Result<(), UnsupportedVersion> {
    if ACCEPTED_VERSIONS.contains(&version) {
        Ok(())
    } else {
        Err(UnsupportedVersion(version.to_owned()))
    }
}

impl From<A2uiMessage> for Wire<ServerBody> {
    fn from(message: A2uiMessage) -> Self {
        let body = match message {
            A2uiMessage::CreateSurface(m) => ServerBody::CreateSurface(m),
            A2uiMessage::UpdateComponents(m) => ServerBody::UpdateComponents(m),
            A2uiMessage::UpdateDataModel(m) => ServerBody::UpdateDataModel(m),
            A2uiMessage::DeleteSurface(m) => ServerBody::DeleteSurface(m),
        };
        Wire {
            version: VERSION.to_owned(),
            body,
        }
    }
}

impl TryFrom<Wire<ServerBody>> for A2uiMessage {
    type Error = UnsupportedVersion;
    fn try_from(wire: Wire<ServerBody>) -> Result<Self, UnsupportedVersion> {
        check_version(&wire.version)?;
        Ok(match wire.body {
            ServerBody::CreateSurface(m) => Self::CreateSurface(m),
            ServerBody::UpdateComponents(m) => Self::UpdateComponents(m),
            ServerBody::UpdateDataModel(m) => Self::UpdateDataModel(m),
            ServerBody::DeleteSurface(m) => Self::DeleteSurface(m),
        })
    }
}

// ---------------------------------------------------------------------------
// Client → server
// ---------------------------------------------------------------------------

/// One client-to-server message, in the same versioned envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(into = "Wire<ClientBody>", try_from = "Wire<ClientBody>")]
pub enum ClientMessage {
    /// A user-initiated action from a component (a button's `event`).
    Action(UserAction),
    /// A client-side error, validation or otherwise.
    Error(ClientError),
}

/// `action`: the component's `action.event`, its context bindings resolved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAction {
    /// The `action.event.name` of the component.
    pub name: String,
    /// The surface it happened on.
    pub surface_id: String,
    /// The component that triggered it.
    pub source_component_id: String,
    /// When, as ISO 8601.
    pub timestamp: String,
    /// The `action.event.context`, every binding resolved against the data model.
    #[serde(default)]
    pub context: BTreeMap<String, Value>,
}

/// `error`: `VALIDATION_FAILED` carries the failing field's `path`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientError {
    /// The code; `VALIDATION_FAILED` for a schema violation.
    pub code: String,
    /// The surface.
    pub surface_id: String,
    /// One or two sentences.
    pub message: String,
    /// The JSON Pointer of the failing field, on validation errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ClientBody {
    Action(UserAction),
    Error(ClientError),
}

impl From<ClientMessage> for Wire<ClientBody> {
    fn from(message: ClientMessage) -> Self {
        let body = match message {
            ClientMessage::Action(a) => ClientBody::Action(a),
            ClientMessage::Error(e) => ClientBody::Error(e),
        };
        Wire {
            version: VERSION.to_owned(),
            body,
        }
    }
}

impl TryFrom<Wire<ClientBody>> for ClientMessage {
    type Error = UnsupportedVersion;
    fn try_from(wire: Wire<ClientBody>) -> Result<Self, UnsupportedVersion> {
        check_version(&wire.version)?;
        Ok(match wire.body {
            ClientBody::Action(a) => Self::Action(a),
            ClientBody::Error(e) => Self::Error(e),
        })
    }
}

// ---------------------------------------------------------------------------
// Capabilities (A2A message metadata and the agent card)
// ---------------------------------------------------------------------------

/// `a2uiClientCapabilities`: what a surface renders, sent in A2A message metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    /// The catalogs the renderer supports.
    pub supported_catalog_ids: Vec<String>,
    /// Catalogs defined inline; only for agents that accept them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_catalogs: Vec<Value>,
}

impl ClientCapabilities {
    /// A renderer of the basic catalog.
    #[must_use]
    pub fn basic() -> Self {
        Self {
            supported_catalog_ids: vec![BASIC_CATALOG.to_owned()],
            inline_catalogs: Vec::new(),
        }
    }

    /// Whether `catalog` is supported.
    #[must_use]
    pub fn supports(&self, catalog: &str) -> bool {
        self.supported_catalog_ids.iter().any(|c| c == catalog)
    }

    /// The capabilities in `metadata`, under the `v0.9.1` key (or `v0.9`).
    #[must_use]
    pub fn from_metadata(metadata: Option<&HashMap<String, Value>>) -> Option<Self> {
        let versions = metadata?.get(CLIENT_CAPABILITIES_KEY)?.as_object()?;
        let value = versions.get(VERSION).or_else(|| versions.get("v0.9"))?;
        serde_json::from_value(value.clone()).ok()
    }

    /// The metadata value: `{ "v0.9.1": { "supportedCatalogIds": [...] } }`.
    #[must_use]
    pub fn to_metadata(&self) -> Value {
        serde_json::json!({ VERSION: self })
    }
}

/// The agent's A2UI capabilities, the `params` of its agent-card extension entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    /// The catalogs the agent generates.
    pub supported_catalog_ids: Vec<String>,
    /// Whether the agent reads `inlineCatalogs` from clients (the harness does not).
    #[serde(default)]
    pub accepts_inline_catalogs: bool,
}

impl ServerCapabilities {
    /// The harness: the basic catalog only, no inline catalogs.
    #[must_use]
    pub fn basic() -> Self {
        Self {
            supported_catalog_ids: vec![BASIC_CATALOG.to_owned()],
            accepts_inline_catalogs: false,
        }
    }

    /// As the extension entry's `params`.
    #[must_use]
    pub fn params(&self) -> HashMap<String, Value> {
        match serde_json::to_value(self) {
            Ok(Value::Object(map)) => map.into_iter().collect(),
            _ => HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Common types
// ---------------------------------------------------------------------------

/// A binding to a JSON Pointer path in the surface's data model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataBinding {
    /// The path; absolute from `/`, or relative inside a list template.
    pub path: String,
}

/// The return type of a catalog function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReturnType {
    /// A string.
    String,
    /// A number.
    Number,
    /// A boolean.
    Boolean,
    /// An array.
    Array,
    /// An object.
    Object,
    /// Anything.
    Any,
    /// Nothing.
    Void,
}

/// A call of a catalog function on the client (`required`, `formatDate`, `and`, …).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FunctionCall {
    /// The function.
    pub call: String,
    /// Its arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<BTreeMap<String, Value>>,
    /// Its return type; `boolean` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_type: Option<ReturnType>,
}

/// A literal, a data-model binding or a function call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dynamic<T> {
    /// A binding into the data model.
    Path(DataBinding),
    /// A client-side function call.
    Call(FunctionCall),
    /// The value itself.
    Literal(T),
}

impl<T> Dynamic<T> {
    /// The literal, when it is one.
    pub fn literal(&self) -> Option<&T> {
        match self {
            Self::Literal(v) => Some(v),
            Self::Path(_) | Self::Call(_) => None,
        }
    }
}

impl From<&str> for Dynamic<String> {
    fn from(s: &str) -> Self {
        Self::Literal(s.to_owned())
    }
}

impl From<String> for Dynamic<String> {
    fn from(s: String) -> Self {
        Self::Literal(s)
    }
}

impl<T> Dynamic<T> {
    /// A binding to `path`.
    pub fn path(path: impl Into<String>) -> Self {
        Self::Path(DataBinding { path: path.into() })
    }
}

/// A dynamic string.
pub type DynamicString = Dynamic<String>;
/// A dynamic number.
pub type DynamicNumber = Dynamic<f64>;
/// A dynamic boolean.
pub type DynamicBoolean = Dynamic<bool>;
/// A dynamic list of strings.
pub type DynamicStringList = Dynamic<Vec<String>>;
/// A dynamic value of any type (an action's context entry).
pub type DynamicValue = Dynamic<Value>;

/// Assistive-technology attributes on any component.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Accessibility {
    /// One to three words naming the purpose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<DynamicString>,
    /// Instructions, format requirements or the result of an action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<DynamicString>,
}

/// The children of a layout: a static list of ids, or one template component
/// instantiated per item of a data-model list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Children {
    /// Static children.
    Ids(Vec<ComponentId>),
    /// A template over a list.
    Template(ChildTemplate),
}

/// A list template: `componentId` rendered once per item at `path`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildTemplate {
    /// The template component.
    pub component_id: ComponentId,
    /// The list in the data model.
    pub path: String,
}

/// What a button does: dispatch an event to the agent, or call a client function.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Action {
    /// An event for the agent.
    Event(EventAction),
    /// A client-side function.
    FunctionCall(FunctionCallAction),
}

impl Action {
    /// An event named `name` with no context.
    pub fn event(name: impl Into<String>) -> Self {
        Self::Event(EventAction {
            event: ActionEvent {
                name: name.into(),
                context: None,
            },
        })
    }
}

/// `{ "event": { … } }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventAction {
    /// The event.
    pub event: ActionEvent,
}

/// The event a component dispatches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionEvent {
    /// The action name the agent receives.
    pub name: String,
    /// Key-value context, literals or bindings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<BTreeMap<String, DynamicValue>>,
}

/// `{ "functionCall": { … } }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FunctionCallAction {
    /// The call.
    pub function_call: FunctionCall,
}

/// A client-side check on an input component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRule {
    /// Valid when true.
    pub condition: DynamicBoolean,
    /// Shown when the check fails.
    pub message: String,
}

// ---------------------------------------------------------------------------
// The basic catalog
// ---------------------------------------------------------------------------

/// A component of the basic catalog: the common properties and the per-kind ones,
/// tagged by `component`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Component {
    /// The id, unique within the surface.
    pub id: ComponentId,
    /// Assistive-technology attributes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility: Option<Accessibility>,
    /// Flex weight inside a `Row` or `Column`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    /// The catalog entry and its properties.
    #[serde(flatten)]
    pub kind: ComponentKind,
}

impl Component {
    /// A component `id` of `kind`, without accessibility attributes or weight.
    pub fn new(id: impl Into<String>, kind: ComponentKind) -> Self {
        Self {
            id: id.into(),
            accessibility: None,
            weight: None,
            kind,
        }
    }
}

/// The eighteen entries of the basic catalog (design.md § Surfaces and renderers).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "component")]
pub enum ComponentKind {
    /// Text, plain or markdown.
    Text {
        /// The text.
        text: DynamicString,
        /// Heading level, caption or body.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<TextVariant>,
    },
    /// An image.
    Image {
        /// Where it is.
        url: DynamicString,
        /// Its alt text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<DynamicString>,
        /// How it fills its box.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fit: Option<ImageFit>,
        /// Its size class.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<ImageVariant>,
    },
    /// An icon from the catalog's set, an SVG path or a binding.
    Icon {
        /// Which icon.
        name: IconName,
    },
    /// A video.
    Video {
        /// Where it is.
        url: DynamicString,
    },
    /// An audio player.
    AudioPlayer {
        /// Where the audio is.
        url: DynamicString,
        /// What it is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<DynamicString>,
    },
    /// A horizontal layout.
    Row {
        /// Its children.
        children: Children,
        /// Main-axis distribution.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        justify: Option<Justify>,
        /// Cross-axis alignment.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        align: Option<Align>,
    },
    /// A vertical layout.
    Column {
        /// Its children.
        children: Children,
        /// Main-axis distribution.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        justify: Option<Justify>,
        /// Cross-axis alignment.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        align: Option<Align>,
    },
    /// A scrolling list.
    List {
        /// Its children.
        children: Children,
        /// Which way it scrolls.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        direction: Option<ListDirection>,
        /// Cross-axis alignment.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        align: Option<Align>,
    },
    /// A card around one child.
    Card {
        /// The child.
        child: ComponentId,
    },
    /// Tabs, each with a title and a child.
    Tabs {
        /// The tabs.
        tabs: Vec<Tab>,
    },
    /// A modal opened by a trigger.
    Modal {
        /// The component that opens it.
        trigger: ComponentId,
        /// What it shows.
        content: ComponentId,
    },
    /// A divider.
    Divider {
        /// Its axis.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        axis: Option<Axis>,
    },
    /// A button around one child, running an action.
    Button {
        /// The child (usually a `Text`).
        child: ComponentId,
        /// Its look.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<ButtonVariant>,
        /// What it does.
        action: Action,
        /// Client-side checks before the action runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checks: Option<Vec<CheckRule>>,
    },
    /// A text input.
    TextField {
        /// Its label.
        label: DynamicString,
        /// Its value, usually a binding.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<DynamicString>,
        /// Its input kind.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<TextFieldVariant>,
        /// A regular expression the value must match.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(rename = "validationRegexp")]
        validation_regexp: Option<String>,
        /// Client-side checks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checks: Option<Vec<CheckRule>>,
    },
    /// A check box.
    CheckBox {
        /// Its label.
        label: DynamicString,
        /// Its value, usually a binding.
        value: DynamicBoolean,
        /// Client-side checks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checks: Option<Vec<CheckRule>>,
    },
    /// A choice among options, single or multiple.
    ChoicePicker {
        /// Its label.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<DynamicString>,
        /// One or many.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<ChoiceVariant>,
        /// The options.
        options: Vec<ChoiceOption>,
        /// The selected values, usually a binding.
        value: DynamicStringList,
        /// Check boxes or chips.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(rename = "displayStyle")]
        display_style: Option<DisplayStyle>,
        /// Whether the options can be filtered.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filterable: Option<bool>,
        /// Client-side checks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checks: Option<Vec<CheckRule>>,
    },
    /// A slider between `min` and `max`.
    Slider {
        /// Its label.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<DynamicString>,
        /// The lower bound; 0 when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        /// The upper bound.
        max: f64,
        /// Its value, usually a binding.
        value: DynamicNumber,
        /// Client-side checks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checks: Option<Vec<CheckRule>>,
    },
    /// A date and/or time input.
    DateTimeInput {
        /// Its value as ISO 8601, usually a binding.
        value: DynamicString,
        /// Whether a date is picked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(rename = "enableDate")]
        enable_date: Option<bool>,
        /// Whether a time is picked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(rename = "enableTime")]
        enable_time: Option<bool>,
        /// The earliest allowed, ISO 8601.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<DynamicString>,
        /// The latest allowed, ISO 8601.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<DynamicString>,
        /// Its label.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<DynamicString>,
        /// Client-side checks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checks: Option<Vec<CheckRule>>,
    },
}

impl ComponentKind {
    /// The catalog entry's name, as written in `component`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Text { .. } => "Text",
            Self::Image { .. } => "Image",
            Self::Icon { .. } => "Icon",
            Self::Video { .. } => "Video",
            Self::AudioPlayer { .. } => "AudioPlayer",
            Self::Row { .. } => "Row",
            Self::Column { .. } => "Column",
            Self::List { .. } => "List",
            Self::Card { .. } => "Card",
            Self::Tabs { .. } => "Tabs",
            Self::Modal { .. } => "Modal",
            Self::Divider { .. } => "Divider",
            Self::Button { .. } => "Button",
            Self::TextField { .. } => "TextField",
            Self::CheckBox { .. } => "CheckBox",
            Self::ChoicePicker { .. } => "ChoicePicker",
            Self::Slider { .. } => "Slider",
            Self::DateTimeInput { .. } => "DateTimeInput",
        }
    }
}

/// A tab of `Tabs`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    /// The title.
    pub title: DynamicString,
    /// The child shown when selected.
    pub child: ComponentId,
}

/// An option of a `ChoicePicker`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    /// What is shown.
    pub label: DynamicString,
    /// The stable value.
    pub value: String,
}

impl ChoiceOption {
    /// An option showing `label` worth `value`.
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: Dynamic::Literal(label.into()),
            value: value.into(),
        }
    }
}

/// `Text.variant`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum TextVariant {
    H1,
    H2,
    H3,
    H4,
    H5,
    Caption,
    Body,
}

/// `Image.fit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum ImageFit {
    Contain,
    Cover,
    Fill,
    None,
    ScaleDown,
}

/// `Image.variant`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum ImageVariant {
    Icon,
    Avatar,
    SmallFeature,
    MediumFeature,
    LargeFeature,
    Header,
}

/// Main-axis distribution of a `Row` or `Column`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum Justify {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
}

/// Cross-axis alignment of a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

/// `List.direction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum ListDirection {
    Vertical,
    Horizontal,
}

/// `Divider.axis`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// `Button.variant`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum ButtonVariant {
    Default,
    Primary,
    Borderless,
}

/// `TextField.variant`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum TextFieldVariant {
    LongText,
    Number,
    ShortText,
    Obscured,
}

/// `ChoicePicker.variant`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum ChoiceVariant {
    MultipleSelection,
    MutuallyExclusive,
}

/// `ChoicePicker.displayStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum DisplayStyle {
    Checkbox,
    Chips,
}

/// `Icon.name`: one of the catalog's icons, an SVG path, or a binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IconName {
    /// A catalog icon.
    Named(Icon),
    /// An SVG path.
    Svg(SvgIcon),
    /// A binding resolving to an icon name.
    Path(DataBinding),
}

/// `{ "svgPath": "…" }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SvgIcon {
    /// The path data.
    pub svg_path: String,
}

/// The basic catalog's icon set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum Icon {
    AccountCircle,
    Add,
    ArrowBack,
    ArrowForward,
    AttachFile,
    CalendarToday,
    Call,
    Camera,
    Check,
    Close,
    Delete,
    Download,
    Edit,
    Event,
    Error,
    FastForward,
    Favorite,
    FavoriteOff,
    Folder,
    Help,
    Home,
    Info,
    LocationOn,
    Lock,
    LockOpen,
    Mail,
    Menu,
    MoreVert,
    MoreHoriz,
    NotificationsOff,
    Notifications,
    Pause,
    Payment,
    Person,
    Phone,
    Photo,
    Play,
    Print,
    Refresh,
    Rewind,
    Search,
    Send,
    Settings,
    Share,
    ShoppingCart,
    SkipNext,
    SkipPrevious,
    Star,
    StarHalf,
    StarOff,
    Stop,
    Upload,
    Visibility,
    VisibilityOff,
    VolumeDown,
    VolumeMute,
    VolumeOff,
    VolumeUp,
    Warning,
}

// ---------------------------------------------------------------------------
// A2A parts
// ---------------------------------------------------------------------------

/// Why a part is not an A2UI payload the harness can read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The part is not an A2UI data part.
    #[error("not an A2UI data part")]
    NotA2ui,
    /// The data is not a list of messages.
    #[error("A2UI data must be an array of messages")]
    NotAList,
    /// A message in the list is malformed.
    #[error("A2UI message {index}: {reason}")]
    Message {
        /// Its position in the list.
        index: usize,
        /// What serde said.
        reason: String,
    },
}

/// The `DataPart` carrying `messages`: the list as `data`, [`MEDIA_TYPE`] as the part's
/// media type and as its `metadata.mimeType` (the extension's own marker).
#[must_use]
pub fn part(messages: &[A2uiMessage]) -> a2a::Part {
    let data = serde_json::to_value(messages).unwrap_or_else(|_| Value::Array(Vec::new()));
    let mut part = a2a::Part::data(data).with_media_type(MEDIA_TYPE);
    part.metadata = Some(HashMap::from([(
        MIME_TYPE_KEY.to_owned(),
        Value::String(MEDIA_TYPE.to_owned()),
    )]));
    part
}

/// Whether `part` is an A2UI data part, by its media type or its `metadata.mimeType`.
#[must_use]
pub fn is_a2ui(part: &a2a::Part) -> bool {
    if !matches!(part.content, a2a::PartContent::Data(_)) {
        return false;
    }
    part.media_type.as_deref() == Some(MEDIA_TYPE)
        || part
            .metadata
            .as_ref()
            .and_then(|m| m.get(MIME_TYPE_KEY))
            .and_then(Value::as_str)
            == Some(MEDIA_TYPE)
}

/// The messages in an A2UI data part.
///
/// # Errors
///
/// When the part is not A2UI, its data is not a list, or a message does not parse; a
/// receiver reports the error and goes on with the other parts (the extension's rule).
pub fn parse_part(part: &a2a::Part) -> Result<Vec<A2uiMessage>, ParseError> {
    if !is_a2ui(part) {
        return Err(ParseError::NotA2ui);
    }
    let a2a::PartContent::Data(data) = &part.content else {
        return Err(ParseError::NotA2ui);
    };
    parse_list(data)
}

/// The messages in a payload value (the `data` of a part).
///
/// # Errors
///
/// When `data` is not a list or a message in it does not parse.
pub fn parse_list(data: &Value) -> Result<Vec<A2uiMessage>, ParseError> {
    let items = data.as_array().ok_or(ParseError::NotAList)?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            serde_json::from_value(item.clone()).map_err(|e| ParseError::Message {
                index,
                reason: e.to_string(),
            })
        })
        .collect()
}

/// Every A2UI message in `message`, across its A2UI parts, in order; malformed parts
/// are skipped.
#[must_use]
pub fn messages_in(message: &a2a::Message) -> Vec<A2uiMessage> {
    message
        .parts
        .iter()
        .filter_map(|p| parse_part(p).ok())
        .flatten()
        .collect()
}

/// The catalogs `messages` create surfaces from.
#[must_use]
pub fn catalogs(messages: &[A2uiMessage]) -> BTreeSet<&str> {
    messages
        .iter()
        .filter_map(|m| match m {
            A2uiMessage::CreateSurface(c) => Some(c.catalog_id.as_str()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The text alternative (R25.2)
// ---------------------------------------------------------------------------

/// The plain-text rendering of `messages` for a surface that cannot show A2UI: the
/// component tree from `root`, one line per vertical child, row children joined with
/// ` · `, inputs as their labels (a choice picker lists its options), media as its
/// description or URL. Bound values (paths, function calls) render as nothing.
#[must_use]
pub fn text_alternative(messages: &[A2uiMessage]) -> String {
    let mut by_id: BTreeMap<&str, &Component> = BTreeMap::new();
    for m in messages {
        if let A2uiMessage::UpdateComponents(u) = m {
            for c in &u.components {
                by_id.insert(c.id.as_str(), c);
            }
        }
    }
    let root = by_id.get("root").copied().or_else(|| {
        // Without a root, the first component that no other component references.
        let referenced: BTreeSet<&str> = by_id.values().flat_map(|c| children_of(c)).collect();
        by_id
            .values()
            .find(|c| !referenced.contains(c.id.as_str()))
            .copied()
    });
    let mut lines = Vec::new();
    if let Some(root) = root {
        let mut seen = BTreeSet::new();
        render(root, &by_id, &mut seen, &mut lines);
    }
    lines.join("\n")
}

/// The ids a component references, in order.
fn children_of(c: &Component) -> Vec<&str> {
    match &c.kind {
        ComponentKind::Row { children, .. }
        | ComponentKind::Column { children, .. }
        | ComponentKind::List { children, .. } => match children {
            Children::Ids(ids) => ids.iter().map(String::as_str).collect(),
            Children::Template(t) => vec![t.component_id.as_str()],
        },
        ComponentKind::Card { child } | ComponentKind::Button { child, .. } => vec![child],
        ComponentKind::Tabs { tabs } => tabs.iter().map(|t| t.child.as_str()).collect(),
        ComponentKind::Modal { trigger, content } => vec![trigger, content],
        _ => Vec::new(),
    }
}

fn lit(s: &DynamicString) -> &str {
    s.literal().map_or("", String::as_str)
}

fn opt(s: Option<&DynamicString>) -> &str {
    s.map_or("", lit)
}

/// Renders `c` into `lines`; `seen` breaks reference cycles.
fn render<'a>(
    c: &'a Component,
    by_id: &BTreeMap<&'a str, &'a Component>,
    seen: &mut BTreeSet<&'a str>,
    lines: &mut Vec<String>,
) {
    if !seen.insert(c.id.as_str()) {
        return;
    }
    let push = |lines: &mut Vec<String>, s: String| {
        if !s.is_empty() {
            lines.push(s);
        }
    };
    match &c.kind {
        ComponentKind::Text { text, .. } => push(lines, lit(text).to_owned()),
        ComponentKind::Image {
            url, description, ..
        }
        | ComponentKind::AudioPlayer { url, description } => {
            let d = opt(description.as_ref());
            push(lines, if d.is_empty() { lit(url) } else { d }.to_owned());
        }
        ComponentKind::Video { url } => push(lines, lit(url).to_owned()),
        ComponentKind::Icon { name } => {
            if let IconName::Named(icon) = name {
                push(
                    lines,
                    serde_json::to_value(icon)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                );
            }
        }
        ComponentKind::Row { .. } => {
            let mut cells = Vec::new();
            for id in children_of(c) {
                if let Some(child) = by_id.get(id) {
                    render(child, by_id, seen, &mut cells);
                }
            }
            push(lines, cells.join(" · "));
        }
        ComponentKind::Column { .. }
        | ComponentKind::List { .. }
        | ComponentKind::Card { .. }
        | ComponentKind::Button { .. }
        | ComponentKind::Modal { .. } => {
            for id in children_of(c) {
                if let Some(child) = by_id.get(id) {
                    render(child, by_id, seen, lines);
                }
            }
        }
        ComponentKind::Tabs { tabs } => {
            for tab in tabs {
                let mut body = Vec::new();
                if let Some(child) = by_id.get(tab.child.as_str()) {
                    render(child, by_id, seen, &mut body);
                }
                let title = lit(&tab.title);
                push(
                    lines,
                    if body.is_empty() {
                        title.to_owned()
                    } else {
                        format!("{title}: {}", body.join(" "))
                    },
                );
            }
        }
        ComponentKind::Divider { .. } => {}
        ComponentKind::TextField { label, .. } | ComponentKind::CheckBox { label, .. } => {
            push(lines, lit(label).to_owned());
        }
        ComponentKind::ChoicePicker { label, options, .. } => {
            let names: Vec<&str> = options.iter().map(|o| lit(&o.label)).collect();
            let label = opt(label.as_ref());
            let list = names.join(" / ");
            push(
                lines,
                match (label.is_empty(), list.is_empty()) {
                    (true, _) => list,
                    (false, true) => label.to_owned(),
                    (false, false) => format!("{label}: {list}"),
                },
            );
        }
        ComponentKind::Slider {
            label, min, max, ..
        } => {
            let label = opt(label.as_ref());
            let range = format!("{}–{max}", min.unwrap_or(0.0));
            push(
                lines,
                if label.is_empty() {
                    range
                } else {
                    format!("{label} ({range})")
                },
            );
        }
        ComponentKind::DateTimeInput { label, .. } => push(lines, opt(label.as_ref()).to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ids(ids: &[&str]) -> Children {
        Children::Ids(ids.iter().map(|s| (*s).to_owned()).collect())
    }

    /// One of each of the eighteen catalog entries, every optional field set.
    #[allow(clippy::too_many_lines)]
    fn every_component() -> Vec<Component> {
        let checks = Some(vec![CheckRule {
            condition: Dynamic::Call(FunctionCall {
                call: "required".into(),
                args: Some(BTreeMap::from([(
                    "value".to_owned(),
                    json!({ "path": "/v" }),
                )])),
                return_type: Some(ReturnType::Boolean),
            }),
            message: "Required".into(),
        }]);
        let mut root = Component::new(
            "root",
            ComponentKind::Column {
                children: ids(&["t", "img", "ico", "vid", "aud", "row"]),
                justify: Some(Justify::SpaceBetween),
                align: Some(Align::Stretch),
            },
        );
        root.accessibility = Some(Accessibility {
            label: Some("Form".into()),
            description: Some(Dynamic::path("/help")),
        });
        let mut weighted = Component::new(
            "row",
            ComponentKind::Row {
                children: Children::Template(ChildTemplate {
                    component_id: "t".into(),
                    path: "/items".into(),
                }),
                justify: Some(Justify::Start),
                align: Some(Align::Center),
            },
        );
        weighted.weight = Some(2.0);
        vec![
            root,
            weighted,
            Component::new(
                "t",
                ComponentKind::Text {
                    text: Dynamic::path("name"),
                    variant: Some(TextVariant::H2),
                },
            ),
            Component::new(
                "img",
                ComponentKind::Image {
                    url: "https://x/a.png".into(),
                    description: Some("a cat".into()),
                    fit: Some(ImageFit::ScaleDown),
                    variant: Some(ImageVariant::LargeFeature),
                },
            ),
            Component::new(
                "ico",
                ComponentKind::Icon {
                    name: IconName::Named(Icon::ShoppingCart),
                },
            ),
            Component::new(
                "ico2",
                ComponentKind::Icon {
                    name: IconName::Svg(SvgIcon {
                        svg_path: "M0 0h24v24H0z".into(),
                    }),
                },
            ),
            Component::new(
                "ico3",
                ComponentKind::Icon {
                    name: IconName::Path(DataBinding {
                        path: "/icon".into(),
                    }),
                },
            ),
            Component::new(
                "vid",
                ComponentKind::Video {
                    url: "https://x/a.mp4".into(),
                },
            ),
            Component::new(
                "aud",
                ComponentKind::AudioPlayer {
                    url: "https://x/a.wav".into(),
                    description: Some("a song".into()),
                },
            ),
            Component::new(
                "list",
                ComponentKind::List {
                    children: ids(&["t"]),
                    direction: Some(ListDirection::Horizontal),
                    align: Some(Align::End),
                },
            ),
            Component::new("card", ComponentKind::Card { child: "t".into() }),
            Component::new(
                "tabs",
                ComponentKind::Tabs {
                    tabs: vec![Tab {
                        title: "One".into(),
                        child: "t".into(),
                    }],
                },
            ),
            Component::new(
                "modal",
                ComponentKind::Modal {
                    trigger: "btn".into(),
                    content: "card".into(),
                },
            ),
            Component::new(
                "div",
                ComponentKind::Divider {
                    axis: Some(Axis::Vertical),
                },
            ),
            Component::new(
                "btn",
                ComponentKind::Button {
                    child: "t".into(),
                    variant: Some(ButtonVariant::Primary),
                    action: Action::Event(EventAction {
                        event: ActionEvent {
                            name: "submit".into(),
                            context: Some(BTreeMap::from([
                                ("user".to_owned(), Dynamic::path("/username")),
                                ("static".to_owned(), Dynamic::Literal(json!(42))),
                            ])),
                        },
                    }),
                    checks: checks.clone(),
                },
            ),
            Component::new(
                "btn2",
                ComponentKind::Button {
                    child: "t".into(),
                    variant: None,
                    action: Action::FunctionCall(FunctionCallAction {
                        function_call: FunctionCall {
                            call: "openUrl".into(),
                            args: Some(BTreeMap::from([("url".to_owned(), json!("https://x"))])),
                            return_type: Some(ReturnType::Void),
                        },
                    }),
                    checks: None,
                },
            ),
            Component::new(
                "tf",
                ComponentKind::TextField {
                    label: "Password".into(),
                    value: Some(Dynamic::path("/password")),
                    variant: Some(TextFieldVariant::Obscured),
                    validation_regexp: Some("^.{8,}$".into()),
                    checks: checks.clone(),
                },
            ),
            Component::new(
                "cb",
                ComponentKind::CheckBox {
                    label: "Agree".into(),
                    value: Dynamic::path("/agree"),
                    checks: None,
                },
            ),
            Component::new(
                "cp",
                ComponentKind::ChoicePicker {
                    label: Some("Audience".into()),
                    variant: Some(ChoiceVariant::MutuallyExclusive),
                    options: vec![ChoiceOption::new("Customers", "customers")],
                    value: Dynamic::path("/audience"),
                    display_style: Some(DisplayStyle::Chips),
                    filterable: Some(true),
                    checks: checks.clone(),
                },
            ),
            Component::new(
                "sl",
                ComponentKind::Slider {
                    label: Some("Volume".into()),
                    min: Some(0.0),
                    max: 11.0,
                    value: Dynamic::Literal(7.5),
                    checks: None,
                },
            ),
            Component::new(
                "dt",
                ComponentKind::DateTimeInput {
                    value: Dynamic::path("/when"),
                    enable_date: Some(true),
                    enable_time: Some(false),
                    min: Some("2026-01-01".into()),
                    max: Some(Dynamic::path("/max")),
                    label: Some("When".into()),
                    checks,
                },
            ),
        ]
    }

    /// T1: every basic component serializes to the catalog's shape and parses back.
    #[test]
    fn every_basic_component_round_trips() {
        let components = every_component();
        let names: BTreeSet<&str> = components.iter().map(|c| c.kind.name()).collect();
        assert_eq!(names.len(), 18, "every catalog entry is covered: {names:?}");
        for c in components {
            let value = serde_json::to_value(&c).unwrap();
            assert_eq!(value["component"], c.kind.name(), "{value}");
            assert_eq!(value["id"], c.id, "{value}");
            let back: Component = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(back, c, "{value}");
        }
    }

    #[test]
    fn dynamic_values_read_literals_paths_and_calls() {
        let literal: DynamicString = serde_json::from_value(json!("hi")).unwrap();
        assert_eq!(literal, Dynamic::Literal("hi".into()));
        let bound: DynamicString = serde_json::from_value(json!({ "path": "/x" })).unwrap();
        assert_eq!(bound, Dynamic::path("/x"));
        let call: DynamicString = serde_json::from_value(
            json!({ "call": "formatDate", "args": { "d": "2026" }, "returnType": "string" }),
        )
        .unwrap();
        assert!(
            matches!(call, Dynamic::Call(FunctionCall { ref call, .. }) if call == "formatDate")
        );
        let number: DynamicNumber = serde_json::from_value(json!(3)).unwrap();
        assert_eq!(number, Dynamic::Literal(3.0));
        let list: DynamicStringList = serde_json::from_value(json!(["a", "b"])).unwrap();
        assert_eq!(list, Dynamic::Literal(vec!["a".into(), "b".into()]));
        assert!(serde_json::from_value::<DynamicString>(json!({ "nope": 1 })).is_err());
        assert_eq!(
            serde_json::to_value(Dynamic::<String>::path("/x")).unwrap(),
            json!({ "path": "/x" })
        );
    }

    /// The specification's own samples parse and serialize back field for field.
    #[test]
    fn the_specification_samples_round_trip() {
        for fixture in [
            include_str!("../../../../tests/fixtures/a2ui/simple_login_form.json"),
            include_str!("../../../../tests/fixtures/a2ui/child_list_template.json"),
        ] {
            let sample: Value = serde_json::from_str(fixture).unwrap();
            let messages: Vec<A2uiMessage> =
                serde_json::from_value(sample["messages"].clone()).unwrap();
            assert!(matches!(messages[0], A2uiMessage::CreateSurface(_)));
            assert!(matches!(messages[1], A2uiMessage::UpdateComponents(_)));
            let out = serde_json::to_value(&messages).unwrap();
            for (i, original) in sample["messages"].as_array().unwrap().iter().enumerate() {
                assert_eq!(out[i]["version"], VERSION, "written as 0.9.1");
                let mut expected = original.clone();
                expected["version"] = json!(VERSION);
                assert_eq!(out[i], expected, "message {i} of {}", sample["name"]);
            }
        }
    }

    #[test]
    fn an_unknown_version_is_refused_and_both_known_ones_are_read() {
        let body = json!({ "deleteSurface": { "surfaceId": "s" } });
        for v in ACCEPTED_VERSIONS {
            let mut m = body.clone();
            m["version"] = json!(v);
            let parsed: A2uiMessage = serde_json::from_value(m).unwrap();
            assert_eq!(parsed.surface_id(), "s");
        }
        let mut m = body;
        m["version"] = json!("v2.0");
        let err = serde_json::from_value::<A2uiMessage>(m)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported A2UI version"), "{err}");
    }

    #[test]
    fn client_messages_carry_actions_and_errors() {
        let action = ClientMessage::Action(UserAction {
            name: "submit".into(),
            surface_id: "s".into(),
            source_component_id: "btn".into(),
            timestamp: "2026-01-15T12:00:00Z".into(),
            context: BTreeMap::from([("email".to_owned(), json!("a@b"))]),
        });
        let value = serde_json::to_value(&action).unwrap();
        assert_eq!(value["version"], VERSION);
        assert_eq!(value["action"]["name"], "submit");
        assert_eq!(
            serde_json::from_value::<ClientMessage>(value).unwrap(),
            action
        );
        let error: ClientMessage = serde_json::from_value(json!({
            "version": "v0.9.1",
            "error": { "code": "VALIDATION_FAILED", "surfaceId": "s", "path": "/components/0/text", "message": "no" }
        }))
        .unwrap();
        assert!(
            matches!(error, ClientMessage::Error(ClientError { ref code, .. }) if code == "VALIDATION_FAILED")
        );
    }

    #[test]
    fn client_capabilities_read_either_version_key_and_write_the_current_one() {
        let meta = HashMap::from([(
            CLIENT_CAPABILITIES_KEY.to_owned(),
            json!({ "v0.9": { "supportedCatalogIds": [BASIC_CATALOG] } }),
        )]);
        let caps = ClientCapabilities::from_metadata(Some(&meta)).unwrap();
        assert!(caps.supports(BASIC_CATALOG));
        assert_eq!(
            ClientCapabilities::basic().to_metadata(),
            json!({ "v0.9.1": { "supportedCatalogIds": [BASIC_CATALOG] } })
        );
        assert!(ClientCapabilities::from_metadata(None).is_none());
        let params = ServerCapabilities::basic().params();
        assert_eq!(params["supportedCatalogIds"], json!([BASIC_CATALOG]));
        assert_eq!(params["acceptsInlineCatalogs"], json!(false));
    }

    #[test]
    fn a_part_names_the_media_type_twice_and_parses_back() {
        let messages = vec![A2uiMessage::DeleteSurface(DeleteSurface {
            surface_id: "s".into(),
        })];
        let part = part(&messages);
        assert_eq!(part.media_type.as_deref(), Some(MEDIA_TYPE));
        assert_eq!(part.metadata.as_ref().unwrap()[MIME_TYPE_KEY], MEDIA_TYPE);
        assert!(is_a2ui(&part));
        assert_eq!(parse_part(&part).unwrap(), messages);
        assert!(!is_a2ui(&a2a::Part::text("x")));
        assert_eq!(parse_part(&a2a::Part::text("x")), Err(ParseError::NotA2ui));
        // The extension's marker alone is enough.
        let mut marked = a2a::Part::data(json!([]));
        marked.metadata = Some(HashMap::from([(
            MIME_TYPE_KEY.to_owned(),
            json!(MEDIA_TYPE),
        )]));
        assert!(is_a2ui(&marked));
        assert_eq!(parse_list(&json!({})), Err(ParseError::NotAList));
        assert!(matches!(
            parse_list(&json!([{ "version": "v0.9.1" }])),
            Err(ParseError::Message { index: 0, .. })
        ));
        let message = a2a::Message::new(a2a::Role::Agent, vec![a2a::Part::text("alt"), part]);
        assert_eq!(messages_in(&message), messages);
    }

    #[test]
    fn the_text_alternative_walks_the_tree_from_root() {
        let components = vec![
            Component::new(
                "root",
                ComponentKind::Column {
                    children: ids(&["q", "pick", "other", "actions"]),
                    justify: None,
                    align: None,
                },
            ),
            Component::new(
                "q",
                ComponentKind::Text {
                    text: "Which audience?".into(),
                    variant: Some(TextVariant::H3),
                },
            ),
            Component::new(
                "pick",
                ComponentKind::ChoicePicker {
                    label: None,
                    variant: Some(ChoiceVariant::MutuallyExclusive),
                    options: vec![
                        ChoiceOption::new("Customers", "customers"),
                        ChoiceOption::new("Internal", "internal"),
                    ],
                    value: Dynamic::path("/audience"),
                    display_style: None,
                    filterable: None,
                    checks: None,
                },
            ),
            Component::new(
                "other",
                ComponentKind::TextField {
                    label: "Something else".into(),
                    value: Some(Dynamic::path("/other")),
                    variant: None,
                    validation_regexp: None,
                    checks: None,
                },
            ),
            Component::new(
                "actions",
                ComponentKind::Row {
                    children: ids(&["send", "skip"]),
                    justify: None,
                    align: None,
                },
            ),
            Component::new(
                "send",
                ComponentKind::Button {
                    child: "send_t".into(),
                    variant: Some(ButtonVariant::Primary),
                    action: Action::event("answer"),
                    checks: None,
                },
            ),
            Component::new(
                "send_t",
                ComponentKind::Text {
                    text: "Send answer".into(),
                    variant: None,
                },
            ),
            Component::new(
                "skip",
                ComponentKind::Button {
                    child: "skip_t".into(),
                    variant: None,
                    action: Action::event("skip"),
                    checks: None,
                },
            ),
            Component::new(
                "skip_t",
                ComponentKind::Text {
                    text: "Skip".into(),
                    variant: None,
                },
            ),
        ];
        let messages = vec![
            A2uiMessage::CreateSurface(CreateSurface {
                surface_id: "ask".into(),
                catalog_id: BASIC_CATALOG.into(),
                theme: None,
                send_data_model: Some(true),
            }),
            A2uiMessage::UpdateComponents(UpdateComponents {
                surface_id: "ask".into(),
                components,
            }),
        ];
        assert_eq!(
            text_alternative(&messages),
            "Which audience?\nCustomers / Internal\nSomething else\nSend answer · Skip"
        );
        assert_eq!(catalogs(&messages), BTreeSet::from([BASIC_CATALOG]));
        assert_eq!(text_alternative(&[]), "");
    }
}
