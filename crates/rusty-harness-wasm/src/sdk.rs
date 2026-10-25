//! `Harness`: the SDK door from JavaScript (R29.4, R30.3).

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::lock::Mutex;
use js_sys::{Function, Object, Promise, Reflect};
use rusty_harness::config::Config;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

/// The TypeScript the generated declarations cannot spell: the configuration (from the
/// JSON schema), the A2A 1.0 JSON shapes and the host's callbacks.
#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = concat!(
    include_str!("../ts/config.d.ts"),
    "\n",
    include_str!("../ts/a2a.d.ts"),
    "\n",
    include_str!("../ts/host.d.ts"),
);

type Item = Result<a2a::StreamResponse, a2a::A2AError>;

#[wasm_bindgen]
extern "C" {
    /// The page's host callbacks (`HostOptions` in TypeScript).
    #[wasm_bindgen(typescript_type = "HostOptions")]
    pub type HostOptions;
}

/// A harness running in this page: the loop, the registry and `none` durability, with
/// every platform capability supplied by the host (R29.1).
#[wasm_bindgen]
pub struct Harness {
    inner: Rc<rusty_harness::Harness>,
}

impl core::fmt::Debug for Harness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Harness").finish_non_exhaustive()
    }
}

fn parse<T: serde::de::DeserializeOwned>(value: &JsValue, what: &str) -> Result<T, JsError> {
    let text = js_sys::JSON::stringify(value)
        .ok()
        .and_then(|s| s.as_string())
        .ok_or_else(|| JsError::new(&format!("{what} is not JSON")))?;
    serde_json::from_str(&text).map_err(|e| JsError::new(&format!("{what}: {e}")))
}

fn to_js<T: serde::Serialize>(value: &T) -> JsValue {
    serde_json::to_string(value)
        .ok()
        .and_then(|s| js_sys::JSON::parse(&s).ok())
        .unwrap_or(JsValue::UNDEFINED)
}

/// An A2A error as a JavaScript `Error` carrying its JSON-RPC `code`.
fn a2a_error(e: &a2a::A2AError) -> JsValue {
    let error = js_sys::Error::new(&e.to_string());
    let _ = Reflect::set(&error, &"code".into(), &e.code.into());
    error.into()
}

