//! The `Host` built from JavaScript callbacks (R29.2, design.md § Host capabilities).
//!
//! Each capability is a page-supplied callback, defaulting to the platform's own: `fetch`,
//! `WebSocket`, `Date.now`, `crypto.getRandomValues` and `console`. Secrets have no
//! default: the page resolves a reference itself or the harness has none (abuse 11).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use rusty_harness::host::{
    Clock, Host, HostError, HttpClient, HttpRequest, HttpResponse, HttpStreamingResponse,
    LogRecord, LogSink, Rng, Secret, SecretError, SecretRef, SecretResolver, SpanRecord, WebSocket,
    WebSocketClient, WsMessage,
};
use rusty_harness::runtime::boxed_stream;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// A JavaScript value the harness holds behind its `Send + Sync` capability traits.
///
/// Without the `atomics` target feature `wasm32-unknown-unknown` has one thread and no
/// way to start another, so nothing can move it across threads.
#[derive(Clone, Debug)]
pub(crate) struct Local<T>(pub(crate) T);

// SAFETY: see the type's doc; the build refuses threaded wasm below.
unsafe impl<T> Send for Local<T> {}
// SAFETY: as above.
unsafe impl<T> Sync for Local<T> {}

#[cfg(target_feature = "atomics")]
compile_error!("the JS host assumes single-threaded wasm32 (no `atomics` target feature)");

