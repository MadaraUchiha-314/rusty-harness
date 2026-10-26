//! `OpenAI`'s Responses API (`POST /v1/responses`): the wire for `OpenAI`'s own models,
//! where Chat Completions refuses function tools together with reasoning on current
//! models. Same [`ModelRequest`] in, same [`ModelEvent`]s out as the Chat Completions
//! wire; requests are not stored on `OpenAI`'s side (`store: false`).

use base64::Engine as _;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::{Value, json};

use super::super::sse;
use super::super::{
    ContentPart, EventStream, MessageRole, ModelError, ModelEvent, ModelRequest, StopReason, Usage,
    WireNames,
};
use crate::harness::tools::ToolCall;

/// The request body for `request` (snapshot-tested).
#[must_use]
pub fn body(model: &str, request: &ModelRequest) -> Value {
    let names = WireNames::new(&request.tools);
    let mut input = Vec::new();
    for message in &request.messages {
        match message.role {
            MessageRole::User => {
                input.push(json!({ "role": "user", "content": user_parts(&message.parts) }));
            }
            MessageRole::Assistant => {
                let text = text_of(&message.parts);
                if !text.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": text }],
                    }));
                }
                for call in &message.tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": names.wire(&call.name),
                        "arguments": call.arguments.to_string(),
                    }));
                }
            }
            MessageRole::Tool => input.push(json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id,
                "output": text_of(&message.parts),
            })),
        }
    }
    let mut body = json!({
        "model": model,
        "instructions": request.instructions.render(),
        "input": input,
        "stream": true,
        "store": false,
    });
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": names.wire(&t.name),
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect();
    }
    if let Some(effort) = request.effort {
        body["reasoning"] = json!({ "effort": effort.to_string() });
    }
    if let Some(max) = request.max_output_tokens {
        body["max_output_tokens"] = json!(max);
    }
    body
}

fn user_parts(parts: &[ContentPart]) -> Value {
    parts
        .iter()
        .map(|p| match p {
            ContentPart::Text { text } => json!({ "type": "input_text", "text": text }),
            ContentPart::Image { media_type, data } => json!({
                "type": "input_image",
                "image_url": format!(
                    "data:{media_type};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(data)
                ),
            }),
            ContentPart::Audio { media_type, data } => json!({
                "type": "input_audio",
                "input_audio": {
                    "data": base64::engine::general_purpose::STANDARD.encode(data),
                    "format": media_type.strip_prefix("audio/").unwrap_or(media_type),
                },
            }),
        })
        .collect()
}

fn text_of(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One server-sent event of a streamed response; only the fields the harness reads.
#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    delta: Option<String>,
    #[serde(default)]
    item: Option<Item>,
    #[serde(default)]
    response: Option<Response>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    usage: Option<ResponseUsage>,
    #[serde(default)]
    incomplete_details: Option<Incomplete>,
    #[serde(default)]
    error: Option<ResponseError>,
}

#[derive(Deserialize)]
struct ResponseUsage {
    input_tokens: u64,
    output_tokens: u64,
    #[serde(default)]
    input_tokens_details: Option<InputDetails>,
}

#[derive(Deserialize)]
struct InputDetails {
    #[serde(default)]
    cached_tokens: u64,
}

#[derive(Deserialize)]
struct Incomplete {
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseError {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Default)]
struct Decoder {
    called: bool,
    stop: Option<StopReason>,
}

