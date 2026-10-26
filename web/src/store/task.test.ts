// Feature: Web renderer — the per-task store
// Requirement: docs/specs/issue-3/requirements.md#R26
import { describe, expect, it } from "vitest";
import type { Message, StreamResponse, Task } from "@a2a-js/sdk";
import {
  A2UI_MEDIA_TYPE,
  CHANNELS_EXT,
  PLAN_EXT,
  TASK_EXT,
  a2uiMessages,
  commandTokens,
  messageText,
  reduce,
  stateName,
} from "./task";

const TASK = "task_01";
const CTX = "ctx_1";

function message(
  id: string,
  role: 1 | 2,
  parts: Message["parts"],
  metadata?: Record<string, unknown>,
): Message {
  return {
    messageId: id,
    contextId: CTX,
    taskId: TASK,
    role,
    parts,
    metadata,
    extensions: [],
    referenceTaskIds: [],
  };
}

function text(value: string, metadata?: Record<string, unknown>): Message["parts"][number] {
  return { content: { $case: "text", value }, metadata, filename: "", mediaType: "" };
}

function status(
  state: number,
  msg: Message | undefined,
  metadata?: Record<string, unknown>,
  timestamp = "2026-10-24T12:31:00Z",
): StreamResponse {
  return {
    payload: {
      $case: "statusUpdate",
      value: {
        taskId: TASK,
        contextId: CTX,
        status: { state, message: msg, timestamp },
        metadata,
      },
    },
  };
}

function task(state: number, history: Message[] = []): StreamResponse {
  const value: Task = {
    id: TASK,
    contextId: CTX,
    status: { state, message: undefined, timestamp: "2026-10-24T12:30:00Z" },
    artifacts: [],
    history,
    metadata: {
      [TASK_EXT]: { name: "release notes", mode: "long-running", sub_tasks: [{ task: "sub-1" }] },
      [PLAN_EXT]: {
        id: "p1",
        task: TASK,
        version: 1,
        steps: [
          { id: "collect", name: "collect", description: "", status: "done" },
          { id: "draft", name: "draft", description: "", status: "running" },
        ],
      },
    },
  };
  return { payload: { $case: "task", value } };
}

function chunk(append: boolean, value: string): StreamResponse {
  return {
    payload: {
      $case: "artifactUpdate",
      value: {
        taskId: TASK,
        contextId: CTX,
        artifact: {
          artifactId: "a1",
          name: "notes.md",
          description: "",
          parts: [text(value)],
          metadata: undefined,
          extensions: [],
        },
        append,
        lastChunk: !append,
        metadata: undefined,
      },
    },
  };
}

describe("the task store", () => {
  it("reduces a task, its status updates and its metadata into one view", () => {
    let view = reduce(undefined, task(2, [message("m1", 1, [text("Draft release notes")])]));
    expect(view?.meta?.name).toBe("release notes");
    expect(view?.plan?.steps.map((s) => s.status)).toEqual(["done", "running"]);
    expect(view?.turns.map((t) => [t.role, t.text])).toEqual([["user", "Draft release notes"]]);
    view = reduce(
      view,
      status(6, message("m2", 2, [text("Done: the draft is attached.")]), {
        [TASK_EXT]: { sub_state: { kind: "awaiting-next-turn" } },
      }),
    );
    expect(stateName(view!.state)).toBe("input-required");
    expect(view?.subState?.kind).toBe("awaiting-next-turn");
    expect(view?.turns).toHaveLength(2);
    expect(view?.activity.map((a) => a.kind)).toEqual(["created", "state"]);
    // The same message again (a replay) is not duplicated.
    view = reduce(view, status(6, message("m2", 2, [text("Done: the draft is attached.")])));
    expect(view?.turns).toHaveLength(2);
  });

  it("keeps partial output per attempt and drops a retracted attempt's", () => {
    let view = reduce(undefined, task(2));
    view = reduce(
      view,
      status(2, message("p1", 2, [text("half an answer")], { [TASK_EXT]: { attempt: 1 } })),
    );
    view = reduce(view, status(2, undefined, { [TASK_EXT]: { retracted: [1] } }));
    expect(view?.turns).toEqual([]);
    expect(view?.retracted).toEqual([1]);
    view = reduce(
      view,
      status(6, message("p2", 2, [text("the whole answer")], { [TASK_EXT]: { attempt: 2 } })),
    );
    expect(view?.turns.map((t) => t.text)).toEqual(["the whole answer"]);
  });

  it("lifts A2UI parts out of an ask and hides their text alternative", () => {
    const a2ui = [{ version: "v0.9.1", createSurface: { surfaceId: "s", catalogId: "basic" } }];
    const ask = message(
      "ask",
      2,
      [
        text("Which audience?", { alternativeFor: A2UI_MEDIA_TYPE }),
        {
          content: { $case: "data", value: a2ui },
          metadata: { mimeType: A2UI_MEDIA_TYPE },
          filename: "",
          mediaType: A2UI_MEDIA_TYPE,
        },
      ],
      { [CHANNELS_EXT]: { channel: "ch-1", ask: "ch-1", question_kind: "a2ui" } },
    );
    expect(a2uiMessages(ask)).toEqual(a2ui);
    expect(messageText(ask)).toBe("");
    const view = reduce(reduce(undefined, task(2)), status(6, ask));
    const turn = view?.turns.at(-1);
    expect(turn?.a2ui).toEqual(a2ui);
    expect(turn?.channel?.channel).toBe("ch-1");
    expect(view?.activity.at(-2)).toMatchObject({ kind: "ask", detail: "ch-1" });
  });

  it("folds artifact chunks by id and appends when asked", () => {
    let view = reduce(reduce(undefined, task(2)), chunk(false, "# Notes"));
    view = reduce(view, chunk(true, "\n- one"));
    expect(view?.artifacts).toHaveLength(1);
    expect(view?.artifacts[0]?.parts).toHaveLength(2);
  });

  it("takes a plan/v1 artifact as the task's plan, not as an output", () => {
    const plan = {
      id: "plan-task",
      task: TASK,
      steps: [{ id: "draft", name: "Draft the notes", description: "", status: "done" }],
      edges: [],
      version: 2,
    };
    const update: StreamResponse = {
      payload: {
        $case: "artifactUpdate",
        value: {
          taskId: TASK,
          contextId: CTX,
          artifact: {
            artifactId: "plan-task",
            name: "plan",
            description: "",
            parts: [],
            metadata: { [PLAN_EXT]: plan },
            extensions: [PLAN_EXT],
          },
          append: false,
          lastChunk: true,
          metadata: undefined,
        },
      },
    };
    const view = reduce(reduce(undefined, task(2)), update);
    expect(view?.plan?.steps[0]?.name).toBe("Draft the notes");
    expect(view?.artifacts).toHaveLength(0);
  });

  it("finds registered commands at the start of a line only", () => {
    const commands = ["model", "effort", "exit"];
    expect(commandTokens("/model gpt", commands)).toEqual([{ start: 0, end: 6, name: "model" }]);
    expect(commandTokens("/modle gpt", commands)).toEqual([]);
    expect(commandTokens("say /model", commands)).toEqual([]);
    expect(commandTokens("a\n/exit", commands)).toEqual([{ start: 2, end: 7, name: "exit" }]);
  });
});
