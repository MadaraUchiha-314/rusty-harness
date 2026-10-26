// One store per task: the A2A event stream reduced into turns, artifacts, status and
// the extension payloads, keyed by attempt so a `retracted` status update drops a failed
// attempt's partial output (design.md § Web renderer, R26.6, R22.4).

import {
  Role,
  TaskState,
  type Message,
  type Part,
  type StreamResponse,
  type Task,
} from "@a2a-js/sdk";

const EXT = "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/";
export const TASK_EXT = `${EXT}task/v1`;
export const PARTICIPANTS_EXT = `${EXT}participants/v1`;
export const PLAN_EXT = `${EXT}plan/v1`;
export const CHANNELS_EXT = `${EXT}channels/v1`;
export const COMMANDS_EXT = `${EXT}commands/v1`;
export const A2UI_EXT = "https://a2ui.org/a2a-extension/a2ui/v0.9.1";
export const A2UI_MEDIA_TYPE = "application/a2ui+json";
export const BASIC_CATALOG = "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json";

/** The task extension's sub-state (`task/v1`). */
export interface SubState {
  kind: string;
  ask?: string;
  reason?: string;
  task?: string;
}

/** The task extension payload the harness puts on tasks and status messages. */
export interface TaskMeta {
  name?: string;
  type?: string;
  goal?: string;
  description?: string;
  mode?: string;
  sub_tasks?: { task: string }[];
  parent_tasks?: { task: string }[];
  reference_task_ids?: string[];
  sub_state?: SubState | null;
  error?: { kind: string; message: string };
}

/** A participant (`participants/v1`). */
export interface Participant {
  id: string;
  display: string;
  kind: unknown;
  roles: string[];
}

/** A plan step (`plan/v1`). */
export interface PlanStep {
  id: string;
  name: string;
  description: string;
  status: "pending" | "running" | "done" | "skipped" | "failed";
  linked_tasks?: { task: string }[];
}

/** The plan (`plan/v1`). */
export interface Plan {
  id: string;
  task: string;
  steps: PlanStep[];
  version: number;
}

/** The channel payload on an ask or a reply (`channels/v1`). */
export interface ChannelsMeta {
  channel: string;
  ask: string;
  to?: string[];
  question_kind?: "text" | "a2ui";
  reply_to?: string;
  redacted?: boolean;
}

/** A command token to highlight (`commands/v1`, or derived from the card's list). */
export interface CommandToken {
  start: number;
  end: number;
  name: string;
}

/** One message turn. */
export interface Turn {
  id: string;
  role: "user" | "agent";
  parts: Part[];
  /** The text parts joined. */
  text: string;
  /** The attempt that produced it (0 when the harness did not say). */
  attempt: number;
  at?: string;
  channel?: ChannelsMeta;
  /** A2UI messages lifted out of the message's A2UI parts, in order. */
  a2ui: unknown[];
  /** Command outcome, when the message reports one. */
  commandResult?: { name: string; outcome: string };
}

/** An artifact, chunks folded. */
export interface ArtifactView {
  id: string;
  name: string;
  description: string;
  parts: Part[];
  attempt: number;
}

/** One line of the activity timeline. */
export interface Activity {
  at?: string;
  kind: "created" | "state" | "artifact" | "ask" | "retracted";
  detail: string;
}

/** The view of one task. */
export interface TaskView {
  id: string;
  contextId: string;
  state: TaskState;
  subState?: SubState;
  meta?: TaskMeta;
  participants: Participant[];
  plan?: Plan;
  turns: Turn[];
  artifacts: ArtifactView[];
  activity: Activity[];
  retracted: number[];
  updatedAt?: string;
}

type Metadata = Record<string, unknown> | undefined;

// Extension payloads are the wire's JSON, shaped by the schemas under `specs/a2a-ext/`;
// this is the one place the renderer names that shape.
// oxlint-disable-next-line typescript/no-unnecessary-type-parameters -- the caller names the payload type
function ext<T>(metadata: Metadata, key: string): T | undefined {
  const v: unknown = metadata?.[key];
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- schema-shaped wire JSON
  return v && typeof v === "object" ? (v as T) : undefined;
}

/** The attempt an event belongs to, when the harness tagged it. */
export function attemptOf(metadata: Metadata): number {
  const meta = ext<{ attempt?: unknown }>(metadata, TASK_EXT);
  return typeof meta?.attempt === "number" ? meta.attempt : 0;
}

/** The text of a part, if it is one. */
export function partText(part: Part): string | undefined {
  return part.content?.$case === "text" ? part.content.value : undefined;
}

