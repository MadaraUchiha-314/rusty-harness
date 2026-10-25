// A scripted A2A harness for the browser tests: the agent card and the JSON-RPC 1.0
// methods the renderer calls, answering `SendStreamingMessage` with the event stream of
// one prototype state (R26.7). Only the wire is mocked; the page runs the real bundle
// against the real SDK.

import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { isJson, list, portOf, rec, type Json } from "./json";

const EXT = "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/";
export const TASK_EXT = `${EXT}task/v1`;
export const PARTICIPANTS_EXT = `${EXT}participants/v1`;
export const PLAN_EXT = `${EXT}plan/v1`;
export const CHANNELS_EXT = `${EXT}channels/v1`;
export const COMMANDS_EXT = `${EXT}commands/v1`;
export const A2UI_EXT = "https://a2ui.org/a2a-extension/a2ui/v0.9.1";
export const A2UI_MEDIA_TYPE = "application/a2ui+json";
export const BASIC_CATALOG = "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json";

export const TASK = "task_01HZX4V3K9N2";
export const CONTEXT = "ctx_9a1f";

export type { Json } from "./json";

/** What one turn of a scenario sends, and whether the stream then stays open. */
export interface Turn {
  events: Json[];
  /** Keep the SSE open after the events (a turn still streaming). */
  hold?: boolean;
  /** Milliseconds between events. */
  pace?: number;
}

/** A scenario: the turns answered in order to successive `SendStreamingMessage` calls. */
export type Scenario = (request: Json, index: number) => Turn;

export interface MockHarness {
  url: string;
  /** Every JSON-RPC request the page made, in order. */
  requests: Json[];
  close(): Promise<void>;
}

function card(url: string): Json {
  const parsed: unknown = JSON.parse(
    readFileSync(
      join(import.meta.dirname, "../../../crates/rusty-harness/tests/snapshots/agent_card.json"),
      "utf8",
    ),
  );
  const snapshot = rec(parsed);
  const interfaces: Json[] = [];
  for (const i of list(snapshot.supportedInterfaces)) interfaces.push({ ...i, url });
  return { ...snapshot, name: "demo-agent", url, supportedInterfaces: interfaces };
}

function cors(res: ServerResponse): void {
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Headers", "*");
  res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
  res.setHeader("Access-Control-Expose-Headers", "*");
}

function body(req: IncomingMessage): Promise<string> {
  return new Promise((resolve) => {
    let data = "";
    req.on("data", (chunk: Buffer) => {
      data += chunk.toString();
    });
    req.on("end", () => resolve(data));
  });
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** Starts the mock on a free port with `scenario`. */
export async function startMock(scenario: Scenario): Promise<MockHarness> {
  const requests: Json[] = [];
  let sends = 0;
  let lastTask: Json | undefined;
  const open = new Set<ServerResponse>();
  const server: Server = createServer(async (req, res) => {
    cors(res);
    if (req.method === "OPTIONS") {
      res.writeHead(204).end();
      return;
    }
    const base = `http://127.0.0.1:${portOf(server.address())}/`;
    if (req.method === "GET" && req.url?.endsWith("/.well-known/agent-card.json")) {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify(card(base)));
      return;
    }
    const parsed: unknown = JSON.parse(await body(req));
    const envelope = rec(parsed);
    const rpc = {
      id: envelope.id,
      method: typeof envelope.method === "string" ? envelope.method : "",
      params: isJson(envelope.params) ? envelope.params : {},
    };
    requests.push({ method: rpc.method, params: rpc.params });
    const reply = (result: unknown) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ jsonrpc: "2.0", id: rpc.id, result }));
    };
    switch (rpc.method) {
      case "SendStreamingMessage":
      case "SubscribeToTask": {
        const turn = scenario(rpc.params, sends);
        if (rpc.method === "SendStreamingMessage") sends += 1;
        res.writeHead(200, {
          "content-type": "text/event-stream",
          "cache-control": "no-cache",
          connection: "keep-alive",
        });
        open.add(res);
        for (const event of turn.events) {
          if (isJson(event.task)) lastTask = event.task;
          res.write(`data: ${JSON.stringify({ jsonrpc: "2.0", id: rpc.id, result: event })}\n\n`);
          // oxlint-disable-next-line no-await-in-loop -- the events are paced on purpose
          if (turn.pace) await sleep(turn.pace);
        }
        if (!turn.hold) {
          open.delete(res);
          res.end();
        }
        return;
      }
      case "GetTask":
        reply(
          lastTask ?? { id: TASK, contextId: CONTEXT, status: { state: "TASK_STATE_SUBMITTED" } },
        );
        return;
      case "CancelTask":
        reply({
          ...(lastTask ?? { id: TASK, contextId: CONTEXT }),
          status: { state: "TASK_STATE_CANCELED" },
        });
        return;
      case "ListTasks":
        reply({ tasks: lastTask ? [lastTask] : [] });
        return;
      default:
        res.writeHead(200, { "content-type": "application/json" });
        res.end(
          JSON.stringify({ jsonrpc: "2.0", id: rpc.id, error: { code: -32601, message: "no" } }),
        );
    }
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", () => r()));
  const url = `http://127.0.0.1:${portOf(server.address())}/`;
  return {
    url,
    requests,
    close: () =>
      new Promise<void>((r) => {
        for (const res of open) res.end();
        server.close(() => r());
      }),
  };
}

