//! Surface capabilities and modality negotiation (R25.1–3).
//!
//! A surface is an A2A client. It declares what it renders — media types and A2UI
//! catalogs — and the harness sends it only that, with a text alternative for the rest
//! ([`SurfaceCapabilities::negotiate`]). Over A2A the declaration is the message's
//! `a2uiClientCapabilities` metadata ([`SurfaceCapabilities::from_message`]); the
//! `acceptedOutputModes` of a send configuration never reach the executor, so a surface
//! that restricts media types applies the same negotiation on its own side (the TUI
//! does; design.md § Surfaces and renderers).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::interaction::renderer::a2ui;

/// A media type such as `text/plain` or `image/*`.
pub type MediaType = String;
/// An A2UI catalog id.
pub type CatalogId = String;

/// What a surface can take in and render (R25.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceCapabilities {
    /// What the surface sends: text, audio, images.
    #[serde(default)]
    pub input_modes: Vec<MediaType>,
    /// What it renders; an empty list is a surface that did not say, which receives
    /// everything but A2UI (an A2UI payload always needs a declared catalog).
    #[serde(default)]
    pub output_modes: Vec<MediaType>,
    /// The A2UI catalogs it renders.
    #[serde(default)]
    pub a2ui_catalogs: Vec<CatalogId>,
    /// Whether it renders tool UIs through MCP Apps (R25.5; nothing produces them yet).
    #[serde(default)]
    pub mcp_apps: bool,
}

impl Default for SurfaceCapabilities {
    fn default() -> Self {
        Self::text_only()
    }
}

const TEXT_PLAIN: &str = "text/plain";

impl SurfaceCapabilities {
    /// A surface that reads and writes plain text and nothing else.
    #[must_use]
    pub fn text_only() -> Self {
        Self {
            input_modes: vec![TEXT_PLAIN.to_owned()],
            output_modes: vec![TEXT_PLAIN.to_owned()],
            a2ui_catalogs: Vec::new(),
            mcp_apps: false,
        }
    }

    /// A surface that declared nothing: every media type, no A2UI.
    #[must_use]
    pub fn undeclared() -> Self {
        Self {
            input_modes: Vec::new(),
            output_modes: Vec::new(),
            a2ui_catalogs: Vec::new(),
            mcp_apps: false,
        }
    }

    /// Also renders `catalog`.
    #[must_use]
    pub fn with_a2ui(mut self, catalog: impl Into<CatalogId>) -> Self {
        self.a2ui_catalogs.push(catalog.into());
        self
    }

    /// Also renders `media_type` (a wildcard such as `image/*` is fine).
    #[must_use]
    pub fn with_output(mut self, media_type: impl Into<MediaType>) -> Self {
        self.output_modes.push(media_type.into());
        self
    }

    /// The surface behind an incoming A2A message: undeclared, plus the A2UI catalogs
    /// its `a2uiClientCapabilities` metadata names.
    #[must_use]
    pub fn from_message(message: &a2a::Message) -> Self {
        Self::from_metadata(message.metadata.as_ref())
    }

    /// The surface behind request metadata (see [`Self::from_message`]).
    #[must_use]
    pub fn from_metadata(metadata: Option<&HashMap<String, Value>>) -> Self {
        let mut caps = Self::undeclared();
        if let Some(client) = a2ui::ClientCapabilities::from_metadata(metadata) {
            caps.a2ui_catalogs = client.supported_catalog_ids;
        }
        caps
    }

    /// What this surface puts in its outgoing message metadata: the A2UI client
    /// capabilities when it renders a catalog, nothing otherwise.
    #[must_use]
    pub fn client_metadata(&self) -> HashMap<String, Value> {
        if self.a2ui_catalogs.is_empty() {
            return HashMap::new();
        }
        let client = a2ui::ClientCapabilities {
            supported_catalog_ids: self.a2ui_catalogs.clone(),
            inline_catalogs: Vec::new(),
        };
        HashMap::from([(
            a2ui::CLIENT_CAPABILITIES_KEY.to_owned(),
            client.to_metadata(),
        )])
    }

    /// Whether the surface renders `media_type`: text always, anything when it declared
    /// nothing, else an exact or wildcard match (`image/*`, `*/*`).
    #[must_use]
    pub fn accepts(&self, media_type: &str) -> bool {
        let media = media_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if media.starts_with("text/") || self.output_modes.is_empty() {
            return true;
        }
        self.output_modes.iter().any(|mode| {
            let mode = mode.trim().to_ascii_lowercase();
            mode == media
                || mode == "*/*"
                || mode
                    .strip_suffix("/*")
                    .is_some_and(|prefix| media.split('/').next() == Some(prefix))
        })
    }