impl Decoder {
    /// The events for one SSE `data:`; `Err` ends the stream.
    fn feed(&mut self, data: &str) -> Result<Vec<ModelEvent>, ModelError> {
        let event: Event =
            serde_json::from_str(data).map_err(|e| ModelError::Decode(e.to_string()))?;
        let mut out = Vec::new();
        match event.kind.as_str() {
            "response.output_text.delta" => {
                if let Some(text) = event.delta.filter(|t| !t.is_empty()) {
                    out.push(ModelEvent::TextDelta { text });
                }
            }
            "response.output_item.done" => {
                if let Some(item) = event.item.filter(|i| i.kind == "function_call") {
                    self.called = true;
                    let arguments = item.arguments.unwrap_or_default();
                    out.push(ModelEvent::ToolCall {
                        call: ToolCall {
                            id: item.call_id.unwrap_or_default(),
                            name: item.name.unwrap_or_default(),
                            arguments: serde_json::from_str(&arguments)
                                .unwrap_or(Value::Object(serde_json::Map::new())),
                        },
                    });
                }
            }
            "response.completed" | "response.incomplete" => {
                let response = event.response;
                if let Some(usage) = response.as_ref().and_then(|r| r.usage.as_ref()) {
                    out.push(ModelEvent::Usage {
                        usage: Usage {
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                            cached_input_tokens: usage
                                .input_tokens_details
                                .as_ref()
                                .map_or(0, |d| d.cached_tokens),
                        },
                    });
                }
                let incomplete = response
                    .and_then(|r| r.incomplete_details)
                    .and_then(|d| d.reason);
                self.stop = Some(match incomplete.as_deref() {
                    Some("max_output_tokens") => StopReason::MaxTokens,
                    Some(other) => StopReason::Other(other.to_owned()),
                    None if self.called => StopReason::ToolUse,
                    None => StopReason::EndTurn,
                });
            }
            "response.failed" => {
                let message = event
                    .response
                    .and_then(|r| r.error)
                    .and_then(|e| e.message)
                    .unwrap_or_else(|| "the response failed".into());
                return Err(ModelError::Retryable {
                    status: None,
                    message,
                    retry_after: None,
                });
            }
            "error" => {
                return Err(ModelError::NonRetryable {
                    status: None,
                    message: event.message.unwrap_or_else(|| "stream error".into()),
                });
            }
            _ => {}
        }
        Ok(out)
    }

    /// The stop, once the stream has ended: only `response.completed` or
    /// `response.incomplete` ends a turn, so a stream cut before either (a dropped
    /// connection, a proxy timeout) is a retryable failure, not a truncated answer.
    fn finish(&mut self) -> Result<ModelEvent, ModelError> {
        self.stop
            .take()
            .map(|reason| ModelEvent::Stop { reason })
            .ok_or_else(|| ModelError::Retryable {
                status: None,
                message: "the response stream ended before response.completed".into(),
                retry_after: None,
            })
    }
}

/// Turns the Responses API's server-sent events into model events.
pub fn decode(bytes: crate::host::ByteStream) -> EventStream<'static> {
    let stream = futures::stream::unfold(
        (sse::events(bytes), Decoder::default(), false),
        |(mut sse, mut decoder, ended)| async move {
            if ended {
                return None;
            }
            Some(match sse.next().await {
                None => (vec![decoder.finish()], (sse, decoder, true)),
                Some(Err(err)) => (
                    vec![Err(ModelError::Retryable {
                        status: None,
                        message: err.to_string(),
                        retry_after: None,
                    })],
                    (sse, decoder, true),
                ),
                Some(Ok(event)) => match decoder.feed(&event.data) {
                    Ok(events) => (events.into_iter().map(Ok).collect(), (sse, decoder, false)),
                    Err(err) => (vec![Err(err)], (sse, decoder, true)),
                },
            })
        },
    )
    .flat_map(futures::stream::iter);
    crate::runtime::boxed_stream(stream)
}

#[cfg(test)]
mod tests {
    use super::{body, decode};
    use crate::assets::{DataRole, Prompt};
    use crate::harness::models::llm::sse::chunks;
    use crate::harness::models::llm::{
        ContextMessage, MessageRole, ModelEvent, ModelRequest, StopReason, collect,
    };
    use crate::harness::tools::{Idempotency, ToolCall, ToolDescriptor};

