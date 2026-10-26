# T6 — snapshots

<!-- markdownlint-disable MD013 -->

Serialized outputs that must not drift. The snapshots themselves are committed under `crates/rusty-harness/tests/snapshots/` and `crates/rusty-harness-wasm/ts/`; this run compares against them.

## Snapshot tests

```sh
cargo nextest run -p rusty_harness --all-features -E 'test(snapshot_)'
```

Outcome: **passed** in 2s, 2026-10-25, at `0a97f5b`.

```text
Starting 17 tests across 30 binaries (230 tests skipped)
        PASS [   0.014s] ( 1/17) rusty_harness harness::models::llm::openai::responses::tests::snapshot_openai_responses_request_body
        PASS [   0.014s] ( 2/17) rusty_harness config::tests::snapshot_default_config_toml
        PASS [   0.015s] ( 3/17) rusty_harness harness::models::llm::anthropic::tests::snapshot_anthropic_request_body
        PASS [   0.021s] ( 4/17) rusty_harness config::tests::snapshot_config_schema
        PASS [   0.007s] ( 5/17) rusty_harness::assets snapshot_rendered_default_prompts
        PASS [   0.010s] ( 6/17) rusty_harness harness::models::llm::openai::tests::snapshot_openai_request_body
        PASS [   0.009s] ( 7/17) rusty_harness::renderer snapshot_a2ui_ask_payload
        PASS [   0.007s] ( 8/17) rusty_harness::speech snapshot_realtime_session_config
        PASS [   0.164s] ( 9/17) rusty_harness::server snapshot_agent_card
        PASS [   0.157s] (10/17) rusty_harness::tui tui_snapshot_a2ui
        PASS [   0.021s] (11/17) rusty_harness::tui tui_snapshot_chat
        PASS [   0.023s] (12/17) rusty_harness::tui tui_snapshot_done
        PASS [   0.019s] (13/17) rusty_harness::tui tui_snapshot_first
        PASS [   0.218s] (14/17) rusty_harness::contract_a2a_extensions snapshot_extension_payloads
        PASS [   0.027s] (15/17) rusty_harness::tui tui_snapshot_model
        PASS [   0.025s] (16/17) rusty_harness::tui tui_snapshot_voice
        PASS [   0.269s] (17/17) rusty_harness::cli snapshot_first_run_output
     Summary [   0.284s] 17 tests run: 17 passed, 230 skipped
```
