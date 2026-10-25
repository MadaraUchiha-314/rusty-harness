// Narrowing helpers for the untyped JSON the mock and the page exchange.
export type Json = Record<string, unknown>;

export function isJson(v: unknown): v is Json {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** `v` as an object, or an error naming what it was. */
export function rec(v: unknown): Json {
  if (!isJson(v)) throw new Error(`not an object: ${JSON.stringify(v)}`);
  return v;
}

/** `v` as a list of objects. */
export function list(v: unknown): Json[] {
  if (!Array.isArray(v)) throw new Error(`not a list: ${JSON.stringify(v)}`);
  return v.map(rec);
}

/** The port a Node server listens on. */
export function portOf(address: string | { port: number } | null): number {
  if (!address || typeof address === "string") throw new Error("no port");
  return address.port;
}
