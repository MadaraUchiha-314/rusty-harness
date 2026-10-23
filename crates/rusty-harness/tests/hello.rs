//! Integration test: the public API as an external consumer sees it.
#![cfg(feature = "rt-tokio")]

use rusty_harness::runtime::TokioRuntime;
use rusty_harness::{Runtime, hello};

#[tokio::test]
async fn hello_from_a_spawned_task() {
    let (tx, rx) = tokio::sync::oneshot::channel();
    TokioRuntime::current().spawn(async move {
        tx.send(hello().await).expect("receiver alive");
    });
    assert_eq!(rx.await.expect("task ran"), "Hello from rusty-harness!");
}
