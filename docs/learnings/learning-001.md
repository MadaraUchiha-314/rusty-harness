# Learning 001: an end-to-end canary finds liveness bugs, not only leaks

- **Date:** 2026-10-25
- **Source:** system-feedback
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3) (M3, task 26)

## What happened

The second leg of the credential canary ran the real stack (the binary's builder, a
provider over HTTP, the A2A server over TCP, the file store on disk) and made the
provider refuse the second call with a `401` whose body echoed the key. The test hung:
a non-retryable model error propagated out of the loop as an infrastructure failure,
in-process durable execution retried it five times with backoff and then gave up
*without touching the task*, so the task stayed `working` and the caller's stream never
ended. Every unit test of the loop had modelled failures as retryable or as vetoes.

## Learning

A scenario that drives the whole stack with a hostile dependency finds bugs that
per-module tests cannot: the gap was between two correct components (the loop's error
propagation and the executor's retry policy). The canary's value was not only the
redaction it asserted but the realistic failure it injected on the way.

## Action

- The loop fails a task with a typed error on its status (`kind: model`) for a
  non-retryable provider error (R9.6); durable execution fails on the record what it
  gives up on; provider clients scrub the credential from an echoing error body.
- When a testing plan has an abuse scenario, inject the dependency's failure in it and
  assert liveness (the request ends) alongside the security property.