fn js_error(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            Reflect::get(e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

fn callback(options: &JsValue, name: &str) -> Option<Function> {
    Reflect::get(options, &name.into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
}

fn global(path: &[&str]) -> Option<JsValue> {
    let mut v: JsValue = js_sys::global().into();
    for key in path {
        v = Reflect::get(&v, &(*key).into()).ok()?;
        if v.is_undefined() || v.is_null() {
            return None;
        }
    }
    Some(v)
}

/// Builds the host from the page's options object (`HostOptions` in TypeScript).
///
/// # Errors
///
/// When `allowOrigins` is not a list of origins.
pub(crate) fn from_options(options: &JsValue) -> Result<Host, JsError> {
    let options = if options.is_undefined() || options.is_null() {
        Object::new().into()
    } else {
        options.clone()
    };
    let mut builder = Host::builder();
    let fetch = callback(&options, "fetch").or_else(|| {
        let fetch = global(&["fetch"])?.dyn_into::<Function>().ok()?;
        // Unbound `fetch` throws "Illegal invocation" in browsers.
        Some(fetch.bind0(&js_sys::global()))
    });
    if let Some(fetch) = fetch {
        builder = builder.http(Arc::new(JsHttp {
            fetch: Local(fetch),
            allow: allow_origins(&options)?,
        }));
    }
    // The defaults below are Rust closures and bound built-ins, never functions compiled
    // from strings: a page whose CSP has no 'unsafe-eval' refuses those.
    let socket = callback(&options, "webSocket").or_else(|| {
        global(&["WebSocket"]).map(|_| {
            Closure::<dyn Fn(String) -> Result<JsValue, JsValue>>::new(|url: String| {
                web_sys::WebSocket::new(&url).map(JsValue::from)
            })
            .into_js_value()
            .unchecked_into::<Function>()
        })
    });
    if let Some(open) = socket {
        builder = builder.websocket(Arc::new(JsWebSockets { open: Local(open) }));
    }
    let now = callback(&options, "now")
        .or_else(|| global(&["Date", "now"]).and_then(|f| f.dyn_into::<Function>().ok()));
    if let Some(now) = now {
        builder = builder.clock(Arc::new(JsClock { now: Local(now) }));
    }
    let random = callback(&options, "randomBytes").or_else(|| {
        let crypto = global(&["crypto"])?;
        let fill = Reflect::get(&crypto, &"getRandomValues".into())
            .ok()?
            .dyn_into::<Function>()
            .ok()?;
        Some(fill.bind0(&crypto))
    });
    if let Some(fill) = random {
        builder = builder.rng(Arc::new(JsRng { fill: Local(fill) }));
    }
    if let Some(resolve) = callback(&options, "secret") {
        builder = builder.secrets(Arc::new(JsSecrets {
            resolve: Local(resolve),
        }));
    }
    let log = callback(&options, "log").unwrap_or_else(|| {
        Closure::<dyn Fn(JsValue)>::new(|record: JsValue| {
            let field = |k: &str| Reflect::get(&record, &k.into()).unwrap_or(JsValue::UNDEFINED);
            let line = JsValue::from(format!(
                "[{}] {}",
                field("target").as_string().unwrap_or_default(),
                field("message").as_string().unwrap_or_default()
            ));
            let fields = field("fields");
            match field("level").as_string().as_deref() {
                Some("error") => web_sys::console::error_2(&line, &fields),
                Some("warn") => web_sys::console::warn_2(&line, &fields),
                _ => web_sys::console::log_2(&line, &fields),
            }
        })
        .into_js_value()
        .unchecked_into::<Function>()
    });
    builder = builder.log_sink(Arc::new(JsLog { emit: Local(log) }));
    Ok(builder.build())
}

/// `allowOrigins`: when present, the only origins `fetch` may reach (R29.2).
fn allow_origins(options: &JsValue) -> Result<Option<Vec<String>>, JsError> {
    let v = Reflect::get(options, &"allowOrigins".into()).unwrap_or(JsValue::UNDEFINED);
    if v.is_undefined() || v.is_null() {
        return Ok(None);
    }
    let list: Array = v
        .dyn_into()
        .map_err(|_| JsError::new("allowOrigins must be a list of origins"))?;
    let mut out = Vec::new();
    for item in list.iter() {
        let text = item
            .as_string()
            .ok_or_else(|| JsError::new("allowOrigins must be a list of origins"))?;
        let url = url::Url::parse(&text)
            .map_err(|_| JsError::new(&format!("{text} is not an origin")))?;
        out.push(url.origin().ascii_serialization());
    }
    Ok(Some(out))
}

struct JsHttp {
    fetch: Local<Function>,
    allow: Option<Vec<String>>,
}

/// Aborts its request when dropped: a caller that stops waiting for a response, or stops
/// reading a streamed body (a cancelled task mid-reply), ends the download instead of
/// leaving the browser, and the provider, working until the server finishes. Aborting
/// a request that already completed does nothing.
struct Abort(Local<Option<web_sys::AbortController>>);

impl Drop for Abort {
    fn drop(&mut self) {
        if let Some(controller) = &self.0.0 {
            controller.abort();
        }
    }
}

impl JsHttp {
    async fn response(
        &self,
        request: HttpRequest,
    ) -> Result<(web_sys::Response, Abort), HostError> {
        if let Some(allow) = &self.allow {
            let origin = request.url.origin().ascii_serialization();
            if !allow.contains(&origin) {
                return Err(HostError::Denied(format!(
                    "{origin} is not in the host's allowOrigins"
                )));
            }
        }
        let headers = Object::new();
        for (name, value) in &request.headers {
            let value = value.to_str().map_err(|_| HostError::Http {
                status: None,
                message: format!("header {name} is not text"),
            })?;
            Reflect::set(&headers, &name.as_str().into(), &value.into())
                .map_err(|e| HostError::Io(js_error(&e)))?;
        }
        let init = Object::new();
        let set = |k: &str, v: &JsValue| {
            Reflect::set(&init, &k.into(), v).map_err(|e| HostError::Io(js_error(&e)))
        };
        set("method", &request.method.as_str().into())?;
        set("headers", &headers)?;
        let abort = Abort(Local(web_sys::AbortController::new().ok()));
        if let Some(controller) = &abort.0.0 {
            set("signal", &controller.signal())?;
        }
        // Only the first URL is checked against allowOrigins: a redirect followed by
        // fetch would re-send the body and the key headers to an origin nobody allowed,
        // so with a list, a redirect fails the request instead (fail closed).
        if self.allow.is_some() {
            set("redirect", &"error".into())?;
        }
        if !request.body.is_empty() {
            set("body", &Uint8Array::from(request.body.as_ref()).into())?;
        }
        let promise = self
            .fetch
            .0
            .call2(&JsValue::NULL, &request.url.as_str().into(), &init)
            .map_err(|e| http_error(&e))?;
        let response = JsFuture::from(js_sys::Promise::resolve(&promise))
            .await
            .map_err(|e| http_error(&e))?;
        Ok((response.unchecked_into(), abort))
    }
}

fn http_error(e: &JsValue) -> HostError {
    HostError::Http {
        status: None,
        message: js_error(e),
    }
}

fn head(response: &web_sys::Response) -> Result<(http::StatusCode, http::HeaderMap), HostError> {
    let status = http::StatusCode::from_u16(response.status()).map_err(|_| HostError::Http {
        status: Some(response.status()),
        message: "status out of range".into(),
    })?;
    let mut headers = http::HeaderMap::new();
    let entries = js_sys::try_iter(&response.headers())
        .ok()
        .flatten()
        .into_iter()
        .flatten();
    for entry in entries.flatten() {
        let pair: Array = entry.unchecked_into();
        if let (Some(k), Some(v)) = (pair.get(0).as_string(), pair.get(1).as_string())
            && let (Ok(k), Ok(v)) = (
                http::HeaderName::from_bytes(k.as_bytes()),
                http::HeaderValue::from_str(&v),
            )
        {
            headers.append(k, v);
        }
    }
    Ok((status, headers))
}

rusty_harness::harness_async_trait! {
    impl HttpClient for JsHttp {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HostError> {
            let (response, _abort) = self.response(request).await?;
            let (status, headers) = head(&response)?;
            let buffer = JsFuture::from(response.array_buffer().map_err(|e| http_error(&e))?)
                .await
                .map_err(|e| http_error(&e))?;
            Ok(HttpResponse { status, headers, body: Bytes::from(Uint8Array::new(&buffer).to_vec()) })
        }

        async fn send_streaming(&self, request: HttpRequest) -> Result<HttpStreamingResponse, HostError> {
            let (response, abort) = self.response(request).await?;
            let (status, headers) = head(&response)?;
            let Some(body) = response.body() else {
                return Ok(HttpStreamingResponse { status, headers, body: boxed_stream(futures::stream::empty()) });
            };
            let reader: web_sys::ReadableStreamDefaultReader = body.get_reader().unchecked_into();
            let reader = Local(reader);
            // The abort guard lives as long as the body stream: dropped with it.
            let chunks = futures::stream::unfold((Some(reader), abort), |(reader, abort)| async move {
                let reader = reader?;
                let read = match JsFuture::from(reader.0.read()).await {
                    Ok(read) => read,
                    Err(e) => return Some((Err(http_error(&e)), (None, abort))),
                };
                let done = Reflect::get(&read, &"done".into()).ok().and_then(|d| d.as_bool()).unwrap_or(true);
                if done {
                    return None;
                }
                let value = Reflect::get(&read, &"value".into()).unwrap_or(JsValue::UNDEFINED);
                Some((Ok(Bytes::from(Uint8Array::new(&value).to_vec())), (Some(reader), abort)))
            });
            Ok(HttpStreamingResponse { status, headers, body: boxed_stream(chunks) })
        }
    }
}

struct JsWebSockets {
    open: Local<Function>,
}

struct JsSocket {
    socket: Local<web_sys::WebSocket>,
    frames: mpsc::UnboundedReceiver<Result<WsMessage, HostError>>,
    // Kept alive for as long as the socket is: the handlers the browser calls.
    _handlers: Local<Vec<JsValue>>,
}

impl Drop for JsSocket {
    /// A socket nobody holds is closed, not left open (and billed) until the page goes.
    fn drop(&mut self) {
        let _ = self.socket.0.close();
    }
}

rusty_harness::harness_async_trait! {
    impl WebSocketClient for JsWebSockets {
        async fn connect(&self, url: url::Url, headers: http::HeaderMap) -> Result<Box<dyn WebSocket>, HostError> {
            if !headers.is_empty() {
                // A browser cannot set headers on a WebSocket; a credential header in
                // particular has no safe place to go (abuse 11).
                return Err(HostError::Denied("the browser cannot send headers on a WebSocket".into()));
            }
            let socket: web_sys::WebSocket = self
                .open
                .0
                .call1(&JsValue::NULL, &url.as_str().into())
                .map_err(|e| HostError::Io(js_error(&e)))?
                .unchecked_into();
            socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
            let (tx, frames) = mpsc::unbounded();
            let (opened_tx, opened) = oneshot::channel::<Result<(), HostError>>();
            let opened_tx = Rc::new(RefCell::new(Some(opened_tx)));
            let on_open = {
                let opened_tx = Rc::clone(&opened_tx);
                Closure::<dyn FnMut()>::new(move || {
                    if let Some(tx) = opened_tx.borrow_mut().take() {
                        let _ = tx.send(Ok(()));
                    }
                })
            };
            let on_message = {
                let tx = tx.clone();
                Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
                    let data = e.data();
                    let frame = match data.as_string() {
                        Some(text) => WsMessage::Text(text),
                        None => WsMessage::Binary(Bytes::from(Uint8Array::new(&data).to_vec())),
                    };
                    let _ = tx.unbounded_send(Ok(frame));
                })
            };
            let on_close = {
                let tx = tx.clone();
                let opened_tx = Rc::clone(&opened_tx);
                Closure::<dyn FnMut()>::new(move || {
                    if let Some(open) = opened_tx.borrow_mut().take() {
                        let _ = open.send(Err(HostError::Io("the WebSocket closed before it opened".into())));
                    }
                    let _ = tx.unbounded_send(Ok(WsMessage::Close));
                    tx.close_channel();
                })
            };
            let on_error = {
                Closure::<dyn FnMut()>::new(move || {
                    if let Some(open) = opened_tx.borrow_mut().take() {
                        let _ = open.send(Err(HostError::Io("the WebSocket failed to open".into())));
                    }
                    let _ = tx.unbounded_send(Err(HostError::Io("WebSocket error".into())));
                })
            };
            socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
            socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            let handlers = vec![
                on_open.into_js_value(),
                on_message.into_js_value(),
                on_close.into_js_value(),
                on_error.into_js_value(),
            ];
            // Built before the handshake so that a connect dropped mid-way closes it too.
            let socket = JsSocket { socket: Local(socket), frames, _handlers: Local(handlers) };
            opened.await.map_err(|_| HostError::Io("the WebSocket went away".into()))??;
            Ok(Box::new(socket))
        }
    }
}

rusty_harness::harness_async_trait! {
    impl WebSocket for JsSocket {
        async fn send(&mut self, message: WsMessage) -> Result<(), HostError> {
            let sent = match message {
                WsMessage::Text(text) => self.socket.0.send_with_str(&text),
                WsMessage::Binary(bytes) => self.socket.0.send_with_u8_array(&bytes),
                WsMessage::Close => self.socket.0.close(),
                _ => Ok(()),
            };
            sent.map_err(|e| HostError::Io(js_error(&e)))
        }

        async fn next(&mut self) -> Option<Result<WsMessage, HostError>> {
            self.frames.next().await
        }

        async fn close(&mut self) -> Result<(), HostError> {
            self.socket.0.close().map_err(|e| HostError::Io(js_error(&e)))
        }
    }
}

struct JsClock {
    now: Local<Function>,
}

impl Clock for JsClock {
    fn now(&self) -> DateTime<Utc> {
        let ms = self
            .now
            .0
            .call0(&JsValue::NULL)
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or_default();
        #[allow(clippy::cast_possible_truncation)] // milliseconds since 1970 fit in i64
        DateTime::from_timestamp_millis(ms as i64).unwrap_or_default()
    }
}

struct JsRng {
    fill: Local<Function>,
}

impl Rng for JsRng {
    fn fill(&self, buf: &mut [u8]) {
        let array = Uint8Array::new_with_length(u32::try_from(buf.len()).unwrap_or(u32::MAX));
        let _ = self.fill.0.call1(&JsValue::NULL, &array);
        array.copy_to(buf);
    }
}

struct JsSecrets {
    resolve: Local<Function>,
}

rusty_harness::harness_async_trait! {
    impl SecretResolver for JsSecrets {
        async fn resolve(&self, reference: &SecretRef) -> Result<Secret<String>, SecretError> {
            let text = reference.to_string();
            let fail = |reason: String| SecretError::Unresolvable { reference: text.clone(), reason };
            let value = self
                .resolve
                .0
                .call1(&JsValue::NULL, &text.as_str().into())
                .map_err(|e| fail(js_error(&e)))?;
            let value = JsFuture::from(js_sys::Promise::resolve(&value))
                .await
                .map_err(|e| fail(js_error(&e)))?;
            value
                .as_string()
                .filter(|s| !s.is_empty())
                .map(Secret::new)
                .ok_or_else(|| fail("the page's secret callback returned no text".into()))
        }
    }
}

struct JsLog {
    emit: Local<Function>,
}

fn fields(pairs: &[(String, String)]) -> Object {
    let out = Object::new();
    for (k, v) in pairs {
        let _ = Reflect::set(&out, &k.as_str().into(), &v.as_str().into());
    }
    out
}

impl LogSink for JsLog {
    fn emit(&self, record: LogRecord) {
        let r = Object::new();
        let _ = Reflect::set(&r, &"level".into(), &record.level.into());
        let _ = Reflect::set(&r, &"target".into(), &record.target.into());
        let _ = Reflect::set(&r, &"message".into(), &record.message.into());
        let _ = Reflect::set(&r, &"fields".into(), &fields(&record.fields));
        let _ = self.emit.0.call1(&JsValue::NULL, &r);
    }

    fn export_span(&self, span: SpanRecord) {
        let r = Object::new();
        let _ = Reflect::set(&r, &"level".into(), &"trace".into());
        let _ = Reflect::set(&r, &"target".into(), &"span".into());
        let _ = Reflect::set(
            &r,
            &"message".into(),
            &format!("{} {}µs", span.name, span.duration_us).into(),
        );
        let _ = Reflect::set(&r, &"fields".into(), &fields(&span.attributes));
        let _ = self.emit.0.call1(&JsValue::NULL, &r);
    }
}

#[cfg(test)]
mod tests {
    use super::{JsHttp, Local};
    use futures::StreamExt as _;
    use js_sys::{Function, Reflect};
    use rusty_harness::host::{HttpClient as _, HttpRequest};
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// A streamed body the caller stops reading (a cancelled task mid-reply) aborts the
    /// request: without it the browser keeps downloading, and the provider keeps
    /// generating, until the server finishes on its own.
    #[wasm_bindgen_test(async)]
    async fn dropping_a_streamed_body_aborts_the_request() {
        // A page fetch whose body sends one chunk and never ends; it keeps the signal.
        let seen = js_sys::Object::new();
        let fetch = Function::new_with_args(
            "seen",
            "return async (url, init) => { seen.signal = init.signal; \
             const body = new ReadableStream({ start(c) { c.enqueue(new Uint8Array([104, 105])); } }); \
             return new Response(body, { status: 200 }); }",
        )
        .call1(&JsValue::NULL, &seen)
        .expect("the fixture evaluates")
        .into();
        let http = JsHttp {
            fetch: Local(fetch),
            allow: None,
        };
        let request = HttpRequest::new(
            http::Method::POST,
            "https://api.example/v1/responses".parse().unwrap(),
        );
        let mut response = http.send_streaming(request).await.expect("a response");
        let first = response.body.next().await.expect("a chunk").expect("bytes");
        assert_eq!(first.as_ref(), b"hi");
        let signal = Reflect::get(&seen, &"signal".into()).expect("the signal");
        assert_eq!(
            Reflect::get(&signal, &"aborted".into()).unwrap().as_bool(),
            Some(false),
            "still running while it is read"
        );
        drop(response);
        assert_eq!(
            Reflect::get(&signal, &"aborted".into()).unwrap().as_bool(),
            Some(true),
            "dropping the body aborts the request"
        );
    }
}
