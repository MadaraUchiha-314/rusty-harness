# Learning 002: a signal in a run's tail is lost; CI's slower disk found it

- **Date:** 2026-10-25
- **Source:** user-feedback
- **Work item:** [#3](https://github.com/MadaraUchiha-314/rusty-harness/issues/3) (M3/M4)

## What happened

`get_task_after_a_restart_returns_the_full_a2a_task` stalled CI builds for over 17
minutes while passing locally in a fifth of a second, even pinned to one CPU. The
operator asked for it to be fixed before anything else. A nextest termination policy plus
a stderr trace in the test showed the second message being accepted 110 ms before the
first run returned: the in-process mode saw the task "running" and spawned nothing, the
loop was past its last boundary, and the signal was lost. Locally the window is
sub-millisecond.

## Learning

A liveness race that needs a slow disk to open does not reproduce by pinning CPUs; the
fastest route was to make CI tell: terminate a stalled test and keep its captured output,
then trace the loop in the failing test. The bug itself was a classic "check then spawn"
race around a running-set, and the same shape existed in the workflow's wait.

## Action

- The `none` mode re-runs a task whose run ended waiting when the inbox holds an unapplied
  message; the workflow waits relative to the signals seen at the iteration's start.
- `.config/nextest.toml` terminates a test after five minutes; the restart test keeps its
  stderr trace, so the next stall reports where it stopped.
