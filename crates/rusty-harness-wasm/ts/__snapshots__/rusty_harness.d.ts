/* tslint:disable */
/* eslint-disable */

// Generated from crates/rusty-harness/rusty-harness.schema.json by scripts/types.ts.
// Do not edit: run `bun run types`.


/**
 * A credential by reference: env:NAME, keyring:SERVICE/USER or file:PATH. Never a literal value.
 */
export type SecretRef = string
/**
 * A duration: a number followed by ms, s, m or h, e.g. 30s or 15m.
 */
export type Duration = string
/**
 * `[server.security]`.
 */
export type SecuritySchemeConfig = ({
    scheme: "bearer"
    /**
     * A credential by reference: env:NAME, keyring:SERVICE/USER or file:PATH. Never a literal value.
     */
    token: string
} | {
    /**
     * The header name.
     */
    header: string
    /**
     * A credential by reference: env:NAME, keyring:SERVICE/USER or file:PATH. Never a literal value.
     */
    key: string
    scheme: "api-key"
})

/**
 * The whole configuration (design.md § Data models § Configuration).
 */
export interface Config {
    agent?: AgentConfig
    /**
     * Sub-agents by name.
     */
    agents?: {
        [k: string]: SubAgentConfig | undefined
    }
    context?: ContextConfig
    durable?: DurableConfig
    hooks?: HooksConfig
    /**
     * The locale for user-facing strings (R33.8); `LANG` then `en` when absent.
     */
    locale?: (string | null)
    loop?: LoopConfig
    /**
     * Language models by alias; `default` is the one the loop starts with.
     */
    models?: {
        [k: string]: ModelConfig | undefined
    }
    o11y?: O11YConfig
    persistence?: PersistenceConfig
    plugins?: PluginsConfig
    server?: ServerConfig
    /**
     * System One models by alias.
     */
    system_one?: {
        [k: string]: SystemOneConfig | undefined
    }
    tools?: ToolsConfig
    tui?: TuiConfig
    voice?: VoiceConfig
}
/**
 * The agent this harness serves.
 */
export interface AgentConfig {
    /**
     * One sentence for the agent card.
     */
    description?: string
    /**
     * The agent's name, as the agent card shows it.
     */
    name?: string
    /**
     * A prompt fragment reference (`file:` or a registered id) replacing the default
     * system prompt (R7.2).
     */
    system_prompt?: (string | null)
}
/**
 * `[agents.<name>]`.
 */
export interface SubAgentConfig {
    /**
     * The credential to present, by reference.
     */
    api_key?: (SecretRef | null)
    /**
     * Where the sub-agent's A2A agent card is served.
     */
    card_url: string
}
/**
 * The context window.
 */
export interface ContextConfig {
    compaction?: CompactionConfig
    /**
     * The positions in order (R15.1).
     */
    order?: string[]
}
/**
 * Compaction (R16).
 */
export interface CompactionConfig {
    /**
     * Recent turns kept verbatim (R16.2).
     */
    keep_recent_turns?: number
    /**
     * The model alias that summarises; the active model when absent.
     */
    model?: (string | null)
    /**
     * Compact when the estimate exceeds this fraction of the window (R16.1).
     */
    threshold?: number
}
/**
 * Durable execution.
 */
export interface DurableConfig {
    /**
     * The mode.
     */
    mode?: ("none" | "embedded" | "remote")
    retry?: RetryConfig
    temporal?: TemporalConfig
}
/**
 * Activity retries (R22.3).
 */
export interface RetryConfig {
    /**
     * Multiplier per attempt.
     */
    coefficient?: number
    /**
     * The first wait.
     */
    initial?: string
    /**
     * Attempts in total; always finite.
     */
    max_attempts?: number
    /**
     * The longest wait.
     */
    max_interval?: string
}
/**
 * The Temporal connection for `remote` (and the task queue for both Temporal modes).
 */
export interface TemporalConfig {
    /**
     * The service address, by reference (it may embed an account id).
     */
    address?: (SecretRef | null)
    /**
     * The API key, by reference.
     */
    api_key?: (SecretRef | null)
    /**
     * Read `TEMPORAL_ADDRESS`, `TEMPORAL_NAMESPACE` and `TEMPORAL_API_KEY` through the
     * SDK's own environment loader instead of the references above.
     */
    from_env?: boolean
    /**
     * The namespace, by reference.
     */
    namespace?: (SecretRef | null)
    /**
     * The task queue.
     */
    task_queue?: string
}
/**
 * Hook ordering and policies.
 */
