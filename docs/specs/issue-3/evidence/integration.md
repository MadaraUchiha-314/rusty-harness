# T2 — integration scenarios

<!-- markdownlint-disable MD013 -->

Modules collaborating, each test carrying its Gherkin docstring. The Temporal scenarios run on the SDK's ephemeral dev server. The table is `the-loop scenarios --glob 'crates/*/tests/**/*.rs' --glob 'crates/*/tests/*.rs' --format markdown`.

## Scenarios

| # | Feature | Scenario | Requirement | Location |
|---|---|---|---|---|
| 1 | Prompts as assets | Every registered prompt fragment resolves to a markdown file | docs/specs/issue-3/requirements.md#R33 | crates/rusty-harness/tests/assets.rs:28 |
| 2 | Prompts as assets | A missing key in a locale falls back to en and a missing key in en fails the build | — | crates/rusty-harness/tests/assets.rs:74 |
| 3 | Prompts as assets | No prompt is a string literal in code | — | crates/rusty-harness/tests/assets.rs:110 |
| 4 | — | The canary credential appears nowhere observable | — | crates/rusty-harness/tests/canary.rs:105 |
| 5 | — | A restart in none mode resumes from the last persisted iteration | — | crates/rusty-harness/tests/chaos_loop.rs:150 |
| 6 | — | A crash between two store writes leaves a consistent store on reopen | — | crates/rusty-harness/tests/chaos_store.rs:96 |
| 7 | — | The demo configuration loads as checked in | — | crates/rusty-harness/tests/cli.rs:123 |
| 8 | — | Every extension payload the harness produces matches its published schema | — | crates/rusty-harness/tests/contract_a2a_extensions.rs:231 |
| 9 | — | Killing the activity worker re-dispatches the model call once | — | crates/rusty-harness/tests/durable_chaos.rs:463 |
| 10 | — | A crashed activity's failure and retry hooks run on the surviving worker | — | crates/rusty-harness/tests/durable_chaos.rs:469 |
| 11 | — | Killing the workflow worker replays without a repeated tool side effect | — | crates/rusty-harness/tests/durable_chaos.rs:581 |
| 12 | — | Killing the worker after the peer accepted and before the id was recorded | — | crates/rusty-harness/tests/durable_chaos.rs:645 |
| 13 | — | The embedded dev server starts and a trivial workflow completes | — | crates/rusty-harness/tests/durable_temporal.rs:59 |
| 14 | — | The canary credential appears nowhere observable (Temporal leg) | — | crates/rusty-harness/tests/durable_temporal.rs:63 |
| 15 | — | A long-running task stays in input-required across three turns (embedded) | — | crates/rusty-harness/tests/durable_temporal.rs:240 |
| 16 | — | A retried model call retracts the partial output of the failed attempt | — | crates/rusty-harness/tests/durable_temporal.rs:341 |
| 17 | — | Spans across a signal and a retry link into one trace | — | crates/rusty-harness/tests/durable_temporal.rs:500 |
| 18 | — | The demo runs end to end in none mode | — | crates/rusty-harness/tests/e2e.rs:372 |
| 19 | — | The demo runs end to end in remote mode | — | crates/rusty-harness/tests/e2e.rs:382 |
| 20 | — | A duplicate inbox signal applies the event once | — | crates/rusty-harness/tests/inbox.rs:120 |
| 21 | — | A follow-up to a completed task starts a new task with referenceTaskIds | — | crates/rusty-harness/tests/inbox.rs:166 |
| 22 | — | An inbox event for a running task is applied at the next loop boundary | — | crates/rusty-harness/tests/inbox.rs:407 |
| 23 | — | A webhook post reaches the task through the inbox | — | crates/rusty-harness/tests/inbox_route.rs:49 |
| 24 | — | An image sent to a text-only model is rejected naming the modality | — | crates/rusty-harness/tests/multimodal.rs:91 |
| 25 | — | An image sent to a vision model reaches it as an image part | — | crates/rusty-harness/tests/multimodal.rs:114 |
| 26 | — | An image a tool renders reaches the participants as an artifact | — | crates/rusty-harness/tests/multimodal.rs:178 |
| 27 | — | A spoken message is transcribed and the spoken turn is answered in speech | — | crates/rusty-harness/tests/multimodal.rs:318 |
| 28 | — | Every operation of a task appears in one trace | — | crates/rusty-harness/tests/o11y.rs:132 |
| 29 | — | A plugin with an invalid manifest leaves no entity behind | — | crates/rusty-harness/tests/plugins.rs:31 |
| 30 | — | A plugin whose manifest escapes its directory is refused whole (abuse case 6) | — | crates/rusty-harness/tests/plugins.rs:80 |
| 31 | — | A skill body is loaded on activation and not before | — | crates/rusty-harness/tests/plugins.rs:107 |
| 32 | — | A registered default is replaced by a plugin without a code change | — | crates/rusty-harness/tests/plugins.rs:150 |
| 33 | — | A programmatic plugin registers identically to one from the file system | — | crates/rusty-harness/tests/plugins.rs:224 |
| 34 | — | A configured plugin path that cannot be read fails discovery | — | crates/rusty-harness/tests/plugins.rs:273 |
| 35 | — | A `file:` system prompt replaces the default (R7.2) | — | crates/rusty-harness/tests/sdk.rs:117 |
| 36 | — | A person's command runs instead of the model (R10.1, R10.6) | — | crates/rusty-harness/tests/sdk.rs:177 |
| 37 | — | A watcher invoking /model is refused (abuse 10), through the agent | — | crates/rusty-harness/tests/sdk.rs:234 |
| 38 | — | The official Rust client drives the server over JSON-RPC | — | crates/rusty-harness/tests/server.rs:54 |
| 39 | — | A browser surface on another origin reaches a loopback harness | — | crates/rusty-harness/tests/server.rs:267 |
| 40 | — | An extension not advertised in the card is not activated | — | crates/rusty-harness/tests/server_auth.rs:342 |
| 41 | — | A task watcher outside a channel sees the state change but not the question | — | crates/rusty-harness/tests/server_channels.rs:106 |
| 42 | — | `GetTask` after a restart returns the full A2A task | — | crates/rusty-harness/tests/server_channels.rs:218 |
| 43 | — | The voice session's key never leaves the harness | — | crates/rusty-harness/tests/speech.rs:142 |
| 44 | — | Several tool calls run concurrently and return in issue order | — | crates/rusty-harness/tests/tools_mcp.rs:89 |
| 45 | — | A tool result above the size limit is truncated by the configured policy | — | crates/rusty-harness/tests/tools_mcp.rs:123 |
| 46 | — | A pre hook veto stops tool execution and the model is told | — | crates/rusty-harness/tests/tools_mcp.rs:140 |
| 47 | — | Push-to-talk holds with key releases and toggles without them | — | crates/rusty-harness/tests/tui.rs:641 |
| 48 | — | A command token is highlighted in the composer and in a sent message | — | crates/rusty-harness/tests/tui.rs:733 |
| 49 | — | Answering an A2UI form sends the action as an A2UI client message on the | — | crates/rusty-harness/tests/tui.rs:745 |
| 50 | — | A completed task is followed up as a new task that references it | — | crates/rusty-harness/tests/tui.rs:782 |
| 51 | — | Complete sends the task extension's transition and cancel sends `CancelTask` | — | crates/rusty-harness/tests/tui.rs:810 |
| 52 | — | An image artifact renders in a graphics-capable terminal and degrades to alt text | — | crates/rusty-harness/tests/tui.rs:869 |
| 53 | — | An A2UI ask renders in the web renderer and the TUI (the TUI leg) | — | crates/rusty-harness/tests/tui.rs:1003 |
| 54 | — | :the_wasm_build_runs_a_task_against_a_mocked_host().await; | — | crates/rusty-harness-wasm/tests/browser.rs:20 |
| 55 | — | :a_fetch_outside_allow_origins_is_refused().await; | — | crates/rusty-harness-wasm/tests/browser.rs:25 |
| 56 | Host capabilities on wasm32 | An unavailable capability fails at construction on wasm32 | docs/specs/issue-3/requirements.md#R29 | crates/rusty-harness-wasm/tests/node.rs:19 |
| 57 | Host capabilities on wasm32 | :the_wasm_build_runs_a_task_against_a_mocked_host().await; | — | crates/rusty-harness-wasm/tests/node.rs:37 |
| 58 | Host capabilities on wasm32 | :a_fetch_outside_allow_origins_is_refused().await; | — | crates/rusty-harness-wasm/tests/node.rs:42 |
| 59 | WASM and browser support | The wasm build runs a task against a mocked host | docs/specs/issue-3/requirements.md#R29 | crates/rusty-harness-wasm/tests/scenario/mod.rs:51 |
| 60 | WASM and browser support | A fetch outside the host's allowOrigins is refused | — | crates/rusty-harness-wasm/tests/scenario/mod.rs:92 |

## Run

```sh
cargo nextest run --workspace --all-features -E 'kind(test)' --no-fail-fast
```

Outcome: **passed** in 18s, 2026-10-25, at `0a97f5b`. The five skipped tests are the credential-gated `e2e_*` journeys and the chaos victims (`#[ignore]`d children each chaos test spawns); T4 and T13 run them. The three `LEAK` marks are nextest noting that the Temporal dev server, a child process, held the test's output handles for a moment after it passed; they are not failures.

```text
Starting 84 tests across 30 binaries (5 tests and 3 binaries skipped)
        PASS [   0.007s] ( 1/84) rusty_harness::assets contract_prompt_files_match_registry
        PASS [   0.007s] ( 2/84) rusty_harness::assets assets_every_prompt_fragment_resolves_to_a_file
        PASS [   0.009s] ( 3/84) rusty_harness::assets assets_placeholder_sets_match_en
        PASS [   0.007s] ( 4/84) rusty_harness::assets contract_strings_catalogs_match_en
        PASS [   0.007s] ( 5/84) rusty_harness::assets snapshot_rendered_default_prompts
        PASS [   0.081s] ( 6/84) rusty_harness::assets assets_no_raw_prompt_literals
        PASS [   0.102s] ( 7/84) rusty_harness::chaos_store chaos_crash_between_store_writes_leaves_a_consistent_store_on_reopen
        PASS [   0.199s] ( 8/84) rusty_harness::chaos_loop chaos_restart_in_none_mode_resumes_from_the_last_persisted_iteration
        PASS [   0.148s] ( 9/84) rusty_harness::cli a_configured_credential_that_does_not_resolve_refuses_to_start
        PASS [   0.008s] (10/84) rusty_harness::cli the_demo_fixture_config_loads
        PASS [   0.123s] (11/84) rusty_harness::cli abuse_remote_durable_unreachable_refuses_to_start
        PASS [   0.082s] (12/84) rusty_harness::cli snapshot_first_run_output
        PASS [   0.059s] (13/84) rusty_harness::contract_a2a_extensions contract_a2a_extension_schemas
        PASS [   0.008s] (14/84) rusty_harness::contract_agent_trait contract_agent_trait_matches_sherma_shape
        PASS [   0.006s] (15/84) rusty_harness::contract_hook_catalogue contract_hook_catalogue_covers_sherma_points
        PASS [   0.006s] (16/84) rusty_harness::contract_hook_catalogue contract_hook_catalogue_lists_every_point_of_the_design_table
        PASS [   0.024s] (17/84) rusty_harness::contract_a2a_extensions snapshot_extension_payloads
        PASS [   0.007s] (18/84) rusty_harness::contract_module_tree contract_module_tree_matches_design
        PASS [   0.122s] (19/84) rusty_harness::contract_a2a_extensions contract_extension_payloads_match_their_schemas
        PASS [   0.356s] (20/84) rusty_harness::canary abuse_canary_credential_appears_nowhere_observable_end_to_end
        LEAK [   4.620s] (21/84) rusty_harness::durable_temporal one_trace::spans_across_a_signal_and_a_retry_link_into_one_trace
        LEAK [   2.250s] (22/84) rusty_harness::durable_temporal task_workflow::a_long_running_task_runs_three_turns_as_a_temporal_workflow
        PASS [   7.657s] (23/84) rusty_harness::durable_chaos chaos_killing_the_worker_after_the_peer_accepted_and_before_the_id_was_recorded_creates_no_duplicate_remote_task
        PASS [   8.567s] (24/84) rusty_harness::durable_chaos chaos_killing_the_activity_worker_redispatches_the_model_call_once
        PASS [   0.397s] (25/84) rusty_harness::heartbeat the_heartbeat_forwards_an_event_whose_signal_failed
        PASS [   0.005s] (26/84) rusty_harness::hello hello_from_a_spawned_task
        PASS [   0.140s] (27/84) rusty_harness::inbox a_duplicate_inbox_signal_applies_the_event_once
        PASS [   1.498s] (28/84) rusty_harness::durable_temporal the_embedded_dev_server_starts_and_a_trivial_workflow_completes
        PASS [   0.022s] (29/84) rusty_harness::inbox a_message_accepted_in_the_runs_tail_runs_the_next_turn_once
        PASS [   0.277s] (30/84) rusty_harness::inbox a_follow_up_to_a_completed_task_starts_a_new_task_with_reference_task_ids
        PASS [   0.212s] (31/84) rusty_harness::inbox abuse_inbox_event_for_a_terminal_task_is_rejected_without_creating_one
        PASS [   0.126s] (32/84) rusty_harness::multimodal an_image_a_tool_renders_reaches_the_participants_as_an_artifact
        PASS [   0.049s] (33/84) rusty_harness::multimodal an_image_sent_to_a_text_only_model_is_rejected_naming_the_modality
        PASS [   0.052s] (34/84) rusty_harness::multimodal an_image_sent_to_a_vision_model_reaches_it_as_an_image_part
        PASS [   0.247s] (35/84) rusty_harness::inbox_route abuse_inbox_post_from_a_non_participant_is_refused_and_a_participants_is_applied
        PASS [   0.067s] (36/84) rusty_harness::multimodal spoken::a_spoken_message_is_transcribed_and_the_spoken_turn_is_answered_in_speech
        PASS [   0.069s] (37/84) rusty_harness::plan a_plan_with_a_cycle_is_refused
        PASS [   0.020s] (38/84) rusty_harness::plan the_model_plans_records_a_step_and_everyone_sees_the_plan
        PASS [   0.008s] (39/84) rusty_harness::plugins a_plugin_with_an_invalid_manifest_leaves_no_entity_behind
        PASS [   0.007s] (40/84) rusty_harness::plugins a_programmatic_plugin_registers_identically
        PASS [   0.165s] (41/84) rusty_harness::o11y every_operation_of_a_task_appears_in_one_trace
        PASS [   0.009s] (42/84) rusty_harness::plugins a_registered_default_is_replaced_by_a_plugin_without_a_code_change
        PASS [   0.007s] (43/84) rusty_harness::plugins a_skill_body_is_loaded_on_activation_and_not_before
        PASS [   0.007s] (44/84) rusty_harness::plugins abuse_plugin_manifest_escaping_its_directory_is_refused_whole
        PASS [   0.008s] (45/84) rusty_harness::renderer snapshot_a2ui_ask_payload
        PASS [   0.037s] (46/84) rusty_harness::sdk a_persons_command_runs_instead_of_the_model
        PASS [   0.050s] (47/84) rusty_harness::sdk a_file_system_prompt_replaces_the_default
        PASS [   0.013s] (48/84) rusty_harness::sdk abuse_watcher_command_through_the_agent_is_refused
        PASS [   0.010s] (49/84) rusty_harness::sdk building_fails_closed_without_a_default_model_or_with_a_bad_store
        PASS [   0.014s] (50/84) rusty_harness::sdk the_harness_is_usable_from_rust_in_process
        PASS [   0.046s] (51/84) rusty_harness::server snapshot_agent_card
        LEAK [   3.023s] (52/84) rusty_harness::durable_temporal task_workflow::a_retried_model_call_retracts_the_failed_attempt
        PASS [   0.144s] (53/84) rusty_harness::server a_browser_origin_is_allowed_on_loopback_and_only_listed_origins_with_a_list
        PASS [   0.156s] (54/84) rusty_harness::server the_official_rust_client_drives_the_server_over_jsonrpc
        PASS [   0.109s] (55/84) rusty_harness::server_auth abuse_unauthenticated_send_message_is_rejected_before_any_task_is_created
        PASS [   0.186s] (56/84) rusty_harness::server_auth abuse_non_participant_gets_task_not_found_and_empty_list_tasks
        PASS [   0.101s] (57/84) rusty_harness::server_auth an_extension_not_advertised_in_the_card_is_not_activated
        PASS [   0.373s] (58/84) rusty_harness::server_auth abuse_ninth_stream_of_one_caller_is_rejected_and_the_other_task_keeps_streaming
        PASS [   0.185s] (59/84) rusty_harness::server_channels a_task_watcher_outside_a_channel_sees_the_state_change_but_not_the_question
        PASS [   0.007s] (60/84) rusty_harness::speech abuse_voice_key_never_leaves_the_harness
        PASS [   0.006s] (61/84) rusty_harness::speech snapshot_realtime_session_config
        PASS [   0.031s] (62/84) rusty_harness::tools_mcp a_pre_hook_veto_stops_tool_execution_and_the_model_is_told
        PASS [   0.025s] (63/84) rusty_harness::tools_mcp a_tool_result_above_the_size_limit_is_truncated_by_the_configured_policy
        PASS [   0.014s] (64/84) rusty_harness::tools_mcp abuse_denied_tool_call_is_not_executed_and_model_receives_a_tool_error
        PASS [   0.193s] (65/84) rusty_harness::server_channels abuse_push_config_naming_a_link_local_address_is_rejected
        PASS [   0.218s] (66/84) rusty_harness::server_channels get_task_after_a_restart_returns_the_full_a2a_task
        PASS [   0.012s] (67/84) rusty_harness::tui a_message_after_a_terminal_task_starts_a_new_task_that_references_it
        PASS [   0.057s] (68/84) rusty_harness::tui a_capture_is_sent_as_a_voice_part_and_a_spoken_reply_is_queued_for_playback
        PASS [   0.010s] (69/84) rusty_harness::tui a_registered_command_is_highlighted_and_an_unknown_one_is_not
        PASS [   0.009s] (70/84) rusty_harness::tui a_remote_card_supplies_the_commands_and_the_model_aliases
        PASS [   0.021s] (71/84) rusty_harness::tui a_plan_artifact_feeds_the_plan_strip_not_the_chat
        PASS [   0.012s] (72/84) rusty_harness::tui an_a2ui_form_answer_becomes_a_client_action_on_the_channel
        PASS [   0.011s] (73/84) rusty_harness::tui complete_and_cancel_are_explicit_controls
        PASS [   0.052s] (74/84) rusty_harness::tui an_image_artifact_renders_in_a_graphics_capable_terminal_and_degrades_to_alt_text
        PASS [   0.012s] (75/84) rusty_harness::tui push_to_talk_holds_with_releases_and_toggles_without
        PASS [   0.022s] (76/84) rusty_harness::tui tui_snapshot_a2ui
        PASS [   0.019s] (77/84) rusty_harness::tui tui_snapshot_chat
        PASS [   0.019s] (78/84) rusty_harness::tui tui_snapshot_done
        PASS [   0.016s] (79/84) rusty_harness::tui tui_snapshot_first
        PASS [   0.017s] (80/84) rusty_harness::tui tui_snapshot_model
        PASS [   0.016s] (81/84) rusty_harness::tui tui_snapshot_voice
        PASS [   0.166s] (82/84) rusty_harness::tui on_one_harness::an_a2ui_ask_renders_in_the_tui_on_one_harness
        PASS [   0.413s] (83/84) rusty_harness::tools_mcp several_tool_calls_run_concurrently_and_return_in_issue_order
        PASS [  15.231s] (84/84) rusty_harness::durable_chaos chaos_killing_the_workflow_worker_replays_without_a_repeated_tool_side_effect
     Summary [  15.592s] 84 tests run: 84 passed (3 leaky), 5 skipped
```
