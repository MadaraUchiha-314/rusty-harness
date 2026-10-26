# T8 — security and abuse-case evidence

<!-- markdownlint-disable MD013 -->

Run 2026-10-25 on `loop/issue-3-m7-ship` (task 48).

## The abuse run

`cargo nextest run --workspace --all-features -E 'test(abuse_)'`: **19 passed**, 0 failed.

| # | Abuse case (design.md § Security design) | Test(s) | Result |
|---|---|---|---|
| 1 | request without credentials | `server_auth::abuse_unauthenticated_send_message_is_rejected_before_any_task_is_created` | pass |
| 2 | caller addresses another's task | `server_auth::abuse_non_participant_gets_task_not_found_and_empty_list_tasks`; `inbox_route::abuse_inbox_post_from_a_non_participant_is_refused_and_a_participants_is_applied` | pass |
| 3 | injected instructions in content | `context_window::tests::abuse_injected_instructions_in_tool_result_change_no_policy_config_or_role`; `commands::tests::abuse_command_in_tool_result_is_not_parsed` | pass |
| 4 | tool call denied by policy | `tools::tests::abuse_denied_tool_call_is_not_executed_and_model_receives_a_tool_error`; the same over MCP in `tools_mcp` | pass |
| 5 | outbound call to an unregistered agent | `a2a_client::tests::abuse_agent_call_to_an_unregistered_agent_is_refused` | pass |
| 6 | malicious plugin manifest | `plugins::abuse_plugin_manifest_escaping_its_directory_is_refused_whole` | pass |
| 7 | credential leakage | `o11y::tests::abuse_canary_credential_appears_nowhere_observable`; `canary::abuse_canary_credential_appears_nowhere_observable_end_to_end`; `speech::abuse_voice_key_never_leaves_the_harness`; `config::tests::abuse_literal_credential_in_config_is_refused`; Temporal payloads: the assertions of `abuse_temporal_payloads_carry_no_secret` run inside `durable_temporal::the_embedded_dev_server_starts_and_a_trivial_workflow_completes` (one dev server per binary), green in the full suite | pass |
| 8 | inbox flood / many streams | `server_auth::abuse_ninth_stream_of_one_caller_is_rejected_and_the_other_task_keeps_streaming`; `inbox::abuse_inbox_event_for_a_terminal_task_is_rejected_without_creating_one` | pass |
| 9 | push webhook to an internal address | `server_channels::abuse_push_config_naming_a_link_local_address_is_rejected` | pass |
| 10 | command by a role that may not | `commands::tests::abuse_watcher_invoking_model_command_is_refused`; **new**: `sdk::abuse_watcher_command_through_the_agent_is_refused` (the runtime path task 54 wired) | pass |
| 11 | key in the browser bundle | `web/tests/bundle-scan.test.ts` (`abuse_npm_bundle_contains_no_provider_key_pattern`), below | pass |

Fail-closed start: `cli::abuse_remote_durable_unreachable_refuses_to_start` (pass).

## The credential canary

The end-to-end canary (`canary.rs`) runs the demo path (real host, real files, the
provider over HTTP, the A2A server over TCP) with a canary model key and a canary bearer
token, and looks for either in every A2A response, the card, the store on disk, the logs
and spans, and the provider error path: **not found anywhere**. The voice
leg (`abuse_voice_key_never_leaves_the_harness`) covers the WebSocket handshake and
the HTTP speech calls. The live demo runs (`e2e-none.md`, `e2e-remote.md`) were
scanned for key patterns, the Temporal address and the namespace: none present.

## The bundle scan

`bun run --cwd web test:bundle-scan`: **1 passed**. Key patterns (OpenAI, Anthropic, AWS,
GitHub, Slack, bearer literals) are refused in the web renderer's bundle and in all three
builds of the npm WASM package. Provider hosts are refused in the web renderer's bundle
only. The WASM SDK is a harness: it names the providers it defaults to and reaches them
only through the page's `fetch` (`the_wasm_build_runs_a_task_against_a_mocked_host`,
`a_fetch_outside_allow_origins_is_refused`). The sweep found the scan failing on the WASM
build since task 42 had added it, and scoped the host rule accordingly.