/** Whether a part is an A2UI data part (media type or the extension's `mimeType`). */
export function isA2ui(part: Part): boolean {
  if (part.content?.$case !== "data") return false;
  const mime: unknown = part.metadata?.mimeType;
  return part.mediaType === A2UI_MEDIA_TYPE || mime === A2UI_MEDIA_TYPE;
}

/** The A2UI messages across a message's A2UI parts. */
export function a2uiMessages(message: Message): unknown[] {
  const out: unknown[] = [];
  for (const part of message.parts) {
    if (!isA2ui(part) || part.content?.$case !== "data") continue;
    const data: unknown = part.content.value;
    if (Array.isArray(data)) out.push(...data);
  }
  return out;
}

/** Whether a text part stands in for a part the surface renders itself. */
export function isAlternativeFor(part: Part, mediaType: string): boolean {
  const alt: unknown = part.metadata?.alternativeFor;
  return alt === mediaType;
}

/** The text of a message: its text parts, minus the alternatives of A2UI parts the
 * renderer shows itself. */
export function messageText(message: Message): string {
  const hasA2ui = message.parts.some(isA2ui);
  return message.parts
    .filter((p) => !(hasA2ui && isAlternativeFor(p, A2UI_MEDIA_TYPE)))
    .map(partText)
    .filter((t): t is string => t !== undefined)
    .join("\n");
}

/** Registered commands at the start of a line, as byte ranges to highlight (R26.5). */
export function commandTokens(text: string, commands: readonly string[]): CommandToken[] {
  const out: CommandToken[] = [];
  let offset = 0;
  for (const line of text.split(/(?<=\n)/)) {
    const m = /^(\s*)\/([A-Za-z0-9_-]+)/.exec(line);
    if (m && commands.includes(m[2])) {
      const start = offset + m[1].length;
      out.push({ start, end: start + 1 + m[2].length, name: m[2] });
    }
    offset += line.length;
  }
  return out;
}

function turnOf(message: Message, at: string | undefined): Turn {
  const commands = ext<{ result?: { name: string; outcome: string } }>(
    message.metadata,
    COMMANDS_EXT,
  );
  const turn: Turn = {
    id: message.messageId,
    role: message.role === Role.ROLE_USER ? "user" : "agent",
    parts: message.parts,
    text: messageText(message),
    attempt: attemptOf(message.metadata),
    a2ui: a2uiMessages(message),
  };
  if (at) turn.at = at;
  const channel = ext<ChannelsMeta>(message.metadata, CHANNELS_EXT);
  if (channel) turn.channel = channel;
  if (commands?.result) turn.commandResult = commands.result;
  return turn;
}

function fresh(id: string, contextId: string, state: TaskState): TaskView {
  return {
    id,
    contextId,
    state,
    participants: [],
    turns: [],
    artifacts: [],
    activity: [],
    retracted: [],
  };
}

function withMetadata(view: TaskView, metadata: Metadata): TaskView {
  const meta = ext<TaskMeta>(metadata, TASK_EXT);
  const participants = ext<{ participants: Participant[] }>(metadata, PARTICIPANTS_EXT);
  const plan = ext<Plan>(metadata, PLAN_EXT);
  const next = { ...view };
  if (meta) {
    next.meta = { ...view.meta, ...meta };
    if ("sub_state" in meta) {
      if (meta.sub_state) next.subState = meta.sub_state;
      else delete next.subState;
    }
  }
  if (participants?.participants) next.participants = participants.participants;
  if (plan?.steps) next.plan = plan;
  return next;
}

function pushTurn(view: TaskView, message: Message | undefined, at: string | undefined): TaskView {
  if (!message || view.turns.some((t) => t.id === message.messageId && message.messageId)) {
    return view;
  }
  const turn = turnOf(message, at);
  const activity: Activity[] = [...view.activity];
  if (turn.channel && turn.role === "agent") {
    activity.push({
      at,
      kind: "ask",
      detail: turn.channel.channel,
    });
  }
  return { ...view, turns: [...view.turns, turn], activity };
}

function retract(view: TaskView, attempts: number[]): TaskView {
  if (attempts.length === 0) return view;
  const dropped = new Set(attempts);
  return {
    ...view,
    turns: view.turns.filter((t) => !dropped.has(t.attempt)),
    artifacts: view.artifacts.filter((a) => !dropped.has(a.attempt)),
    retracted: [...view.retracted, ...attempts],
    activity: [...view.activity, { kind: "retracted", detail: attempts.join(",") }],
  };
}

