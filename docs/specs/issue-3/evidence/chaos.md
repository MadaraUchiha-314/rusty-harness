# T13 — chaos and crash recovery

<!-- markdownlint-disable MD013 -->

Each `durable_chaos` scenario starts the ephemeral dev server and a second worker process (this test binary running an `#[ignore]`d victim), then kills the victim with SIGKILL mid-run. The plan's command was corrected in this run to name `server`: the outbound-reconciliation scenario needs an A2A peer, and with `durable-embedded` alone the file did not compile. It is now gated on both features.

## Chaos tests

```sh
cargo nextest run -p rusty_harness --features durable-embedded,server -E 'test(chaos_)' --no-fail-fast
```

Outcome: **passed** in 14s, 2026-10-25, at `0a97f5b`.

```text
Starting 5 tests across 29 binaries (210 tests skipped)
        PASS [   0.163s] (1/5) rusty_harness::chaos_store chaos_crash_between_store_writes_leaves_a_consistent_store_on_reopen
        PASS [   0.240s] (2/5) rusty_harness::chaos_loop chaos_restart_in_none_mode_resumes_from_the_last_persisted_iteration
        PASS [   6.769s] (3/5) rusty_harness::durable_chaos chaos_killing_the_worker_after_the_peer_accepted_and_before_the_id_was_recorded_creates_no_duplicate_remote_task
        PASS [   7.655s] (4/5) rusty_harness::durable_chaos chaos_killing_the_activity_worker_redispatches_the_model_call_once
        PASS [  14.327s] (5/5) rusty_harness::durable_chaos chaos_killing_the_workflow_worker_replays_without_a_repeated_tool_side_effect
     Summary [  14.492s] 5 tests run: 5 passed, 210 skipped
```