#[wasm_bindgen]
impl Harness {
    /// Builds a harness from `config` (an object of the configuration's shape, or TOML
    /// text) and the page's `host`; fails closed when a capability the configuration
    /// needs is missing (R29.3).
    ///
    /// # Errors
    ///
    /// An invalid configuration, or a harness that cannot be built.
    pub async fn create(
        #[wasm_bindgen(unchecked_param_type = "Config | string")] config: JsValue,
        host: Option<HostOptions>,
    ) -> Result<Harness, JsError> {
        let config: Config = match config.as_string() {
            Some(toml) => Config::from_layers(Some(&toml), Vec::new(), None)
                .map_err(|e| JsError::new(&e.to_string()))?,
            None => parse(&config, "config")?,
        };
        let host = crate::host::from_options(&host.map_or(JsValue::UNDEFINED, Into::into))?;
        let inner = rusty_harness::HarnessBuilder::new(config)
            .host(host)
            .build()
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Harness {
            inner: Rc::new(inner),
        })
    }

    /// Sends a message and streams the turn back as A2A events.
    ///
    /// # Errors
    ///
    /// A request that is not a `SendMessageRequest`.
    #[wasm_bindgen(js_name = sendMessage, unchecked_return_type = "AsyncIterable<StreamResponse>")]
    pub fn send_message(
        &self,
        #[wasm_bindgen(unchecked_param_type = "SendMessageRequest")] request: JsValue,
    ) -> Result<JsValue, JsError> {
        let request: a2a::SendMessageRequest = parse(&request, "request")?;
        let inner = Rc::clone(&self.inner);
        Ok(iterable(
            move |tx: mpsc::UnboundedSender<Item>| async move {
                let mut events = inner.send_message(request, None);
                while let Some(event) = events.next().await {
                    // The turn runs to its end even when the page stops reading, as it does
                    // when a client disconnects from the server.
                    let _ = tx.unbounded_send(event);
                }
            },
        ))
    }

    /// The task's log so far, then its live events until the turn ends.
    #[wasm_bindgen(unchecked_return_type = "AsyncIterable<StreamResponse>")]
    pub fn subscribe(&self, #[wasm_bindgen(js_name = taskId)] task_id: String) -> JsValue {
        let inner = Rc::clone(&self.inner);
        iterable(move |tx: mpsc::UnboundedSender<Item>| async move {
            match inner.subscribe(&task_id, None).await {
                Ok(mut events) => {
                    while let Some(event) = events.next().await {
                        let _ = tx.unbounded_send(event);
                    }
                }
                Err(e) => {
                    let _ = tx.unbounded_send(Err(e));
                }
            }
        })
    }

    /// Cancels a task.
    ///
    /// # Errors
    ///
    /// An `Error` with the A2A `code` (`TaskNotFound`, not cancelable).
    #[wasm_bindgen(js_name = cancelTask, unchecked_return_type = "Task")]
    pub async fn cancel_task(
        &self,
        #[wasm_bindgen(js_name = taskId)] task_id: String,
    ) -> Result<JsValue, JsValue> {
        self.inner
            .cancel_task(&task_id, None)
            .await
            .map(|t| to_js(&t))
            .map_err(|e| a2a_error(&e))
    }

    /// The task as it stands.
    ///
    /// # Errors
    ///
    /// An `Error` with the A2A `code` (`TaskNotFound`).
    #[wasm_bindgen(js_name = getTask, unchecked_return_type = "Task")]
    pub async fn get_task(
        &self,
        #[wasm_bindgen(js_name = taskId)] task_id: String,
    ) -> Result<JsValue, JsValue> {
        self.inner
            .get_task(&task_id, None)
            .await
            .map(|t| to_js(&t))
            .map_err(|e| a2a_error(&e))
    }
}

/// An `AsyncIterable` over what `produce` sends; `produce` runs on the page's event loop
/// from the first call, reads or not.
fn iterable<F>(produce: impl FnOnce(mpsc::UnboundedSender<Item>) -> F) -> JsValue
where
    F: Future<Output = ()> + 'static,
{
    let (tx, rx) = mpsc::unbounded();
    wasm_bindgen_futures::spawn_local(produce(tx));
    let rx = Rc::new(Mutex::new(Some(rx)));
    let next = {
        let rx = Rc::clone(&rx);
        Closure::<dyn FnMut() -> Promise>::new(move || {
            let rx = Rc::clone(&rx);
            future_to_promise(async move {
                let mut guard = rx.lock().await;
                let item = match guard.as_mut() {
                    Some(rx) => rx.next().await,
                    None => None,
                };
                if item.is_none() {
                    *guard = None;
                }
                match item {
                    Some(Ok(event)) => Ok(result(false, &to_js(&event))),
                    Some(Err(e)) => {
                        *guard = None; // an error ends the stream
                        Err(a2a_error(&e))
                    }
                    None => Ok(result(true, &JsValue::UNDEFINED)),
                }
            })
        })
    };
    let finish = Closure::<dyn FnMut() -> Promise>::new(move || {
        let rx = Rc::clone(&rx);
        future_to_promise(async move {
            *rx.lock().await = None;
            Ok(result(true, &JsValue::UNDEFINED))
        })
    });
    let iterator = Object::new();
    let _ = Reflect::set(&iterator, &"next".into(), &next.into_js_value());
    let _ = Reflect::set(&iterator, &"return".into(), &finish.into_js_value());
    let _ = Reflect::set(
        &iterator,
        &js_sys::Symbol::async_iterator(),
        &Function::new_no_args("return this"),
    );
    iterator.into()
}

fn result(done: bool, value: &JsValue) -> JsValue {
    let r = Object::new();
    let _ = Reflect::set(&r, &"done".into(), &done.into());
    let _ = Reflect::set(&r, &"value".into(), value);
    r.into()
}
