//! The SDK under a strict Content-Security-Policy (no `'unsafe-eval'`): Node started with
//! `--disallow-code-generation-from-strings` refuses `eval` and `new Function` as such a
//! page does. CI runs this file with that flag:
//!
//! ```sh
//! NODE_OPTIONS=--disallow-code-generation-from-strings wasm-pack test --node --test csp
//! ```
//!
//! Without the flag `eval` works and the test proves nothing, so it returns early.
#![cfg(target_arch = "wasm32")]

use js_sys::{Function, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

// The fixtures are a module, not strings evaluated at run time.
#[wasm_bindgen(inline_js = r#"
export function host() {
  const sse =
    'data: {"type":"response.output_text.delta","delta":"no eval needed"}\n\n' +
    'data: {"type":"response.completed","response":{"usage":{"input_tokens":3,"output_tokens":3}}}\n\n';
  return {
    allowOrigins: ["https://api.openai.com"],
    fetch: async () =>
      new Response(sse, { status: 200, headers: { "content-type": "text/event-stream" } }),
    secret: async () => "sk-test",
  };
}
export function config() {
  return { models: { default: { provider: "openai", model: "gpt-6.1", api_key: "env:K" } } };
}
export function request() {
  return { message: { messageId: "m1", taskId: "t1", contextId: "c1", role: "ROLE_USER", parts: [{ text: "hi" }] } };
}
export function iterate(iterable) {
  return iterable[Symbol.asyncIterator]();
}
"#)]
extern "C" {
    fn host() -> JsValue;
    fn config() -> JsValue;
    fn request() -> JsValue;
    fn iterate(iterable: &JsValue) -> JsValue;
}

/// Feature: WASM and browser support
/// Requirement: docs/specs/issue-3/requirements.md#R29
///
/// Scenario: The SDK runs a turn where code generation from strings is refused
///     Given a host with only fetch, a secret and allowOrigins (every other capability defaulted)
///     When a harness is created and a message's events are read with for-await
///     Then the reply arrives, and nothing called eval or new Function
#[wasm_bindgen_test(async)]
async fn the_sdk_runs_without_code_generation_from_strings() {
    if js_sys::eval("1").is_ok() {
        // Not under the flag: nothing here would catch a regression.
        return;
    }
    let harness = rusty_harness_wasm::Harness::create(config(), Some(host().unchecked_into()))
        .await
        .unwrap_or_else(|e| panic!("builds without eval: {:?}", JsValue::from(e)));
    let events = harness
        .send_message(request())
        .unwrap_or_else(|e| panic!("accepted: {:?}", JsValue::from(e)));
    let iterator = iterate(&events);
    let next: Function = Reflect::get(&iterator, &"next".into()).unwrap().into();
    let mut all = String::new();
    loop {
        let step = JsFuture::from(js_sys::Promise::from(next.call0(&iterator).unwrap()))
            .await
            .unwrap_or_else(|e| panic!("an event: {e:?}"));
        if Reflect::get(&step, &"done".into()).unwrap().as_bool() == Some(true) {
            break;
        }
        all.push_str(
            &js_sys::JSON::stringify(&Reflect::get(&step, &"value".into()).unwrap())
                .ok()
                .and_then(|s| s.as_string())
                .unwrap_or_default(),
        );
    }
    assert!(all.contains("no eval needed"), "{all}");
}
