// The only thing that talks to the harness: the official A2A TypeScript SDK (R26.2).
// Every call goes through `@a2a-js/sdk`'s client; this module only shapes requests.

import { ClientFactory, type Client } from "@a2a-js/sdk/client";
import {
  CancelTaskRequest,
  SendMessageRequest,
  SubscribeToTaskRequest,
  type AgentCard,
  type StreamResponse,
  type Task,
} from "@a2a-js/sdk";
import { BASIC_CATALOG, CHANNELS_EXT, COMMANDS_EXT, TASK_EXT } from "@/store/task";

/** The A2UI client capabilities this renderer declares on every message (R25.4). */
export const A2UI_CLIENT_CAPABILITIES = {
  "v0.9.1": { supportedCatalogIds: [BASIC_CATALOG] },
};

/** A connected harness. */
export interface Harness {
  client: Client;
  card: AgentCard;
  /** Registered command names, from the card's `commands/v1` extension (R26.5). */
  commands: string[];
  /** The model aliases `/model` accepts, from the same extension entry. */
  models: string[];
}

const BUILTIN_COMMANDS = ["model", "effort", "exit"];

/** Connects to the harness at `url` through its agent card. */
export async function connect(url: string): Promise<Harness> {
  const client = await new ClientFactory().createFromUrl(url);
  const card = await client.getAgentCard();
  const commandsExt = card.capabilities?.extensions.find((e) => e.uri === COMMANDS_EXT);
  const listed = (commandsExt?.params as { commands?: { name: string }[] } | undefined)?.commands;
  const commands = listed?.length ? listed.map((c) => c.name) : BUILTIN_COMMANDS;
  const models = (commandsExt?.params as { models?: string[] } | undefined)?.models ?? [];
  return { client, card, commands, models };
}

/** What the person sends. */
export interface Outgoing {
  text?: string;
  parts?: Record<string, unknown>[];
  taskId?: string;
  contextId?: string;
  referenceTaskIds?: string[];
  metadata?: Record<string, unknown>;
}

/** The message JSON for `outgoing`: long-running by default, the A2UI capabilities on
 * every message, a reference to the last task when it starts a new one (R21.7). */
export function messageJson(outgoing: Outgoing): Record<string, unknown> {
  const parts = outgoing.parts ?? [{ text: outgoing.text ?? "" }];
  const metadata: Record<string, unknown> = {
    a2uiClientCapabilities: A2UI_CLIENT_CAPABILITIES,
    ...outgoing.metadata,
  };
  if (!outgoing.taskId) {
    const existing: unknown = metadata[TASK_EXT];
    metadata[TASK_EXT] = {
      mode: "long-running",
      ...(typeof existing === "object" && existing !== null ? existing : {}),
    };
  }
  return {
    messageId: crypto.randomUUID(),
    role: "ROLE_USER",
    parts,
    metadata,
    ...(outgoing.taskId ? { taskId: outgoing.taskId } : {}),
    ...(outgoing.contextId ? { contextId: outgoing.contextId } : {}),
    ...(outgoing.referenceTaskIds?.length ? { referenceTaskIds: outgoing.referenceTaskIds } : {}),
  };
}

/** `SendStreamingMessage`. */
export function send(client: Client, outgoing: Outgoing): AsyncGenerator<StreamResponse> {
  return client.sendMessageStream(SendMessageRequest.fromJSON({ message: messageJson(outgoing) }));
}

/** The task extension's complete message (R21.7). */
export function complete(
  client: Client,
  taskId: string,
  contextId: string,
): AsyncGenerator<StreamResponse> {
  return client.sendMessageStream(
    SendMessageRequest.fromJSON({
      message: {
        messageId: crypto.randomUUID(),
        role: "ROLE_USER",
        parts: [],
        taskId,
        contextId,
        metadata: { [TASK_EXT]: { transition: "complete" } },
      },
    }),
  );
}

/** `CancelTask`. */
export function cancel(client: Client, taskId: string): Promise<Task> {
  return client.cancelTask(CancelTaskRequest.fromJSON({ id: taskId }));
}

/** `SubscribeToTask`, for reconnects. */
export function resubscribe(client: Client, taskId: string): AsyncGenerator<StreamResponse> {
  return client.resubscribeTask(SubscribeToTaskRequest.fromJSON({ id: taskId }));
}

/** An A2UI client action as a message on the ask's channel (R25.4, R18). */
export function a2uiAction(
  action: Record<string, unknown>,
  channel: string | undefined,
  taskId: string,
  contextId: string,
): Outgoing {
  const outgoing: Outgoing = {
    parts: [
      {
        data: [{ version: "v0.9.1", action }],
        mediaType: "application/a2ui+json",
        metadata: { mimeType: "application/a2ui+json" },
      },
    ],
    taskId,
    contextId,
  };
  if (channel) {
    outgoing.metadata = {
      [CHANNELS_EXT]: { channel, ask: channel, reply_to: channel, question_kind: "a2ui" },
    };
  }
  return outgoing;
}
