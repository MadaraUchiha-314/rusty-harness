// Feature: A2A API server
// Requirement: docs/specs/issue-3/requirements.md#R19
//
// Scenario: The official TypeScript SDK drives every A2A 1.0 operation against the server
// Scenario: A request with A2A-Version 0.3 is rejected
//
// Boots the `rusty-harness` binary against a mock OpenAI endpoint served from this test
// and drives it with `@a2a-js/sdk` (R19.5, R31.3). No hand-rolled client.

import { ClientFactory } from "@a2a-js/sdk/client";
import {
  CancelTaskRequest,
  GetExtendedAgentCardRequest,
  GetTaskPushNotificationConfigRequest,
  GetTaskRequest,
  ListTasksRequest,
  SendMessageRequest,
  TaskState,
  type Task,
} from "@a2a-js/sdk";
import { spawn, type ChildProcess } from "node:child_process";
import { createServer, type Server } from "node:http";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const REPO = join(import.meta.dirname, "..", "..");
const TASK_EXT = "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/task/v1";

let mock: Server;
let child: ChildProcess;
let base = "";

function sse(lines: string[]): string {
  return lines.map((l) => `data: ${l}\n\n`).join("") + "data: [DONE]\n\n";
}

async function listen(server: Server): Promise<number> {
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", () => r()));
  const addr = server.address();
  if (!addr || typeof addr === "string") throw new Error("no port");
  return addr.port;
}

async function freePort(): Promise<number> {
  const s = createServer();
  const port = await listen(s);
  await new Promise<void>((r) => s.close(() => r()));
  return port;
}

async function waitFor(url: string, ms: number): Promise<void> {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    try {
      // oxlint-disable-next-line no-await-in-loop -- polling until the server answers is sequential by nature
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      /* not up yet */
    }
    // oxlint-disable-next-line no-await-in-loop -- the back-off between polls
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`${url} never answered`);
}

beforeAll(async () => {
  // A mock OpenAI chat-completions endpoint: every call streams one short answer.
  let calls = 0;
  mock = createServer((req, res) => {
    calls += 1;
    res.writeHead(200, { "content-type": "text/event-stream" });
    const chunks = [
      JSON.stringify({ choices: [{ delta: { content: `answer ${calls}` } }] }),
      JSON.stringify({
        choices: [{ delta: {}, finish_reason: "stop" }],
        usage: { prompt_tokens: 10, completion_tokens: 2 },
      }),
    ];
    res.end(sse(chunks));
  });
  const mockPort = await listen(mock);
  const port = await freePort();
  const dir = mkdtempSync(join(tmpdir(), "rh-contract-"));
  writeFileSync(
    join(dir, "config.toml"),
    [
      `[agent]`,
      `name = "contract-agent"`,
      `description = "Drives the A2A conformance suite."`,
      `[models.default]`,
      `provider = "openai"`,
      `model = "mock"`,
      `api_key = "env:RH_CONTRACT_KEY"`,
      `base_url = "http://127.0.0.1:${mockPort}/v1"`,
      `[server]`,
      `bind = "127.0.0.1:${port}"`,
      ``,
    ].join("\n"),
  );
  child = spawn(
    "cargo",
    [
      "run",
      "-q",
      "-p",
      "rusty_harness",
      "--features",
      "cli",
      "--bin",
      "rusty-harness",
      "--",
      "--config",
      join(dir, "config.toml"),
    ],
    {
      cwd: REPO,
      env: { ...process.env, RH_CONTRACT_KEY: "sk-contract", RUST_LOG: "warn" },
      stdio: ["ignore", "inherit", "inherit"],
    },
  );
  base = `http://127.0.0.1:${port}/`;
  await waitFor(`${base}.well-known/agent-card.json`, 600_000);
}, 620_000);

afterAll(async () => {
  child?.kill("SIGTERM");
  await new Promise<void>((r) => mock.close(() => r()));
});

/// A user message in ProtoJSON, the form the SDK's `fromJSON` constructors take.
function userMessage(text: string, taskId?: string, longRunning = false): Record<string, unknown> {
  return {
    messageId: crypto.randomUUID(),
    role: "ROLE_USER",
    parts: [{ text }],
    ...(taskId ? { taskId } : {}),
    ...(longRunning ? { metadata: { [TASK_EXT]: { mode: "long-running" } } } : {}),
  };
}

describe("A2A 1.0 conformance over JSON-RPC", () => {
  it("drives every operation with the official SDK", async () => {
    const client = await new ClientFactory().createFromUrl(base);
    const card = await client.getAgentCard();
    expect(card.name).toBe("contract-agent");
    const uris = (card.capabilities?.extensions ?? []).map((e) => e.uri);
    for (const name of ["task", "participants", "plan", "channels", "commands", "voice"]) {
      expect(uris.some((u) => u.endsWith(`/${name}/v1`))).toBe(true);
    }

    const sent = await client.sendMessage(
      SendMessageRequest.fromJSON({ message: userMessage("hello", undefined, true) }),
    );
    if (!("id" in sent)) throw new Error(`expected a task: ${JSON.stringify(sent)}`);
    const task: Task = sent;
    expect(task.id).toBeTruthy();
    expect(task.status?.state).toBe(TaskState.TASK_STATE_INPUT_REQUIRED);
    expect(task.metadata?.[TASK_EXT]).toBeTruthy();

    const kinds: string[] = [];
    for await (const event of client.sendMessageStream(
      SendMessageRequest.fromJSON({ message: userMessage("again", task.id) }),
    )) {
      kinds.push(event.payload?.$case ?? "unknown");
    }
    expect(kinds.length).toBeGreaterThan(1);
    expect(kinds[0]).toBe("task");
    expect(kinds).toContain("statusUpdate");

    const got = await client.getTask(GetTaskRequest.fromJSON({ id: task.id }));
    expect(got.id).toBe(task.id);
    expect((got.history ?? []).length).toBeGreaterThanOrEqual(2);

    const listed = await client.listTasks(ListTasksRequest.fromJSON({ contextId: task.contextId }));
    expect(listed.tasks.length).toBe(1);

    // Push configs are refused until the operator allows webhook hosts (task 23).
    await expect(async () =>
      client.getTaskPushNotificationConfig(
        GetTaskPushNotificationConfigRequest.fromJSON({ taskId: task.id, id: "none" }),
      ),
    ).rejects.toBeTruthy();

    const canceled = await client.cancelTask(CancelTaskRequest.fromJSON({ id: task.id }));
    expect(canceled.status?.state).toBe(TaskState.TASK_STATE_CANCELED);
    await expect(async () =>
      client.getExtendedAgentCard(GetExtendedAgentCardRequest.fromJSON({})),
    ).rejects.toBeTruthy();
  }, 120_000);

  it("rejects a request naming A2A-Version 0.3", async () => {
    const res = await fetch(base, {
      method: "POST",
      headers: { "content-type": "application/json", "A2A-Version": "0.3" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "GetTask", params: { id: "nope" } }),
    });
    expect(await res.text()).toMatch(/version/i);
  });
});