export interface HooksConfig {
    /**
     * Hook ids in the order they run when priorities tie (R4.5).
     */
    order?: string[]
    /**
     * Failure policy per point, `fail-closed` or `log-and-continue` (R4.6); the
     * security-relevant points cannot be lowered.
     */
    policies?: {
        [k: string]: string | undefined
    }
}
/**
 * The core loop's limits.
 */
export interface LoopConfig {
    /**
     * How often the heartbeat ticks (R23.1).
     */
    heartbeat_interval?: string
    /**
     * How long a long-running task waits for the next turn before the idle outcome.
     */
    idle_timeout?: (Duration | null)
    /**
     * Model calls per task before the limit outcome (R11.4).
     */
    max_iterations?: number
    /**
     * Total tokens per task, when set.
     */
    max_tokens?: (number | null)
    /**
     * `fail` or `input-required` when a limit is reached.
     */
    on_limit?: string
    /**
     * A duration: a number followed by ms, s, m or h, e.g. 30s or 15m.
     */
    signal_grace?: string
    /**
     * A duration: a number followed by ms, s, m or h, e.g. 30s or 15m.
     */
    stall_after?: string
    /**
     * Wall clock per task, when set.
     */
    wall_clock?: (Duration | null)
}
/**
 * `[models.<alias>]`.
 */
export interface ModelConfig {
    /**
     * A credential by reference: env:NAME, keyring:SERVICE/USER or file:PATH. Never a literal value.
     */
    api_key: string
    /**
     * The API base URL for `OpenAI`-compatible providers; the provider's own when absent.
     */
    base_url?: (string | null)
    /**
     * The default reasoning effort (`low`, `medium`, `high`).
     */
    effort?: (string | null)
    /**
     * The model id the provider knows.
     */
    model: string
    /**
     * The provider.
     */
    provider: ("openai" | "anthropic")
    retry?: RetryConfig1
}
/**
 * Retry policy for retryable provider errors (R9.6).
 */
export interface RetryConfig1 {
    /**
     * Multiplier per attempt.
     */
    coefficient?: number
    /**
     * The first wait.
     */
    initial?: string
    /**
     * Attempts in total; always finite.
     */
    max_attempts?: number
    /**
     * The longest wait.
     */
    max_interval?: string
}
/**
 * Observability.
 */
export interface O11YConfig {
    /**
     * Add Langfuse's generation attributes to the spans (R24.3).
     */
    langfuse?: boolean
    /**
     * The log level.
     */
    log_level?: string
    /**
     * The OTLP endpoint; no export when absent (R24.2).
     */
    otlp_endpoint?: (string | null)
}
/**
 * The persistence store.
 */
export interface PersistenceConfig {
    /**
     * The data directory for the file store.
     */
    path?: (string | null)
    /**
     * `memory` or `file`.
     */
    store?: string
}
/**
 * Plugin discovery.
 */
export interface PluginsConfig {
    /**
     * Directories scanned for Agent Plugins.
     */
    paths?: string[]
}
/**
 * The A2A server.
 */
export interface ServerConfig {
    /**
     * Browser origins allowed to call the server (CORS, R25.1): a surface served from
     * another origin, such as the web renderer's dev server. Empty allows any origin
     * while the server is loopback-only and none otherwise (fail-closed).
     */
    allow_origins?: string[]
    /**
     * The socket to bind. Without a security scheme only a loopback address is accepted.
     */
    bind?: string
    /**
     * The bindings to serve: `jsonrpc`, `rest` and, with the feature, `grpc`.
     */
    bindings?: string[]
    limits?: LimitsConfig
    /**
     * The URL clients reach the server at, for the agent card.
     */
    public_url?: (string | null)
    push?: PushConfig
    /**
     * Authentication (R19.4, Q10); absent means loopback only.
     */
    security?: (SecuritySchemeConfig | null)
}
/**
 * Per-caller limits (abuse 8).
 */
export interface LimitsConfig {
    /**
     * Requests per minute per caller.
     */
    requests_per_minute?: number
    /**
     * Open streams per caller.
     */
    streams_per_caller?: number
}
/**
 * Push notifications (abuse 9).
 */
export interface PushConfig {
    /**
     * Hosts a push-notification webhook may name; empty refuses every push config.
     */
    allow_hosts?: string[]
}
/**
 * `[system_one.<alias>]`.
 */
