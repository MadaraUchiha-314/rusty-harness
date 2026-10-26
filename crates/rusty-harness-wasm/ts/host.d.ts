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