// ---------------------------------------------------------------------------
// The prototype's content (design/web-renderer.html)
// ---------------------------------------------------------------------------

export function taskMeta(subState?: Json): Json {
  return {
    name: "Plan the Q4 release notes",
    goal: "release notes for Q4",
    description: "",
    acceptance_criteria: [],
    mode: "long-running",
    sub_tasks: [{ task: "classify-25-prs" }],
    parent_tasks: [],
    reference_task_ids: [],
    ...(subState ? { sub_state: subState } : {}),
  };
}

export function plan(done: number, running: number): Json {
  const names = [
    "Collect merged PRs since last tag",
    "Group by area",
    "Draft notes per audience",
    "Review with reporter",
  ];
  return {
    id: "p1",
    task: TASK,
    version: 3,
    steps: names.map((name, i) => ({
      id: `s${i}`,
      name,
      description: "",
      status: i < done ? "done" : i < done + running ? "running" : "pending",
    })),
    edges: [],
  };
}

export const participants: Json = {
  participants: [
    { id: "you", display: "you", kind: { human: {} }, roles: ["reporter", "admin"] },
    { id: "demo-agent", display: "demo-agent", kind: "system", roles: ["assignee"] },
    { id: "sam", display: "sam", kind: { human: {} }, roles: ["watcher"] },
  ],
};

export function metadata(subState: Json | undefined, planValue: Json): Json {
  return {
    [TASK_EXT]: taskMeta(subState),
    [PARTICIPANTS_EXT]: participants,
    [PLAN_EXT]: planValue,
  };
}

export function agentMessage(id: string, text: string, extra: Json = {}): Json {
  return {
    messageId: id,
    role: "ROLE_AGENT",
    taskId: TASK,
    contextId: CONTEXT,
    parts: [{ text }],
    ...extra,
  };
}

export function taskEvent(state: string, history: Json[], when = "2026-10-24T12:30:00Z"): Json {
  // (the server's first event: the task with its message log so far)
  return {
    task: {
      id: TASK,
      contextId: CONTEXT,
      status: { state, timestamp: when },
      history,
      artifacts: [],
      metadata: metadata(undefined, plan(1, 1)),
    },
  };
}

export function statusEvent(
  state: string,
  message: Json | undefined,
  subState: Json | undefined,
  planValue: Json,
  when: string,
): Json {
  const meta = metadata(subState, planValue);
  return {
    statusUpdate: {
      taskId: TASK,
      contextId: CONTEXT,
      status: {
        state,
        timestamp: when,
        ...(message
          ? {
              message: {
                ...message,
                metadata: { ...(isJson(message.metadata) ? message.metadata : {}), ...meta },
              },
            }
          : {}),
      },
      metadata: meta,
    },
  };
}

export const REPLY =
  "Three areas changed since `v0.3.0`:\n\n- **API**: per-tenant rate limits (#401), pagination on `/events` (#404)\n- **Auth**: refresh-token rotation (#407)\n- **CLI**: the new `--config` flag";

export const DONE_REPLY =
  "Done: the draft is attached. Tell me what to change, or mark the task complete.";

export function artifactEvent(): Json {
  return {
    artifactUpdate: {
      taskId: TASK,
      contextId: CONTEXT,
      artifact: {
        artifactId: "a1",
        name: "release-notes-q4.md",
        description: "markdown",
        parts: [
          {
            text: "# Release notes Q4\n\n- API: per-tenant rate limits\n- Auth: refresh-token rotation",
            mediaType: "text/markdown",
          },
        ],
      },
      append: false,
      lastChunk: true,
    },
  };
}

