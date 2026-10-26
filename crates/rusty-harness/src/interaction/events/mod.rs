//! Inbound event sources (webhook receivers, channel adapters) that feed the inbox (R20.1).
//!
//! An adapter turns something that arrived over HTTP into an [`InboxPost`], the one shape
//! the `/inbox` route accepts; the route then runs it through `Inbox::accept` like any
//! A2A request. Two adapters ship: the identity one for callers that already speak the
//! `/inbox` shape (another harness's push notification), and a generic JSON adapter that
//! wraps any body as a message to a configured task.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::harness::core_loop::task::{ContextId, TaskId};

/// What `/inbox` accepts: one A2A `StreamResponse` addressed to a task or a context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxPost {
    /// The task, when the event addresses one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    /// The context, when a message should create a task in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<ContextId>,
    /// The event, in the shape A2A streams and pushes it.
    pub event: a2a::StreamResponse,
    /// Extension URIs the sender activated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

/// Why an adapter could not translate a body.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdapterError {
    /// The body is not what the adapter expects.
    #[error("{adapter}: {reason}")]
    Malformed {
        /// Which adapter.
        adapter: String,
        /// Why.
        reason: String,
    },
}

/// Turns an inbound HTTP body into an inbox post, or `None` when the body is not an
/// event (a ping, an unsupported type).
pub trait WebhookAdapter: Send + Sync {
    /// The adapter's name, as the route `/events/<name>` spells it.
    fn name(&self) -> &str;
    /// Translates `body` (with lower-cased `headers`).
    ///
    /// # Errors
    ///
    /// [`AdapterError::Malformed`].
    fn translate(
        &self,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<Option<InboxPost>, AdapterError>;
}

/// The identity adapter: the body already is an [`InboxPost`].
#[derive(Debug, Default, Clone, Copy)]
pub struct InboxPostAdapter;

impl WebhookAdapter for InboxPostAdapter {
    #[allow(clippy::unnecessary_literal_bound)] // the trait's signature
    fn name(&self) -> &str {
        "inbox"
    }
    fn translate(
        &self,
        _headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<Option<InboxPost>, AdapterError> {
        serde_json::from_slice(body)
            .map(Some)
            .map_err(|e| AdapterError::Malformed {
                adapter: "inbox".into(),
                reason: e.to_string(),
            })
    }
}

/// Wraps any JSON body as a user message to `task` (a CI system's result, a form).
#[derive(Debug, Clone)]
pub struct JsonMessageAdapter {
    /// The route name.
    pub name: String,
    /// The task every event of this source goes to.
    pub task: TaskId,
    /// A sender id for the message's provenance.
    pub sender: String,
}

impl WebhookAdapter for JsonMessageAdapter {
    fn name(&self) -> &str {
        &self.name
    }
    fn translate(
        &self,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<Option<InboxPost>, AdapterError> {
        if headers
            .get("content-type")
            .is_some_and(|c| !c.contains("json"))
        {
            return Ok(None);
        }
        let value: serde_json::Value =
            serde_json::from_slice(body).map_err(|e| AdapterError::Malformed {
                adapter: self.name.clone(),
                reason: e.to_string(),
            })?;
        let mut message = a2a::Message::new(a2a::Role::User, vec![a2a::Part::data(value)]);
        message.task_id = Some(self.task.clone());
        message.metadata = Some(std::collections::HashMap::from([(
            "source".to_owned(),
            serde_json::json!({ "adapter": self.name, "sender": self.sender }),
        )]));
        Ok(Some(InboxPost {
            task_id: Some(self.task.clone()),
            context_id: None,
            event: a2a::StreamResponse::Message(message),
            extensions: Vec::new(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{InboxPost, InboxPostAdapter, JsonMessageAdapter, WebhookAdapter};
    use std::collections::BTreeMap;

    #[test]
    fn adapters_translate_bodies_into_inbox_posts_or_nothing() {
        let post = InboxPost {
            task_id: Some("t1".into()),
            context_id: None,
            event: a2a::StreamResponse::Message(a2a::Message::new(
                a2a::Role::User,
                vec![a2a::Part::text("hi")],
            )),
            extensions: vec![],
        };
        let body = serde_json::to_vec(&post).unwrap();
        assert_eq!(
            InboxPostAdapter.translate(&BTreeMap::new(), &body).unwrap(),
            Some(post)
        );
        assert!(InboxPostAdapter.translate(&BTreeMap::new(), b"{").is_err());
        let json = JsonMessageAdapter {
            name: "ci".into(),
            task: "t1".into(),
            sender: "ci-bot".into(),
        };
        let headers = BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]);
        let translated = json
            .translate(&headers, br#"{"status":"green"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(translated.task_id.as_deref(), Some("t1"));
        assert!(
            matches!(translated.event, a2a::StreamResponse::Message(m) if m.task_id.as_deref() == Some("t1"))
        );
        let text = BTreeMap::from([("content-type".to_owned(), "text/plain".to_owned())]);
        assert_eq!(json.translate(&text, b"ping").unwrap(), None);
    }
}