    /// Whether the surface renders an A2UI payload using `catalogs`: every catalog it
    /// creates a surface from is declared; a payload creating no surface needs any
    /// declared catalog.
    #[must_use]
    pub fn renders_a2ui<'a>(&self, catalogs: impl IntoIterator<Item = &'a str>) -> bool {
        if self.a2ui_catalogs.is_empty() {
            return false;
        }
        catalogs
            .into_iter()
            .all(|c| self.a2ui_catalogs.iter().any(|mine| mine == c))
    }

    /// `message` as this surface receives it: the parts it renders, a text part in
    /// place of each it does not (unless the message already carries that part's
    /// alternative, marked `alternativeFor`), the A2UI extension dropped when no A2UI
    /// part survived. `fallback` names what a part is when nothing better is known
    /// (its argument is the media type).
    #[must_use]
    pub fn negotiate(
        &self,
        mut message: a2a::Message,
        fallback: &dyn Fn(&str) -> String,
    ) -> a2a::Message {
        let parts = std::mem::take(&mut message.parts);
        message.parts = self.negotiate_parts(parts, fallback);
        if !message.parts.iter().any(a2ui::is_a2ui) {
            if let Some(ext) = message.extensions.as_mut() {
                ext.retain(|e| e != a2ui::EXTENSION_URI);
                if ext.is_empty() {
                    message.extensions = None;
                }
            }
            if let Some(meta) = message.metadata.as_mut() {
                meta.remove(a2ui::EXTENSION_URI);
            }
        }
        message
    }

    /// The parts of a message or artifact as this surface receives them (see
    /// [`Self::negotiate`]).
    #[must_use]
    pub fn negotiate_parts(
        &self,
        parts: Vec<a2a::Part>,
        fallback: &dyn Fn(&str) -> String,
    ) -> Vec<a2a::Part> {
        let covered: Vec<String> = parts
            .iter()
            .filter(|p| matches!(p.content, a2a::PartContent::Text(_)))
            .filter_map(|p| alternative_for(p).map(str::to_owned))
            .collect();
        let mut out = Vec::with_capacity(parts.len());
        for part in parts {
            if matches!(part.content, a2a::PartContent::Text(_)) {
                out.push(part);
                continue;
            }
            let media = media_of(&part);
            let a2ui_messages = a2ui::parse_part(&part).ok();
            let keep = match &a2ui_messages {
                Some(messages) => self.renders_a2ui(a2ui::catalogs(messages)),
                None => self.accepts(&media),
            };
            if keep {
                out.push(part);
            } else if !covered.iter().any(|c| c == &media) {
                let text = alternative_of(&part, a2ui_messages.as_deref())
                    .unwrap_or_else(|| fallback(&media));
                let mut alt = a2a::Part::text(text);
                alt.metadata = Some(HashMap::from([(
                    a2ui::ALTERNATIVE_FOR_KEY.to_owned(),
                    Value::String(media),
                )]));
                out.push(alt);
            }
        }
        out
    }

    /// A stream event as this surface receives it: every message and artifact in it
    /// negotiated (see [`Self::negotiate`]).
    #[must_use]
    pub fn negotiate_response(
        &self,
        response: a2a::StreamResponse,
        fallback: &dyn Fn(&str) -> String,
    ) -> a2a::StreamResponse {
        match response {
            a2a::StreamResponse::Message(m) => {
                a2a::StreamResponse::Message(self.negotiate(m, fallback))
            }
            a2a::StreamResponse::Task(mut task) => {
                if let Some(m) = task.status.message.take() {
                    task.status.message = Some(self.negotiate(m, fallback));
                }
                if let Some(history) = task.history.take() {
                    task.history = Some(
                        history
                            .into_iter()
                            .map(|m| self.negotiate(m, fallback))
                            .collect(),
                    );
                }
                if let Some(artifacts) = task.artifacts.as_mut() {
                    for artifact in artifacts.iter_mut() {
                        let parts = std::mem::take(&mut artifact.parts);
                        artifact.parts = self.negotiate_parts(parts, fallback);
                    }
                }
                a2a::StreamResponse::Task(task)
            }
            a2a::StreamResponse::StatusUpdate(mut update) => {
                if let Some(m) = update.status.message.take() {
                    update.status.message = Some(self.negotiate(m, fallback));
                }
                a2a::StreamResponse::StatusUpdate(update)
            }
            a2a::StreamResponse::ArtifactUpdate(mut update) => {
                let parts = std::mem::take(&mut update.artifact.parts);
                update.artifact.parts = self.negotiate_parts(parts, fallback);
                a2a::StreamResponse::ArtifactUpdate(update)
            }
        }
    }
}

