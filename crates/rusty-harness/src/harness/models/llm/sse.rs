//! Server-sent events over a byte stream, as both providers stream their answers.

use std::collections::VecDeque;

use bytes::Bytes;
use futures::StreamExt;

use crate::host::{ByteStream, HostError};
use crate::runtime::HarnessStream;

/// One SSE event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field, when present.
    pub event: Option<String>,
    /// The joined `data:` lines.
    pub data: String,
}

struct State {
    bytes: ByteStream,
    buf: Vec<u8>,
    queue: VecDeque<SseEvent>,
    finished: bool,
}

/// Splits a byte stream into SSE events; a `data: [DONE]` sentinel ends the stream.
pub fn events(bytes: ByteStream) -> HarnessStream<'static, Result<SseEvent, HostError>> {
    let state = State {
        bytes,
        buf: Vec::new(),
        queue: VecDeque::new(),
        finished: false,
    };
    Box::pin(futures::stream::unfold(state, |mut st| async move {
        loop {
            if let Some(event) = st.queue.pop_front() {
                return Some((Ok(event), st));
            }
            if st.finished {
                return None;
            }
            match st.bytes.next().await {
                Some(Ok(chunk)) => {
                    st.buf.extend_from_slice(&chunk);
                    let (events, rest, done) = parse(&st.buf);
                    st.buf = rest;
                    st.finished = done;
                    st.queue.extend(events);
                }
                Some(Err(err)) => {
                    st.finished = true;
                    return Some((Err(err), st));
                }
                None => {
                    let tail = std::mem::take(&mut st.buf);
                    st.finished = true;
                    st.queue.extend(parse_tail(&tail));
                }
            }
        }
    }))
}

/// Parses complete events (terminated by a blank line) out of `buf`, returning the
/// events, the unparsed remainder and whether `[DONE]` was seen.
fn parse(buf: &[u8]) -> (Vec<SseEvent>, Vec<u8>, bool) {
    let text = String::from_utf8_lossy(buf).into_owned();
    let mut events = Vec::new();
    let mut consumed = 0;
    let mut done = false;
    while let Some(end) = text[consumed..].find("\n\n") {
        let block = &text[consumed..consumed + end];
        consumed += end + 2;
        if let Some(event) = parse_block(block) {
            if event.data.trim() == "[DONE]" {
                done = true;
                break;
            }
            events.push(event);
        }
    }
    (events, buf[consumed.min(buf.len())..].to_vec(), done)
}

fn parse_tail(buf: &[u8]) -> Vec<SseEvent> {
    let text = String::from_utf8_lossy(buf);
    text.split("\n\n")
        .filter_map(parse_block)
        .filter(|e| e.data.trim() != "[DONE]")
        .collect()
}

fn parse_block(block: &str) -> Option<SseEvent> {
    let mut event = SseEvent::default();
    let mut data = Vec::new();
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event.event = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest).to_owned());
        }
    }
    if data.is_empty() {
        return None;
    }
    event.data = data.join("\n");
    Some(event)
}

/// A byte stream from in-memory chunks, for tests.
pub fn chunks(parts: Vec<&'static str>) -> ByteStream {
    Box::pin(futures::stream::iter(
        parts
            .into_iter()
            .map(|p| Ok(Bytes::from_static(p.as_bytes()))),
    ))
}

#[cfg(test)]
mod tests {
    use super::{SseEvent, chunks, events};
    use futures::StreamExt;

    #[tokio::test]
    async fn events_are_reassembled_across_chunks_and_done_ends_the_stream() {
        let stream = events(chunks(vec![
            "event: a\ndata: {\"x\":",
            "1}\n\ndata: two\ndata: lines\n\n",
            "data: [DONE]\n\ndata: after\n\n",
        ]));
        let got: Vec<SseEvent> = stream.map(|e| e.expect("ok")).collect().await;
        assert_eq!(
            got,
            [
                SseEvent {
                    event: Some("a".into()),
                    data: "{\"x\":1}".into()
                },
                SseEvent {
                    event: None,
                    data: "two\nlines".into()
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_trailing_event_without_a_blank_line_is_still_delivered() {
        let got: Vec<SseEvent> = events(chunks(vec!["data: last"]))
            .map(|e| e.expect("ok"))
            .collect()
            .await;
        assert_eq!(
            got,
            [SseEvent {
                event: None,
                data: "last".into()
            }]
        );
    }
}