    fn request() -> ModelRequest {
        let mut req = ModelRequest::new(
            Prompt::data(DataRole::TaskField, "be brief"),
            vec![
                ContextMessage::text(MessageRole::User, "hi"),
                ContextMessage::text(MessageRole::Assistant, "checking"),
                ContextMessage::tool_result("call_1", "42"),
            ],
        );
        req.messages[1].tool_calls.push(ToolCall {
            id: "call_1".into(),
            name: "math.add".into(),
            arguments: serde_json::json!({ "a": 1 }),
        });
        req.tools.push(ToolDescriptor {
            name: "math.add".into(),
            description: "adds".into(),
            parameters: serde_json::json!({ "type": "object" }),
            idempotency: Idempotency::Idempotent,
        });
        req.effort = Some(crate::harness::models::llm::Effort::High);
        req.max_output_tokens = Some(256);
        req
    }

    #[test]
    fn snapshot_openai_responses_request_body() {
        let body = body("gpt-6.1-sol", &request());
        assert_eq!(body["model"], "gpt-6.1-sol");
        assert_eq!(
            body["store"], false,
            "nothing is kept on the provider's side"
        );
        assert!(
            body["instructions"]
                .as_str()
                .is_some_and(|i| i.contains("be brief"))
        );
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["content"][0]["type"], "output_text");
        assert_eq!(body["input"][2]["type"], "function_call");
        assert_eq!(body["input"][2]["name"], "math_add");
        assert_eq!(body["input"][2]["arguments"], "{\"a\":1}");
        assert_eq!(body["input"][3]["type"], "function_call_output");
        assert_eq!(body["input"][3]["call_id"], "call_1");
        assert_eq!(
            body["tools"][0]["name"], "math_add",
            "flat function tools, wire names"
        );
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["max_output_tokens"], 256);
    }

    #[tokio::test]
    async fn decodes_text_function_calls_and_cached_usage() {
        let turn = collect(
            decode(chunks(vec![
                "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n",
                "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n",
                "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call_9\",\"name\":\"math_add\",\"arguments\":\"{\\\"a\\\":1}\"}}\n\n",
                "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1200,\"output_tokens\":7,\"input_tokens_details\":{\"cached_tokens\":1024}}}}\n\n",
            ])),
            None,
        )
        .await
        .expect("turn");
        assert_eq!(turn.text, "Hello");
        assert_eq!(turn.tool_calls[0].id, "call_9");
        assert_eq!(turn.tool_calls[0].arguments, serde_json::json!({ "a": 1 }));
        assert_eq!(turn.stop, Some(StopReason::ToolUse));
        assert_eq!(
            turn.usage.map(|u| (u.input_tokens, u.cached_input_tokens)),
            Some((1200, 1024))
        );
    }

    #[tokio::test]
    async fn an_incomplete_response_stops_at_the_cap_and_a_failure_is_an_error() {
        let turn = collect(
            decode(chunks(vec![
                "data: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n\n",
            ])),
            None,
        )
        .await
        .expect("turn");
        assert_eq!(turn.stop, Some(StopReason::MaxTokens));
        let events: Vec<_> = futures::StreamExt::collect(decode(chunks(vec![
            "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"overloaded\"}}}\n\n",
        ])))
        .await;
        assert!(
            matches!(events.first(), Some(Err(e)) if e.to_string().contains("overloaded")),
            "{events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Ok(ModelEvent::Stop { .. })))
        );
    }

    #[tokio::test]
    async fn a_stream_cut_before_the_response_completes_is_retryable() {
        let events: Vec<_> = futures::StreamExt::collect(decode(chunks(vec![
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"The answer is\"}\n\n",
        ])))
        .await;
        assert!(
            matches!(events.last(), Some(Err(e)) if e.is_retryable()),
            "a truncated answer is a retryable failure, not a finished turn: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Ok(ModelEvent::Stop { .. }))),
            "{events:?}"
        );
    }
}