/// The media type a part claims, or the one its content implies.
fn media_of(part: &a2a::Part) -> String {
    if let Some(m) = part.media_type.as_deref() {
        return m.to_owned();
    }
    if let Some(m) = part
        .metadata
        .as_ref()
        .and_then(|m| m.get(a2ui::MIME_TYPE_KEY))
        .and_then(Value::as_str)
    {
        return m.to_owned();
    }
    match part.content {
        a2a::PartContent::Text(_) => TEXT_PLAIN.to_owned(),
        a2a::PartContent::Data(_) => "application/json".to_owned(),
        a2a::PartContent::Raw(_) | a2a::PartContent::Url(_) => {
            "application/octet-stream".to_owned()
        }
    }
}

/// The media type a text part stands in for, when it is marked as an alternative.
fn alternative_for(part: &a2a::Part) -> Option<&str> {
    part.metadata
        .as_ref()?
        .get(a2ui::ALTERNATIVE_FOR_KEY)?
        .as_str()
}

/// The best text for a part a surface cannot render: its own alt text in metadata, the
/// A2UI tree rendered as text, its filename, or its URL.
fn alternative_of(part: &a2a::Part, a2ui_messages: Option<&[a2ui::A2uiMessage]>) -> Option<String> {
    let from_metadata = part.metadata.as_ref().and_then(|m| {
        ["alt", "description", "text"]
            .iter()
            .find_map(|k| m.get(*k).and_then(Value::as_str))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    });
    if from_metadata.is_some() {
        return from_metadata;
    }
    if let Some(messages) = a2ui_messages {
        let text = a2ui::text_alternative(messages);
        if !text.is_empty() {
            return Some(text);
        }
    }
    if let Some(name) = part.filename.as_deref().filter(|f| !f.is_empty()) {
        return Some(name.to_owned());
    }
    match &part.content {
        a2a::PartContent::Url(url) if !url.is_empty() => Some(url.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::interaction::renderer::a2ui::{
        A2uiMessage, Component, ComponentKind, CreateSurface, UpdateComponents,
    };

    fn fallback(media: &str) -> String {
        format!("[{media}]")
    }

    fn form() -> Vec<A2uiMessage> {
        vec![
            A2uiMessage::CreateSurface(CreateSurface {
                surface_id: "s".into(),
                catalog_id: a2ui::BASIC_CATALOG.into(),
                theme: None,
                send_data_model: None,
            }),
            A2uiMessage::UpdateComponents(UpdateComponents {
                surface_id: "s".into(),
                components: vec![Component::new(
                    "root",
                    ComponentKind::Text {
                        text: "Which audience?".into(),
                        variant: None,
                    },
                )],
            }),
        ]
    }

    #[test]
    fn accepts_matches_exactly_by_wildcard_and_always_for_text() {
        let caps = SurfaceCapabilities::text_only().with_output("image/*");
        assert!(caps.accepts("text/markdown"));
        assert!(caps.accepts("image/png"));
        assert!(caps.accepts("IMAGE/JPEG; q=0.9"));
        assert!(!caps.accepts("audio/wav"));
        assert!(SurfaceCapabilities::undeclared().accepts("audio/wav"));
        assert!(
            SurfaceCapabilities::undeclared()
                .with_output("*/*")
                .accepts("video/mp4")
        );
    }

    #[test]
    fn renders_a2ui_needs_every_catalog_declared() {
        let caps = SurfaceCapabilities::text_only().with_a2ui(a2ui::BASIC_CATALOG);
        assert!(caps.renders_a2ui([a2ui::BASIC_CATALOG]));
        assert!(caps.renders_a2ui(std::iter::empty()));
        assert!(!caps.renders_a2ui(["https://other.example/catalog.json"]));
        assert!(!SurfaceCapabilities::undeclared().renders_a2ui(std::iter::empty()));
    }

    #[test]
    fn a_surface_without_a2ui_gets_the_text_alternative_instead_of_the_data_part() {
        let message = a2a::Message::new(a2a::Role::Agent, vec![a2ui::part(&form())]);
        let out = SurfaceCapabilities::text_only().negotiate(message, &fallback);
        assert_eq!(out.parts.len(), 1);
        assert_eq!(out.parts[0].as_text(), Some("Which audience?"));
        assert_eq!(
            out.parts[0].metadata.as_ref().unwrap()[a2ui::ALTERNATIVE_FOR_KEY],
            a2ui::MEDIA_TYPE
        );
    }

    #[test]
    fn an_alternative_already_in_the_message_is_not_duplicated_and_the_extension_goes() {
        let mut alt = a2a::Part::text("Which audience?");
        alt.metadata = Some(HashMap::from([(
            a2ui::ALTERNATIVE_FOR_KEY.to_owned(),
            Value::String(a2ui::MEDIA_TYPE.to_owned()),
        )]));
        let mut message = a2a::Message::new(a2a::Role::Agent, vec![alt, a2ui::part(&form())]);
        message.extensions = Some(vec![a2ui::EXTENSION_URI.to_owned()]);
        message.metadata = Some(HashMap::from([(a2ui::EXTENSION_URI.to_owned(), json!({}))]));
        let out = SurfaceCapabilities::text_only().negotiate(message.clone(), &fallback);
        assert_eq!(out.parts.len(), 1, "{out:?}");
        assert_eq!(out.extensions, None);
        assert!(!out.metadata.unwrap().contains_key(a2ui::EXTENSION_URI));

        let capable = SurfaceCapabilities::text_only().with_a2ui(a2ui::BASIC_CATALOG);
        let out = capable.negotiate(message, &fallback);
        assert_eq!(out.parts.len(), 2, "both parts for a capable surface");
        assert_eq!(out.extensions, Some(vec![a2ui::EXTENSION_URI.to_owned()]));
    }

    #[test]
    fn an_unrenderable_file_becomes_its_alt_text_then_its_name_then_the_fallback() {
        let caps = SurfaceCapabilities::text_only();
        let mut image = a2a::Part::data(json!({})).with_media_type("image/png");
        image.content = a2a::PartContent::Url("https://x/y.png".into());
        image.metadata = Some(HashMap::from([("alt".to_owned(), json!("a cat"))]));
        let named = a2a::Part::data(json!({}))
            .with_media_type("audio/wav")
            .with_filename("note.wav");
        let bare = a2a::Part::data(json!({ "k": 1 })).with_media_type("application/x-thing");
        let out = caps.negotiate_parts(vec![image, named, bare], &fallback);
        let texts: Vec<_> = out
            .iter()
            .map(|p| p.as_text().unwrap().to_owned())
            .collect();
        assert_eq!(texts, ["a cat", "note.wav", "[application/x-thing]"]);
        let undeclared = SurfaceCapabilities::undeclared();
        let kept = undeclared.negotiate_parts(
            vec![a2a::Part::data(json!({ "k": 1 })).with_media_type("application/x-thing")],
            &fallback,
        );
        assert!(matches!(kept[0].content, a2a::PartContent::Data(_)));
    }

    #[test]
    fn the_surface_behind_a_message_is_read_from_its_a2ui_metadata() {
        let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("hi")]);
        assert_eq!(
            SurfaceCapabilities::from_message(&message).a2ui_catalogs,
            Vec::<String>::new()
        );
        message.metadata = Some(
            SurfaceCapabilities::text_only()
                .with_a2ui(a2ui::BASIC_CATALOG)
                .client_metadata(),
        );
        let caps = SurfaceCapabilities::from_message(&message);
        assert_eq!(caps.a2ui_catalogs, [a2ui::BASIC_CATALOG]);
        assert!(
            caps.output_modes.is_empty(),
            "output modes are not on the wire"
        );
        assert!(
            SurfaceCapabilities::text_only()
                .client_metadata()
                .is_empty()
        );
    }

    #[test]
    fn a_stream_event_is_negotiated_in_its_status_history_and_artifacts() {
        let caps = SurfaceCapabilities::text_only();
        let message = a2a::Message::new(a2a::Role::Agent, vec![a2ui::part(&form())]);
        let task = a2a::Task {
            id: "t".into(),
            context_id: "c".into(),
            status: a2a::TaskStatus {
                state: a2a::TaskState::InputRequired,
                message: Some(message.clone()),
                timestamp: None,
            },
            artifacts: Some(vec![a2a::Artifact {
                artifact_id: "a".into(),
                name: None,
                description: None,
                parts: vec![a2ui::part(&form())],
                metadata: None,
                extensions: None,
            }]),
            history: Some(vec![message]),
            metadata: None,
        };
        let a2a::StreamResponse::Task(out) =
            caps.negotiate_response(a2a::StreamResponse::Task(task), &fallback)
        else {
            panic!("still a task");
        };
        assert_eq!(
            out.status.message.unwrap().parts[0].as_text(),
            Some("Which audience?")
        );
        assert_eq!(
            out.history.unwrap()[0].parts[0].as_text(),
            Some("Which audience?")
        );
        assert_eq!(
            out.artifacts.unwrap()[0].parts[0].as_text(),
            Some("Which audience?")
        );
    }
}
