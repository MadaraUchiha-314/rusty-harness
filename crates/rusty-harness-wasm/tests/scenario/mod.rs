//! The task the node and browser suites both run against a mocked host (T12).

use js_sys::{Function, Reflect};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;

/// A host whose `fetch` answers every model call with one scripted OpenAI stream.
const HOST: &str = r#"
const sse =
  'data: {"choices":[{"delta":{"content":"Hello from "}}]}\n\n' +
  'data: {"choices":[{"delta":{"content":"the browser."},"finish_reason":"stop"}]}\n\n' +
  'data: [DONE]\n\n';
const host = {
  calls: [],
  allowOrigins: ["https://api.openai.com"],
  fetch: async (url, init) => {
    host.calls.push({ url, auth: init.headers.authorization });
    return new Response(sse, { status: 200, headers: { "content-type": "text/event-stream" } });
  },
  secret: (reference) =>
    reference === "env:OPENAI_API_KEY" ? "sk-test" : Promise.reject(new Error("unknown")),
};
return host;
"#;

const CONFIG: &str = r#"
return { models: { default: { provider: "openai", model: "gpt-6.1", api_key: "env:OPENAI_API_KEY" } } };
"#;

const REQUEST: &str = r#"
return { message: { messageId: "m1", taskId: "t1", contextId: "c1", role: "ROLE_USER", parts: [{ text: "say hello" }] } };
"#;

fn eval(body: &str) -> JsValue {
    Function::new_no_args(body)
        .call0(&JsValue::NULL)
        .expect("the fixture evaluates")
}

fn json(v: &JsValue) -> String {
    js_sys::JSON::stringify(v)
        .ok()
        .and_then(|s| s.as_string())
        .unwrap_or_default()
}

/// Feature: WASM and browser support
/// Requirement: docs/specs/issue-3/requirements.md#R29
///
/// Scenario: The wasm build runs a task against a mocked host
///     Given a harness created from a config object and a page host with a mocked fetch
///     When a message is sent through sendMessage
///     Then the streamed events carry the model's reply and the task completes,
///     And the model call went through the host's fetch with the page-resolved key
pub(crate) async fn the_wasm_build_runs_a_task_against_a_mocked_host() {
    let host = eval(HOST);
    let harness =
        rusty_harness_wasm::Harness::create(eval(CONFIG), Some(host.clone().unchecked_into()))
            .await
            .unwrap_or_else(|e| panic!("builds: {:?}", JsValue::from(e)));
    let events = harness
        .send_message(eval(REQUEST))
        .unwrap_or_else(|e| panic!("accepted: {:?}", JsValue::from(e)));
    let next: Function = Reflect::get(&events, &"next".into()).unwrap().into();
    let mut seen = Vec::new();
    loop {
        let step = JsFuture::from(js_sys::Promise::from(next.call0(&events).unwrap()))
            .await
            .unwrap_or_else(|e| panic!("an event: {e:?}"));
        if Reflect::get(&step, &"done".into()).unwrap().as_bool() == Some(true) {
            break;
        }
        seen.push(json(&Reflect::get(&step, &"value".into()).unwrap()));
    }
    let all = seen.join("\n");
    assert!(all.contains("Hello from the browser."), "{all}");
    let task = harness.get_task("t1".into()).await.expect("the task");
    assert!(
        json(&task).contains("TASK_STATE_COMPLETED"),
        "{}",
        json(&task)
    );
    let calls = json(&Reflect::get(&host, &"calls".into()).unwrap());
    assert!(
        calls.contains("https://api.openai.com/v1/chat/completions"),
        "{calls}"
    );
    assert!(calls.contains("Bearer sk-test"), "{calls}");
}

/// Scenario: A fetch outside the host's allowOrigins is refused
///     Given a host whose allowOrigins names only the model provider
///     When the configuration points the model somewhere else
///     Then the turn fails and the host's fetch is never called
pub(crate) async fn a_fetch_outside_allow_origins_is_refused() {
    let host = eval(HOST);
    let config = eval(
        r#"return { models: { default: { provider: "openai", model: "gpt-6.1", api_key: "env:OPENAI_API_KEY", base_url: "https://evil.example/v1" } } };"#,
    );
    let harness = rusty_harness_wasm::Harness::create(config, Some(host.clone().unchecked_into()))
        .await
        .unwrap_or_else(|e| panic!("builds: {:?}", JsValue::from(e)));
    let events = harness.send_message(eval(REQUEST)).expect("accepted");
    let next: Function = Reflect::get(&events, &"next".into()).unwrap().into();
    let mut all = String::new();
    loop {
        match JsFuture::from(js_sys::Promise::from(next.call0(&events).unwrap())).await {
            Ok(step) if Reflect::get(&step, &"done".into()).unwrap().as_bool() == Some(true) => {
                break;
            }
            Ok(step) => all.push_str(&json(&Reflect::get(&step, &"value".into()).unwrap())),
            Err(e) => {
                all.push_str(&format!("{e:?}"));
                break;
            }
        }
    }
    assert!(!all.contains("Hello from the browser."), "{all}");
    let calls = json(&Reflect::get(&host, &"calls".into()).unwrap());
    assert_eq!(calls, "[]", "fetch was never called");
}
