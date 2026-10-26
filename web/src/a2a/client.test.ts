// Feature: Web renderer — requests through the official SDK only
// Requirement: docs/specs/issue-3/requirements.md#R26
import { describe, expect, it } from "vitest";
import { A2UI_CLIENT_CAPABILITIES, a2uiAction, messageJson } from "./client";
import { CHANNELS_EXT, TASK_EXT } from "@/store/task";

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

function rec(v: unknown): Record<string, unknown> {
  if (!isRecord(v)) throw new Error(`not an object: ${String(v)}`);
  return v;
}

function list(v: unknown): Record<string, unknown>[] {
  if (!Array.isArray(v)) throw new Error("not a list");
  return v.map(rec);
}

describe("outgoing messages", () => {
  it("start long-running tasks, declare the A2UI catalog and reference a finished task", () => {
    const first = messageJson({ text: "hello" });
    expect(first.parts).toEqual([{ text: "hello" }]);
    expect(rec(first.metadata)[TASK_EXT]).toEqual({ mode: "long-running" });
    expect(rec(first.metadata).a2uiClientCapabilities).toEqual(A2UI_CLIENT_CAPABILITIES);
    expect(first.taskId).toBeUndefined();

    const next = messageJson({ text: "more", taskId: "t1", contextId: "c1" });
    expect(next.taskId).toBe("t1");
    expect(rec(next.metadata)[TASK_EXT]).toBeUndefined();

    const follow = messageJson({ text: "again", contextId: "c1", referenceTaskIds: ["t1"] });
    expect(follow.referenceTaskIds).toEqual(["t1"]);
    expect(rec(follow.metadata)[TASK_EXT]).toEqual({
      mode: "long-running",
    });
  });

  it("sends an A2UI action as a client message on the ask's channel", () => {
    const out = a2uiAction(
      {
        name: "answer",
        surfaceId: "s",
        sourceComponentId: "send",
        timestamp: "2026-01-01T00:00:00Z",
        context: { audience: ["customers"] },
      },
      "ch-1",
      "t1",
      "c1",
    );
    const json = messageJson(out);
    const part = list(json.parts)[0];
    expect(part?.mediaType).toBe("application/a2ui+json");
    expect(list(part?.data)[0]).toMatchObject({
      version: "v0.9.1",
      action: { name: "answer", context: { audience: ["customers"] } },
    });
    expect(rec(json.metadata)[CHANNELS_EXT]).toEqual({
      channel: "ch-1",
      ask: "ch-1",
      reply_to: "ch-1",
      question_kind: "a2ui",
    });
  });
});