export interface SystemOneConfig {
    /**
     * The credential, by reference.
     */
    api_key?: (SecretRef | null)
    /**
     * The provider; `http` is the generic typed classifier endpoint (Q4).
     */
    provider: string
    /**
     * The endpoint.
     */
    url: string
}
/**
 * Tool execution.
 */
export interface ToolsConfig {
    /**
     * A tool result larger than this, in bytes, goes through the oversize policy (R8.6).
     */
    max_result_bytes?: number
    /**
     * `truncate` (keep the head and the tail) or `summarise` (one call to the `default`
     * model with `prompts/tool/summarise-result.md`).
     */
    oversize?: ("truncate" | "summarise")
}
/**
 * The terminal UI.
 */
export interface TuiConfig {
    /**
     * The push-to-talk key (R27.5).
     */
    push_to_talk_key?: string
}
/**
 * Voice.
 */
export interface VoiceConfig {
    /**
     * The credential, by reference.
     */
    api_key?: (SecretRef | null)
    /**
     * The API base (`https://api.openai.com/v1` when absent).
     */
    base_url?: (string | null)
    /**
     * The model.
     */
    model?: (string | null)
    /**
     * `openai-realtime`, or absent for no voice.
     */
    provider?: (string | null)
    /**
     * The synthesis model (`gpt-4o-mini-tts` when absent).
     */
    speech_model?: (string | null)
    /**
     * The streaming transcription model (`gpt-live-transcribe` when absent).
     */
    transcribe_model?: (string | null)
    /**
     * The voice the model speaks with (`marin` when absent).
     */
    voice?: (string | null)
}

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

// The page's side of the host (R29.2): every platform capability is a callback, each
// defaulting to the platform's own except `secret`, which has none.

/** A log line or a finished span, already redacted. */
export interface LogRecord {
    level: "trace" | "debug" | "info" | "warn" | "error";
    target: string;
    message: string;
    fields: Record<string, string>;
}

/** What the page supplies. Leave a field out to take the default. */
export interface HostOptions {
    /** Outbound HTTP. Default: `globalThis.fetch`. */
    fetch?: (url: string, init: RequestInit) => Promise<Response>;
    /** The only origins `fetch` may reach, e.g. `["https://api.openai.com"]`. Default: any. */
    allowOrigins?: string[];
    /** Opens a WebSocket. Default: `new WebSocket(url)`. Headers cannot be sent. */
    webSocket?: (url: string) => WebSocket;
    /** Milliseconds since the epoch. Default: `Date.now`. */
    now?: () => number;
    /** Fills `buf` with random bytes. Default: `crypto.getRandomValues`. */
    randomBytes?: (buf: Uint8Array) => void;
    /**
     * Resolves a credential reference from the configuration (`env:OPENAI_API_KEY`, …) to
     * its value. No default: without it, a configuration naming a credential fails to build.
     */
    secret?: (reference: string) => string | Promise<string>;
    /** Receives log lines and spans. Default: `console`. */
    log?: (record: LogRecord) => void;
}



/**
 * A harness running in this page: the loop, the registry and `none` durability, with
 * every platform capability supplied by the host (R29.1).
 */
export class Harness {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Cancels a task.
     *
     * # Errors
     *
     * An `Error` with the A2A `code` (`TaskNotFound`, not cancelable).
     */
    cancelTask(taskId: string): Promise<Task>;
    /**
     * Builds a harness from `config` (an object of the configuration's shape, or TOML
     * text) and the page's `host`; fails closed when a capability the configuration
     * needs is missing (R29.3).
     *
     * # Errors
     *
     * An invalid configuration, or a harness that cannot be built.
     */
    static create(config: Config | string, host?: HostOptions | null): Promise<Harness>;
    /**
     * The task as it stands.
     *
     * # Errors
     *
     * An `Error` with the A2A `code` (`TaskNotFound`).
     */
    getTask(taskId: string): Promise<Task>;
    /**
     * Sends a message and streams the turn back as A2A events.
     *
     * # Errors
     *
     * A request that is not a `SendMessageRequest`.
     */
    sendMessage(request: SendMessageRequest): AsyncIterable<StreamResponse>;
    /**
     * The task's log so far, then its live events until the turn ends.
     */
    subscribe(taskId: string): AsyncIterable<StreamResponse>;
}

/**
 * Return a greeting from the harness.
 */
export function hello(): Promise<string>;
