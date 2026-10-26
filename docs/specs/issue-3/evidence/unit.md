# T1 — unit tests

<!-- markdownlint-disable MD013 -->

Every module's public types, through their public interface.

## Library unit tests (every crate, every feature)

```sh
cargo nextest run --workspace --all-features -E 'kind(lib)' --no-fail-fast
```

Outcome: **passed** in 3s, 2026-10-25, at `0a97f5b`.

```text
Starting 156 tests across 2 binaries (31 binaries skipped)
        PASS [   0.017s] (  1/156) rusty_harness app_service::a2a_server::protocol_gap::tests::the_token_bucket_refills_per_minute_and_the_stream_budget_releases_on_drop
        PASS [   0.028s] (  2/156) rusty_harness app_service::a2a_server::protocol_gap::tests::credentials_are_compared_in_constant_time_and_the_participant_header_names_the_caller
        PASS [   0.033s] (  3/156) rusty_harness app_service::a2a_server::protocol_gap::tests::negotiation_keeps_only_advertised_extensions_in_request_order
        PASS [   0.034s] (  4/156) rusty_harness app_service::durable::temporal::codec::tests::a_payload_carrying_a_protected_secret_is_refused_and_others_pass
        PASS [   0.017s] (  5/156) rusty_harness app_service::durable::temporal::tests::remote_options_resolve_references_inside_and_register_the_key_on_the_guard
        PASS [   0.008s] (  6/156) rusty_harness app_service::durable::tests::the_schedule_doubles_from_the_first_wait_and_stays_finite
        PASS [   0.012s] (  7/156) rusty_harness app_service::o11y::langfuse::tests::a_model_call_becomes_a_generation_inside_the_catalogue_span
        PASS [   0.014s] (  8/156) rusty_harness app_service::heartbeat::tests::a_task_idle_past_stall_after_runs_the_stalled_hook_and_the_default_is_a_warning
        PASS [   0.009s] (  9/156) rusty_harness app_service::o11y::redact::tests::credential_shaped_fields_hide_and_content_truncates_above_debug
        PASS [   0.012s] ( 10/156) rusty_harness app_service::o11y::tests::a_failing_operation_closes_its_span_with_the_error
        PASS [   0.010s] ( 11/156) rusty_harness app_service::o11y::tests::abuse_canary_credential_appears_nowhere_observable
        PASS [   0.009s] ( 12/156) rusty_harness app_service::o11y::tests::every_operation_opens_a_span_with_its_attributes_and_nests
        PASS [   0.008s] ( 13/156) rusty_harness app_service::o11y::trace::tests::a_traceparent_round_trips_and_invalid_ones_are_refused
        PASS [   0.010s] ( 14/156) rusty_harness app_service::o11y::trace::tests::a_task_adopts_the_callers_traceparent_or_mints_one
        PASS [   0.009s] ( 15/156) rusty_harness assets::catalog::tests::the_builtin_catalog_falls_back_per_key
        PASS [   0.010s] ( 16/156) rusty_harness assets::catalog::tests::the_subset_formats_arguments_plurals_and_selects
        PASS [   0.008s] ( 17/156) rusty_harness assets::prompt::tests::a_fragment_with_crlf_line_endings_parses_the_same
        PASS [   0.009s] ( 18/156) rusty_harness assets::prompt::tests::a_fragment_parses_its_front_matter_and_checks_its_placeholders
        PASS [   0.007s] ( 19/156) rusty_harness config::duration::tests::parses_and_prints_the_four_units
        PASS [   0.008s] ( 20/156) rusty_harness assets::prompt::tests::the_builtin_registry_falls_back_to_en
        PASS [   0.008s] ( 21/156) rusty_harness assets::prompt::tests::a_prompt_renders_fragments_and_frames_data_and_keeps_provenance
        PASS [   0.008s] ( 22/156) rusty_harness config::tests::environment_values_are_typed_and_unrelated_variables_are_ignored
        PASS [   0.009s] ( 23/156) rusty_harness config::tests::abuse_literal_credential_in_config_is_refused
        PASS [   0.009s] ( 24/156) rusty_harness config::tests::precedence_is_defaults_then_file_then_env_then_code
        PASS [   0.007s] ( 25/156) rusty_harness config::tests::unknown_fields_and_wrong_types_are_refused
        PASS [   0.009s] ( 26/156) rusty_harness config::tests::snapshot_default_config_toml
        PASS [   0.015s] ( 27/156) rusty_harness config::tests::snapshot_config_schema
        PASS [   0.008s] ( 28/156) rusty_harness error::tests::internal_never_names_a_caller_error_code
        PASS [   0.008s] ( 29/156) rusty_harness error::tests::a2a_errors_pass_through_unchanged
        PASS [   0.008s] ( 30/156) rusty_harness error::tests::unsupported_and_illegal_transition_map_to_unsupported_operation
        PASS [   0.011s] ( 31/156) rusty_harness harness::a2a_client::tests::a_retried_agent_call_adopts_the_task_its_earlier_attempt_created
        PASS [   0.012s] ( 32/156) rusty_harness harness::a2a_client::tests::a_sub_agent_whose_card_fails_validation_is_never_shown_to_the_model
        PASS [   0.020s] ( 33/156) rusty_harness harness::a2a_client::tests::a_loopback_sub_agent_result_is_indistinguishable_from_a_remote_one
        PASS [   0.016s] ( 34/156) rusty_harness harness::a2a_client::tests::abuse_agent_call_to_an_unregistered_agent_is_refused
        PASS [   0.015s] ( 35/156) rusty_harness harness::channels::tests::a_participant_ask_round_trips_through_a_channel
        PASS [   0.009s] ( 36/156) rusty_harness harness::commands::tests::a_plugin_declared_command_runs_on_its_mcp_server_with_the_caller_attached
        PASS [   0.017s] ( 37/156) rusty_harness harness::channels::tests::delegation_creates_a_linked_task_for_the_participant
        PASS [   0.008s] ( 38/156) rusty_harness harness::commands::tests::abuse_command_in_tool_result_is_not_parsed
        PASS [   0.016s] ( 39/156) rusty_harness harness::commands::tests::abuse_watcher_invoking_model_command_is_refused
        PASS [   0.010s] ( 40/156) rusty_harness harness::commands::tests::an_unknown_command_is_answered_with_a_suggestion_and_never_sent_on
        PASS [   0.048s] ( 41/156) rusty_harness harness::channels::tests::a_participant_answering_after_the_tool_timeout_resumes_the_original_suspended_call
        PASS [   0.010s] ( 42/156) rusty_harness harness::commands::tests::commands_are_parsed_from_people_only
        PASS [   0.014s] ( 43/156) rusty_harness harness::commands::tests::switching_model_applies_to_the_next_call_without_a_restart
        PASS [   0.016s] ( 44/156) rusty_harness harness::core_loop::agent::tests::a_non_retryable_model_error_fails_the_task_with_a_typed_error
        PASS [   0.021s] ( 45/156) rusty_harness harness::core_loop::agent::tests::a_long_running_task_stays_in_input_required_across_three_turns
        PASS [   0.019s] ( 46/156) rusty_harness harness::core_loop::agent::tests::a_task_whose_parent_completes_is_canceled_by_the_default_policy
        PASS [   0.020s] ( 47/156) rusty_harness harness::core_loop::agent::tests::limits_stop_the_loop_with_a_typed_outcome_and_suspending_tools_wait
        PASS [   0.008s] ( 48/156) rusty_harness harness::core_loop::compaction::tests::compaction_keeps_the_pinned_positions_verbatim
        PASS [   0.008s] ( 49/156) rusty_harness harness::core_loop::compaction::tests::compaction_triggers_above_the_threshold_and_plans_around_recent_and_pinned_turns
        PASS [   0.008s] ( 50/156) rusty_harness harness::core_loop::compaction::tests::context_compact_hooks_can_edit_the_plan_or_veto
        PASS [   0.013s] ( 51/156) rusty_harness harness::core_loop::context_window::tests::abuse_injected_instructions_in_tool_result_change_no_policy_config_or_role
        PASS [   0.008s] ( 52/156) rusty_harness harness::core_loop::context_window::tests::positions_come_out_in_template_order_with_the_breakpoint_after_the_stable_prefix
        PASS [   0.009s] ( 53/156) rusty_harness harness::core_loop::context_window::tests::the_estimate_is_corrected_from_provider_usage
        PASS [   0.011s] ( 54/156) rusty_harness harness::core_loop::persistence::file::tests::a_listing_never_deletes_a_write_in_flight
        PASS [   0.013s] ( 55/156) rusty_harness harness::core_loop::persistence::file::tests::a_store_whose_materialization_is_behind_is_repaired_on_open
        PASS [   0.028s] ( 56/156) rusty_harness harness::core_loop::persistence::file::tests::the_file_store_passes_the_store_scenarios_and_survives_a_reopen
        PASS [   0.009s] ( 57/156) rusty_harness harness::core_loop::persistence::tests::a_tail_replays_then_follows_live_commits_and_ends_at_the_final_event
        PASS [   0.007s] ( 58/156) rusty_harness harness::core_loop::persistence::tests::plan_persist_hooks_fire_on_every_put_plan
        PASS [   0.009s] ( 59/156) rusty_harness harness::core_loop::persistence::tests::the_memory_store_commits_atomically_and_idempotently
        PASS [   0.010s] ( 60/156) rusty_harness harness::core_loop::plan::tests::a_cycle_is_rejected_with_a_typed_error_and_the_plan_is_unchanged
        PASS [   0.007s] ( 61/156) rusty_harness harness::core_loop::plan::tests::topo_and_ready_follow_the_edges_and_outputs_are_recorded
        PASS [   0.007s] ( 62/156) rusty_harness harness::core_loop::state::tests::a_schema_violation_is_a_typed_error_and_the_state_is_unchanged
        PASS [   0.008s] ( 63/156) rusty_harness harness::core_loop::state::tests::typed_values_round_trip_under_named_keys
        PASS [   0.008s] ( 64/156) rusty_harness harness::core_loop::task::tests::a_state_change_runs_its_hooks_and_a_terminal_move_drops_pending_calls
        PASS [   0.008s] ( 65/156) rusty_harness harness::core_loop::task::tests::every_illegal_transition_is_a_typed_error_and_changes_nothing
        PASS [   0.007s] ( 66/156) rusty_harness harness::core_loop::task::tests::every_legal_transition_of_the_diagram_is_accepted
        PASS [   0.008s] ( 67/156) rusty_harness harness::hooks::remote::tests::a_remote_failure_is_a_remote_hook_error
        PASS [   0.008s] ( 68/156) rusty_harness harness::hooks::remote::tests::method_and_tool_names_follow_sherma_shapes
        PASS [   0.010s] ( 69/156) rusty_harness harness::hooks::remote::tests::remote_hooks_over_json_rpc_and_mcp_see_the_same_typed_context_as_local_hooks
        PASS [   0.007s] ( 70/156) rusty_harness harness::hooks::tests::a_pre_hook_that_rewrites_a_tool_call_after_approval_is_rechecked_by_the_guard
        PASS [   0.007s] ( 71/156) rusty_harness harness::hooks::tests::a_pre_veto_stops_the_operation_before_it_runs
        PASS [   0.008s] ( 72/156) rusty_harness harness::hooks::tests::an_in_hook_that_moves_the_target_fails_the_operation
        PASS [   0.008s] ( 73/156) rusty_harness harness::hooks::tests::in_hooks_wrap_outermost_first_and_may_replace
        PASS [   0.009s] ( 74/156) rusty_harness harness::hooks::tests::post_hooks_rewrite_the_output
        PASS [   0.007s] ( 75/156) rusty_harness harness::hooks::tests::pre_failures_follow_the_point_policy_and_security_points_stay_closed
        PASS [   0.010s] ( 76/156) rusty_harness harness::hooks::tests::pre_hooks_run_by_priority_then_after_then_registration
        PASS [   0.008s] ( 77/156) rusty_harness harness::hooks::tests::the_catalogue_names_are_unique_dotted_and_round_trip
        PASS [   0.008s] ( 78/156) rusty_harness harness::models::llm::anthropic::tests::snapshot_anthropic_request_body
        PASS [   0.316s] ( 79/156) rusty_harness harness::core_loop::agent::tests::an_inbox_event_for_a_running_task_is_applied_at_the_next_loop_boundary
        PASS [   0.009s] ( 80/156) rusty_harness harness::models::llm::openai::responses::tests::an_incomplete_response_stops_at_the_cap_and_a_failure_is_an_error
        PASS [   0.010s] ( 81/156) rusty_harness harness::models::llm::anthropic::tests::streams_text_tool_use_and_usage_with_the_api_key_header
        PASS [   0.008s] ( 82/156) rusty_harness harness::models::llm::openai::responses::tests::snapshot_openai_responses_request_body
        PASS [   0.009s] ( 83/156) rusty_harness harness::models::llm::openai::responses::tests::decodes_text_function_calls_and_cached_usage
        PASS [   0.010s] ( 84/156) rusty_harness harness::models::llm::openai::tests::snapshot_openai_request_body
        PASS [   0.012s] ( 85/156) rusty_harness harness::models::llm::openai::tests::dotted_tool_names_go_out_sanitised_and_come_back_as_ours
        PASS [   0.012s] ( 86/156) rusty_harness harness::models::llm::openai::tests::statuses_classify_into_retryable_and_not
        PASS [   0.011s] ( 87/156) rusty_harness harness::models::llm::openai::tests::streams_text_tool_calls_and_usage_and_sets_the_bearer_header
        PASS [   0.008s] ( 88/156) rusty_harness harness::models::llm::sse::tests::events_are_reassembled_across_chunks_and_done_ends_the_stream
        PASS [   0.008s] ( 89/156) rusty_harness harness::models::llm::sse::tests::a_trailing_event_without_a_blank_line_is_still_delivered
        PASS [   0.009s] ( 90/156) rusty_harness harness::models::llm::tests::collect_folds_events_into_a_turn
        PASS [   0.009s] ( 91/156) rusty_harness harness::models::llm::tests::a_request_with_an_image_is_refused_by_a_text_only_model_naming_the_modality
        PASS [   0.010s] ( 92/156) rusty_harness harness::models::llm::tests::retryable_errors_are_retried_with_backoff_and_non_retryable_are_not
        PASS [   0.011s] ( 93/156) rusty_harness harness::models::llm::tests::tool_names_go_out_as_the_providers_accept_and_come_back_as_ours
        PASS [   0.007s] ( 94/156) rusty_harness harness::models::llm::tests::unsupported_modality_is_a_typed_error
        PASS [   0.007s] ( 95/156) rusty_harness harness::models::speech::tests::a_session_error_is_a_model_error_and_other_formats_are_refused
        PASS [   0.007s] ( 96/156) rusty_harness harness::models::speech::tests::an_audio_part_carries_its_voice_meta_and_reads_back
        PASS [   0.009s] ( 97/156) rusty_harness harness::models::speech::tests::a_voice_session_frames_audio_and_sequences_the_models_chunks
        PASS [   0.009s] ( 98/156) rusty_harness harness::models::speech::tests::synthesis_streams_pcm_chunks_and_never_echoes_the_key_in_an_error
        PASS [   0.009s] ( 99/156) rusty_harness harness::models::speech::tests::transcription_streams_deltas_commits_after_the_audio_and_ends_on_the_final
        PASS [   0.417s] (100/156) rusty_harness harness::core_loop::agent::tests::completing_a_working_task_stops_in_flight_activities_and_ends_completed
        PASS [   0.010s] (101/156) rusty_harness harness::models::system_one::tests::the_http_provider_posts_the_judgement_and_checks_the_label
        PASS [   0.014s] (102/156) rusty_harness harness::models::system_one::tests::the_llm_provider_uses_the_classify_fragment_and_maps_the_answer
        PASS [   0.009s] (103/156) rusty_harness harness::plugins::tests::the_manifest_parses_keeps_unknown_fields_aside_and_validates_the_name
        PASS [   0.009s] (104/156) rusty_harness harness::skills::tests::invalid_skills_are_refused_naming_the_field
        PASS [   0.011s] (105/156) rusty_harness harness::skills::tests::the_front_matter_subset_parses_every_specified_field
        PASS [   0.011s] (106/156) rusty_harness harness::system_prompt::tests::a_configured_file_replaces_the_base_and_hooks_can_rewrite_the_plan
        PASS [   0.011s] (107/156) rusty_harness harness::system_prompt::tests::the_default_plan_is_base_plus_participants_rendered_from_fragments
        PASS [   0.009s] (108/156) rusty_harness harness::tools::host_mcp::tests::the_host_channel_client_initializes_pages_tool_lists_and_calls_tools
        PASS [   0.009s] (109/156) rusty_harness harness::tools::tests::a_pre_hook_veto_stops_tool_execution_and_the_model_is_told
        PASS [   0.009s] (110/156) rusty_harness harness::tools::tests::a_tool_result_above_the_size_limit_is_truncated_by_the_configured_policy
        PASS [   0.011s] (111/156) rusty_harness harness::tools::tests::abuse_denied_tool_call_is_not_executed_and_model_receives_a_tool_error
        PASS [   0.009s] (112/156) rusty_harness harness::tools::tests::unknown_tools_invalid_input_failures_and_suspensions_are_typed_outcomes
        PASS [   0.008s] (113/156) rusty_harness host::audio::tests::another_format_is_refused_before_a_device_is_opened
        PASS [   0.008s] (114/156) rusty_harness host::audio::tests::channels_mix_down_and_up
        PASS [   0.009s] (115/156) rusty_harness host::audio::tests::pcm16_round_trips_and_clips
        PASS [   0.011s] (116/156) rusty_harness host::audio::tests::resampling_keeps_the_duration_and_interpolates
        PASS [   0.007s] (117/156) rusty_harness host::native::tests::a_file_reference_under_tilde_is_under_home
        PASS [   0.008s] (118/156) rusty_harness host::native::tests::env_and_file_references_resolve_and_missing_ones_fail_by_name
        PASS [   0.010s] (119/156) rusty_harness host::native::tests::a_spawned_child_sees_only_the_listed_environment
        PASS [   0.009s] (120/156) rusty_harness host::native::tests::the_native_fs_writes_atomically_and_lists_sorted
        PASS [   0.009s] (121/156) rusty_harness host::secret::tests::a_secret_never_prints_its_value
        PASS [   0.007s] (122/156) rusty_harness host::secret::tests::literals_are_refused
        PASS [   0.007s] (123/156) rusty_harness host::secret::tests::references_parse_and_print_round_trip
        PASS [   0.031s] (124/156) rusty_harness host::native::tests::the_native_host_supplies_the_m1_set
        PASS [   0.008s] (125/156) rusty_harness host::testing::tests::an_unknown_reference_fails_by_name_never_by_value
        PASS [   0.008s] (126/156) rusty_harness host::testing::tests::the_seeded_rng_is_deterministic
        PASS [   0.007s] (127/156) rusty_harness host::tests::a_missing_capability_is_a_typed_error_at_lookup
        PASS [   0.007s] (128/156) rusty_harness host::tests::the_builder_supplies_what_it_was_given_and_nothing_else
        PASS [   0.008s] (129/156) rusty_harness interaction::renderer::a2ui::tests::a_part_names_the_media_type_twice_and_parses_back
        PASS [   0.009s] (130/156) rusty_harness interaction::events::tests::adapters_translate_bodies_into_inbox_posts_or_nothing
        PASS [   0.007s] (131/156) rusty_harness interaction::renderer::a2ui::tests::client_capabilities_read_either_version_key_and_write_the_current_one
        PASS [   0.007s] (132/156) rusty_harness interaction::renderer::a2ui::tests::an_unknown_version_is_refused_and_both_known_ones_are_read
        PASS [   0.008s] (133/156) rusty_harness interaction::renderer::a2ui::tests::client_messages_carry_actions_and_errors
        PASS [   0.009s] (134/156) rusty_harness interaction::renderer::a2ui::tests::dynamic_values_read_literals_paths_and_calls
        PASS [   0.009s] (135/156) rusty_harness interaction::renderer::a2ui::tests::the_specification_samples_round_trip
        PASS [   0.010s] (136/156) rusty_harness interaction::renderer::a2ui::tests::every_basic_component_round_trips
        PASS [   0.008s] (137/156) rusty_harness interaction::renderer::a2ui::tests::the_text_alternative_walks_the_tree_from_root
        PASS [   0.013s] (138/156) rusty_harness interaction::surface::tests::a_stream_event_is_negotiated_in_its_status_history_and_artifacts
        PASS [   0.009s] (139/156) rusty_harness interaction::surface::tests::a_surface_without_a2ui_gets_the_text_alternative_instead_of_the_data_part
        PASS [   0.008s] (140/156) rusty_harness interaction::surface::tests::accepts_matches_exactly_by_wildcard_and_always_for_text
        PASS [   0.010s] (141/156) rusty_harness interaction::surface::tests::an_alternative_already_in_the_message_is_not_duplicated_and_the_extension_goes
        PASS [   0.008s] (142/156) rusty_harness interaction::surface::tests::an_unrenderable_file_becomes_its_alt_text_then_its_name_then_the_fallback
        PASS [   0.008s] (143/156) rusty_harness interaction::surface::tests::renders_a2ui_needs_every_catalog_declared
        PASS [   0.012s] (144/156) rusty_harness interaction::surface::tests::the_surface_behind_a_message_is_read_from_its_a2ui_metadata
        PASS [   0.011s] (145/156) rusty_harness registry::tests::a_missing_entity_is_a_typed_error_naming_kind_and_entry
        PASS [   0.007s] (146/156) rusty_harness registry::tests::a_second_registration_of_the_same_identity_is_refused_and_names_the_holder
        PASS [   0.007s] (147/156) rusty_harness registry::tests::latest_picks_the_highest_version_and_list_is_ordered
        PASS [   0.008s] (148/156) rusty_harness registry::tests::registers_and_resolves_the_same_instance
        PASS [   0.010s] (149/156) rusty_harness registry::tests::replace_swaps_the_instance_and_returns_the_old_one
        PASS [   0.007s] (150/156) rusty_harness registry::tests::types_are_separate_maps
        PASS [   0.009s] (151/156) rusty_harness runtime::tokio::tests::sleep_waits_for_the_duration
        PASS [   0.011s] (152/156) rusty_harness runtime::tokio::tests::spawn_runs_the_future
        PASS [   0.007s] (153/156) rusty_harness sdk::tests::the_file_store_path_expands_home
        PASS [   0.007s] (154/156) rusty_harness tests::hello_greets
        PASS [   0.211s] (155/156) rusty_harness harness::tools::tests::several_tool_calls_run_concurrently_and_return_in_issue_order
        PASS [   1.292s] (156/156) rusty_harness app_service::durable::temporal::tests::the_recorded_history_replays_against_the_current_workflow
     Summary [   1.321s] 156 tests run: 156 passed, 0 skipped
```

## Doctests

```sh
cargo test --doc --workspace --all-features
```

Outcome: **passed** in 48s, 2026-10-25, at `0a97f5b`.

```text
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
running 2 tests
test crates/rusty-harness/src/lib.rs - hello (line 35) ... ok
test crates/rusty-harness/src/lib.rs - (line 8) ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
running 1 test
test crates/rusty-harness/src/host/secret.rs - host::secret::Secret (line 136) - compile fail ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s
```

## The web renderer (vitest)

```sh
bun run test
```

Outcome: **passed** in 5s, 2026-10-25, at `0a97f5b`.

```text
Test Files  7 passed (7)
      Tests  24 passed (24)
```