function applyTask(view: TaskView | undefined, task: Task, now: string | undefined): TaskView {
  const state = task.status?.state ?? TaskState.TASK_STATE_UNSPECIFIED;
  let next =
    view && view.id === task.id ? { ...view, state } : fresh(task.id, task.contextId, state);
  next = withMetadata(next, task.metadata);
  if (next.activity.length === 0) {
    next.activity = [
      { at: task.status?.timestamp ?? now, kind: "created", detail: stateName(state) },
    ];
  }
  for (const m of task.history) next = pushTurn(next, m, undefined);
  if (task.status?.message) {
    next = withMetadata(next, task.status.message.metadata);
    next = pushTurn(next, task.status.message, task.status.timestamp);
  }
  for (const a of task.artifacts) next = foldArtifact(next, a, false);
  if (task.status?.timestamp) next.updatedAt = task.status.timestamp;
  return next;
}

function foldArtifact(
  view: TaskView,
  artifact: Task["artifacts"][number],
  append: boolean,
): TaskView {
  const attempt = attemptOf(artifact.metadata);
  const existing = view.artifacts.find((a) => a.id === artifact.artifactId);
  const parts = existing && append ? [...existing.parts, ...artifact.parts] : artifact.parts;
  const entry: ArtifactView = {
    id: artifact.artifactId,
    name: artifact.name || artifact.artifactId,
    description: artifact.description,
    parts,
    attempt,
  };
  const artifacts = existing
    ? view.artifacts.map((a) => (a.id === entry.id ? entry : a))
    : [...view.artifacts, entry];
  const activity = existing
    ? view.activity
    : [...view.activity, { kind: "artifact" as const, detail: entry.name }];
  return { ...view, artifacts, activity };
}

/** The A2A state as the renderers' vocabulary. */
export function stateName(state: TaskState): string {
  switch (state) {
    case TaskState.TASK_STATE_SUBMITTED:
      return "submitted";
    case TaskState.TASK_STATE_WORKING:
      return "working";
    case TaskState.TASK_STATE_COMPLETED:
      return "completed";
    case TaskState.TASK_STATE_FAILED:
      return "failed";
    case TaskState.TASK_STATE_CANCELED:
      return "canceled";
    case TaskState.TASK_STATE_INPUT_REQUIRED:
      return "input-required";
    case TaskState.TASK_STATE_REJECTED:
      return "rejected";
    case TaskState.TASK_STATE_AUTH_REQUIRED:
      return "auth-required";
    default:
      return "unknown";
  }
}

/** Whether a state is terminal. */
export function isTerminal(state: TaskState): boolean {
  return (
    state === TaskState.TASK_STATE_COMPLETED ||
    state === TaskState.TASK_STATE_FAILED ||
    state === TaskState.TASK_STATE_CANCELED ||
    state === TaskState.TASK_STATE_REJECTED
  );
}

/** Whether the task waits for the person. */
export function isInputRequired(state: TaskState): boolean {
  return state === TaskState.TASK_STATE_INPUT_REQUIRED;
}

/** Whether the task is working. */
export function isWorking(state: TaskState): boolean {
  return state === TaskState.TASK_STATE_WORKING;
}

/** Applies one stream event to the view of its task (a new view for a new task). */
export function reduce(
  view: TaskView | undefined,
  response: StreamResponse,
  now?: string,
): TaskView | undefined {
  const payload = response.payload;
  if (!payload) return view;
  switch (payload.$case) {
    case "task":
      return applyTask(view, payload.value, now);
    case "statusUpdate": {
      const u = payload.value;
      const state = u.status?.state ?? view?.state ?? TaskState.TASK_STATE_UNSPECIFIED;
      let next = view && view.id === u.taskId ? { ...view } : fresh(u.taskId, u.contextId, state);
      const changed = next.state !== state;
      next.state = state;
      next = withMetadata(next, u.metadata);
      const retracted = ext<{ retracted?: number[] }>(u.metadata, TASK_EXT)?.retracted;
      if (retracted) next = retract(next, retracted);
      if (u.status?.message) {
        next = withMetadata(next, u.status.message.metadata);
        next = pushTurn(next, u.status.message, u.status.timestamp ?? now);
      }
      if (changed || next.activity.length === 0) {
        next.activity = [
          ...next.activity,
          { at: u.status?.timestamp ?? now, kind: "state", detail: stateName(state) },
        ];
      }
      if (u.status?.timestamp) next.updatedAt = u.status.timestamp;
      return next;
    }
    case "artifactUpdate": {
      const u = payload.value;
      const base =
        view && view.id === u.taskId ? view : fresh(u.taskId, u.contextId, view?.state ?? 0);
      return u.artifact ? foldArtifact(base, u.artifact, u.append) : base;
    }
    case "message": {
      const m = payload.value;
      const base = view ?? fresh(m.taskId, m.contextId, 0);
      return pushTurn(base, m, now);
    }
    default:
      return view;
  }
}
