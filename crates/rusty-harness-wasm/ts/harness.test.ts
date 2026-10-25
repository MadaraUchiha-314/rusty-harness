import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  Harness,
  type Config,
  type HostOptions,
  type SendMessageRequest,
  type StreamResponse,
} from "rusty-harness";
import { configDeclarations } from "../scripts/types";

const sse = [
  'data: {"choices":[{"delta":{"content":"Hello from "}}]}\n\n',
  'data: {"choices":[{"delta":{"content":"TypeScript."},"finish_reason":"stop"}]}\n\n',
  "data: [DONE]\n\n",
].join("");

const config: Config = {
  models: { default: { provider: "openai", model: "gpt-6.1", api_key: "env:OPENAI_API_KEY" } },
};

function mockedHost(calls: string[]): HostOptions {
  return {
    allowOrigins: ["https://api.openai.com"],
    fetch: async (url) => {
      calls.push(url);
      return new Response(sse, { headers: { "content-type": "text/event-stream" } });
    },
    secret: async (reference) => {
      if (reference !== "env:OPENAI_API_KEY") throw new Error(`unknown ${reference}`);
      return "sk-test";
    },
  };
}

function request(taskId: string): SendMessageRequest {
  return {
    message: {
      messageId: `m-${taskId}`,
      taskId,
      contextId: "c1",
      role: "ROLE_USER",
      parts: [{ text: "say hello" }],
    },
  };
}

function text(event: StreamResponse): string {
  const message = "message" in event ? event.message : undefined;
  return (message?.parts ?? []).map((p) => ("text" in p ? p.text : "")).join("");
}

describe("Harness", () => {
  /**
   * Feature: WASM and browser support
   * Requirement: docs/specs/issue-3/requirements.md#R29
   *
   * Scenario: The wasm build runs a task against a mocked host
   *     Given a harness created from a typed Config and a host with a mocked fetch
   *     When a message is sent through sendMessage
   *     Then the reply streams back as an AsyncIterable and the task completes
   */
  it("runs a task against a mocked host", async () => {
    const calls: string[] = [];
    const harness = await Harness.create(config, mockedHost(calls));
    const events: StreamResponse[] = [];
    for await (const event of harness.sendMessage(request("t1"))) events.push(event);
    expect(events.map(text).join("")).toContain("Hello from TypeScript.");
    const task = await harness.getTask("t1");
    expect(task.status.state).toBe("TASK_STATE_COMPLETED");
    expect(calls).toEqual(["https://api.openai.com/v1/chat/completions"]);

    const replay: StreamResponse[] = [];
    for await (const event of harness.subscribe("t1")) replay.push(event);
    expect(replay[0]).toHaveProperty("task.id", "t1");
  });

  it("takes the configuration as TOML text too", async () => {
    const toml = `[models.default]\nprovider = "openai"\nmodel = "gpt-6.1"\napi_key = "env:OPENAI_API_KEY"\n`;
    const harness = await Harness.create(toml, mockedHost([]));
    const events: StreamResponse[] = [];
    for await (const event of harness.sendMessage(request("t2"))) events.push(event);
    expect(events.map(text).join("")).toContain("Hello from TypeScript.");
  });

  it("rejects with the A2A code for an unknown task", async () => {
    const harness = await Harness.create(config, mockedHost([]));
    await expect(harness.cancelTask("missing")).rejects.toHaveProperty("code", -32001);
  });

  /**
   * Scenario: A credential with no page resolver fails the build closed
   *     Given a configuration naming env:OPENAI_API_KEY and a host with no secret callback
   *     When the harness is created
   *     Then creation rejects before any model call (R29.3, abuse 11)
   */
  it("fails closed without a secret resolver", async () => {
    const calls: string[] = [];
    const { secret: _, ...host } = mockedHost(calls);
    await expect(Harness.create(config, host)).rejects.toThrow(/SecretResolver|resolver/);
    expect(calls).toEqual([]);
  });
});

describe("contracts", () => {
  it("contract_typescript_declarations", async () => {
    const dts = readFileSync(
      join(import.meta.dirname, "..", "dist", "bundler", "rusty_harness.d.ts"),
      "utf8",
    );
    await expect(dts).toMatchFileSnapshot("__snapshots__/rusty_harness.d.ts");
  });

  it("contract_config_declarations: ts/config.d.ts matches the JSON schema", async () => {
    const committed = readFileSync(join(import.meta.dirname, "config.d.ts"), "utf8");
    expect(committed).toBe(await configDeclarations());
  });
});
