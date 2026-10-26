# Learning 004 — Project a stream's events with the viewer's membership at each event

- **Date:** 2026-10-25
- **Source:** system-feedback (the TUI's one-harness scenario, issue #3 task 37)
- **Category:** design

## What happened

`SendStreamingMessage` computed the caller's channel memberships once, right after the
message was accepted, and projected every later event with that snapshot. An ask the
loop opened during that very turn created a channel the caller belonged to, but the
snapshot predated it, so the person the question was addressed to received the
redacted status in their own stream. Every earlier test asked on a channel that existed
before the stream started, or read the task afterwards, so the gap stayed hidden until
a renderer consumed the stream end to end.

## Learning

Anything decided per viewer and able to change during a stream — membership, roles, a
redaction rule — belongs to the moment each event is emitted. Re-read it per event
(`Channel::memberships` is one store read) and re-project the event; do not hoist it
out of the loop for efficiency. Drive a surface through the real stream in a test: the
snapshot-then-read tests cannot see this class of bug.

## Applied in

`app_service/a2a_server` (`execute`'s tail re-projects each event),
`crates/rusty-harness/tests/tui.rs::on_one_harness`, [capability: the A2A
server](../capabilities/server.md).
