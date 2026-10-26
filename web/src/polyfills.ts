// The A2A TypeScript SDK decodes and encodes raw (bytes) parts with Node's `Buffer`,
// which browsers do not have (design.md D1: spike first, polyfill second; upstream issue
// 4 in docs/specs/issue-3/evidence/upstream-issues.md). Imported before the SDK.
import { Buffer } from "buffer";

const scope = globalThis as { Buffer?: typeof Buffer };
scope.Buffer ??= Buffer;