## Supply chain

`cargo deny check`: advisories ok, bans ok, licenses ok, sources ok.

## Least privilege

- **Stdio MCP servers get a scrubbed environment**, only the variables the plugin
  lists: `native::tests::a_spawned_child_sees_only_the_listed_environment` (pass).
- **The Temporal Cloud key is namespace-scoped:** an operator attestation, not a test.
  The harness reads it by reference (`keyring:service=temporal,project=rusty-harness`) and
  uses it only against the configured namespace. Whether the key could reach other
  namespaces is a property of the key, set in Temporal Cloud by its owner.

## Verification run

Re-run at the verification node.

### Abuse scenarios and the canary

```sh
cargo nextest run --workspace --all-features -E 'test(abuse_)'
```

Outcome: **passed** in 3s, 2026-10-25, at `0a97f5b`.

```text
Starting 19 tests across 33 binaries (228 tests skipped)
        PASS [   0.100s] ( 1/19) rusty_harness harness::commands::tests::abuse_command_in_tool_result_is_not_parsed
        PASS [   0.259s] ( 2/19) rusty_harness config::tests::abuse_literal_credential_in_config_is_refused
        PASS [   0.170s] ( 3/19) rusty_harness harness::commands::tests::abuse_watcher_invoking_model_command_is_refused
        PASS [   0.270s] ( 4/19) rusty_harness app_service::o11y::tests::abuse_canary_credential_appears_nowhere_observable
        PASS [   0.015s] ( 5/19) rusty_harness harness::core_loop::context_window::tests::abuse_injected_instructions_in_tool_result_change_no_policy_config_or_role
        PASS [   0.079s] ( 6/19) rusty_harness harness::tools::tests::abuse_denied_tool_call_is_not_executed_and_model_receives_a_tool_error
        PASS [   0.349s] ( 7/19) rusty_harness harness::a2a_client::tests::abuse_agent_call_to_an_unregistered_agent_is_refused
        PASS [   0.922s] ( 8/19) rusty_harness::cli abuse_remote_durable_unreachable_refuses_to_start
        PASS [   0.009s] ( 9/19) rusty_harness::plugins abuse_plugin_manifest_escaping_its_directory_is_refused_whole
        PASS [   0.980s] (10/19) rusty_harness::canary abuse_canary_credential_appears_nowhere_observable_end_to_end
        PASS [   0.046s] (11/19) rusty_harness::sdk abuse_watcher_command_through_the_agent_is_refused
        PASS [   0.929s] (12/19) rusty_harness::inbox abuse_inbox_event_for_a_terminal_task_is_rejected_without_creating_one
        PASS [   0.937s] (13/19) rusty_harness::inbox_route abuse_inbox_post_from_a_non_participant_is_refused_and_a_participants_is_applied
        PASS [   0.157s] (14/19) rusty_harness::server_auth abuse_unauthenticated_send_message_is_rejected_before_any_task_is_created
        PASS [   0.006s] (15/19) rusty_harness::speech abuse_voice_key_never_leaves_the_harness
        PASS [   0.032s] (16/19) rusty_harness::tools_mcp abuse_denied_tool_call_is_not_executed_and_model_receives_a_tool_error
        PASS [   0.268s] (17/19) rusty_harness::server_auth abuse_non_participant_gets_task_not_found_and_empty_list_tasks
        PASS [   0.257s] (18/19) rusty_harness::server_channels abuse_push_config_naming_a_link_local_address_is_rejected
        PASS [   0.381s] (19/19) rusty_harness::server_auth abuse_ninth_stream_of_one_caller_is_rejected_and_the_other_task_keeps_streaming
     Summary [   1.631s] 19 tests run: 19 passed, 228 skipped
```

### Bundle scan

```sh
bun run test:bundle-scan
```

Outcome: **passed** in 2s, 2026-10-25, at `0a97f5b`.

```text
✓ 3057 modules transformed.
✓ built in 1.22s
 Test Files  1 passed (1)
      Tests  1 passed (1)
```
