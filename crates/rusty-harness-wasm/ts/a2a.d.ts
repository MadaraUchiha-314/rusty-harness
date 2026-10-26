// The A2A 1.0 JSON binding the harness speaks: the shapes of the official `a2a` Rust
// types on the wire (field-presence unions, upper-case enum names, base64 `raw` bytes).

/** Who sent a message. */
export type Role = "ROLE_USER" | "ROLE_AGENT" | "ROLE_UNSPECIFIED";

/** A task's lifecycle state. */
export type TaskState =
  | "TASK_STATE_UNSPECIFIED"
  | "TASK_STATE_SUBMITTED"
  | "TASK_STATE_WORKING"
  | "TASK_STATE_COMPLETED"
  | "TASK_STATE_FAILED"
  | "TASK_STATE_CANCELED"
  | "TASK_STATE_INPUT_REQUIRED"
  | "TASK_STATE_REJECTED"
  | "TASK_STATE_AUTH_REQUIRED";

/** Fields every part may carry. */
export interface PartCommon {
  filename?: string;
  mediaType?: string;
  metadata?: Record<string, unknown>;
}

/** One piece of content: exactly one of `text`, `raw` (base64), `url` or `data`. */
export type Part = PartCommon &
  ({ text: string } | { raw: string } | { url: string } | { data: unknown });

/** A message between a person and the agent. */
export interface Message {
  messageId: string;
  contextId?: string;
  taskId?: string;
  role: Role;
  parts: Part[];
  metadata?: Record<string, unknown>;
  extensions?: string[];
  referenceTaskIds?: string[];
}

/** A task's status. */
export interface TaskStatus {
  state: TaskState;
  message?: Message;
  /** RFC 3339. */
  timestamp?: string;
}

/** An output of a task. */
export interface Artifact {
  artifactId: string;
  name?: string;
  description?: string;
  parts: Part[];
  metadata?: Record<string, unknown>;
  extensions?: string[];
}

/** A unit of work. */
export interface Task {
  id: string;
  contextId: string;
  status: TaskStatus;
  artifacts?: Artifact[];
  history?: Message[];
  metadata?: Record<string, unknown>;
}

/** A task's status changed. */
export interface TaskStatusUpdateEvent {
  taskId: string;
  contextId: string;
  status: TaskStatus;
  metadata?: Record<string, unknown>;
}

/** A task produced (part of) an artifact. */
export interface TaskArtifactUpdateEvent {
  taskId: string;
  contextId: string;
  artifact: Artifact;
  append?: boolean;
  lastChunk?: boolean;
  metadata?: Record<string, unknown>;
}

/** One event of a turn: exactly one of the four. */
export type StreamResponse =
  | { task: Task }
  | { message: Message }
  | { statusUpdate: TaskStatusUpdateEvent }
  | { artifactUpdate: TaskArtifactUpdateEvent };

/** How the harness should answer. */
export interface SendMessageConfiguration {
  acceptedOutputModes?: string[];
  historyLength?: number;
  returnImmediately?: boolean;
}

/** `sendMessage`'s argument. */
export interface SendMessageRequest {
  message: Message;
  configuration?: SendMessageConfiguration;
  metadata?: Record<string, unknown>;
  tenant?: string;
}

/** A rejected call: an `Error` with the A2A JSON-RPC error `code`. */
export interface A2AError extends Error {
  code: number;
}