/** The A2UI ask of the prototype (the same form task 33 snapshots). */
export function a2uiAsk(): Json {
  const components = [
    {
      id: "root",
      component: "Column",
      children: ["question", "detail", "audience", "other", "actions"],
    },
    {
      id: "question",
      component: "Text",
      text: "Which audience are these notes for?",
      variant: "h3",
    },
    {
      id: "detail",
      component: "Text",
      text: "The tone and the level of detail depend on it. Pick one, or type another.",
      variant: "body",
    },
    {
      id: "audience",
      component: "ChoicePicker",
      variant: "mutuallyExclusive",
      options: [
        { label: "Customers", value: "customers" },
        { label: "Internal engineering", value: "internal" },
        { label: "Both, two sections", value: "both" },
      ],
      value: { path: "/audience" },
    },
    { id: "other", component: "TextField", label: "Something else", value: { path: "/other" } },
    { id: "actions", component: "Row", children: ["send", "skip"] },
    {
      id: "send",
      component: "Button",
      child: "send_label",
      variant: "primary",
      action: {
        event: {
          name: "answer",
          context: { audience: { path: "/audience" }, other: { path: "/other" } },
        },
      },
    },
    { id: "send_label", component: "Text", text: "Send answer" },
    {
      id: "skip",
      component: "Button",
      child: "skip_label",
      variant: "borderless",
      action: { event: { name: "skip" } },
    },
    { id: "skip_label", component: "Text", text: "Skip" },
  ];
  const messages = [
    {
      version: "v0.9.1",
      createSurface: { surfaceId: "ask-ch-7f3a", catalogId: BASIC_CATALOG, sendDataModel: true },
    },
    { version: "v0.9.1", updateComponents: { surfaceId: "ask-ch-7f3a", components } },
    {
      version: "v0.9.1",
      updateDataModel: { surfaceId: "ask-ch-7f3a", value: { audience: ["customers"], other: "" } },
    },
  ];
  return agentMessage("msg_ask", "", {
    parts: [
      {
        text: "Which audience are these notes for?\nCustomers / Internal engineering / Both, two sections\nSomething else\nSend answer · Skip",
        metadata: { alternativeFor: A2UI_MEDIA_TYPE },
      },
      { data: messages, mediaType: A2UI_MEDIA_TYPE, metadata: { mimeType: A2UI_MEDIA_TYPE } },
    ],
    extensions: [A2UI_EXT],
    metadata: {
      [CHANNELS_EXT]: { channel: "ch-7f3a", ask: "ch-7f3a", to: ["you"], question_kind: "a2ui" },
      [A2UI_EXT]: {},
    },
  });
}

/** The scenarios, one per prototype state the wire can produce. */
/** The user's message as the server stores it (the task's history). */
function sent(request: Json): Json[] {
  const message = isJson(request.message) ? request.message : {};
  return [{ ...message, taskId: TASK, contextId: CONTEXT }];
}

const idle: Scenario = (request) => ({
  events: [
    taskEvent("TASK_STATE_WORKING", sent(request)),
    artifactEvent(),
    statusEvent(
      "TASK_STATE_INPUT_REQUIRED",
      agentMessage("m3", DONE_REPLY),
      { kind: "awaiting-next-turn" },
      plan(3, 1),
      "2026-10-24T12:35:00Z",
    ),
  ],
});

export const scenarios: Record<string, Scenario> = {
  // The reply is still arriving: the stream stays open after the text.
  streaming: (request) => ({
    events: [
      taskEvent("TASK_STATE_WORKING", sent(request)),
      statusEvent(
        "TASK_STATE_WORKING",
        agentMessage("m2", REPLY),
        undefined,
        plan(1, 1),
        "2026-10-24T12:31:00Z",
      ),
    ],
    hold: true,
  }),
  // Delivered: input-required, awaiting the next turn, with the artifact.
  idle,
  // The A2UI ask on a channel.
  a2ui: (request) => ({
    events: [
      taskEvent("TASK_STATE_WORKING", sent(request)),
      statusEvent(
        "TASK_STATE_INPUT_REQUIRED",
        a2uiAsk(),
        { kind: "awaiting-participant", ask: "ch-7f3a" },
        plan(2, 1),
        "2026-10-24T12:33:00Z",
      ),
    ],
  }),
  // An unknown command: the harness answers, the model is never called.
  unknown: (request) => ({
    events: [
      taskEvent("TASK_STATE_WORKING", sent(request)),
      statusEvent(
        "TASK_STATE_INPUT_REQUIRED",
        agentMessage("m4", "Unknown command /modle. Did you mean /model?"),
        { kind: "awaiting-next-turn" },
        plan(3, 1),
        "2026-10-24T12:36:00Z",
      ),
    ],
  }),
  // Delivered, then completed by the person (the second send is the transition).
  done: (_request, index) =>
    index === 0
      ? idle(_request, index)
      : {
          events: [
            statusEvent(
              "TASK_STATE_COMPLETED",
              undefined,
              undefined,
              plan(4, 0),
              "2026-10-24T12:41:00Z",
            ),
          ],
        },
};
