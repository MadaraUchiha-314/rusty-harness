// Feature: Web renderer — the bundle carries no credential (T8, abuse 11, boundary 7)
// Requirement: docs/specs/issue-3/requirements.md#R26
//
// Scenario: The npm bundle contains no provider key pattern and makes model calls only
// through the host HttpClient
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const WEB_DIST = join(import.meta.dirname, "..", "dist");
const WASM_DIST = join(import.meta.dirname, "..", "..", "crates", "rusty-harness-wasm", "dist");

const KEY_PATTERNS: [string, RegExp][] = [
  // A key is a long run of alphanumerics after the prefix; `sk-image-conic-…` style
  // identifiers in a graphics library are not.
  ["OpenAI key", /sk-(?:proj-)?[A-Za-z0-9]{20,}/],
  ["Anthropic key", /sk-ant-[A-Za-z0-9-]{10,}[A-Za-z0-9]{10,}/],
  ["AWS access key", /AKIA[0-9A-Z]{16}/],
  ["GitHub token", /gh[pousr]_[A-Za-z0-9]{30,}/],
  ["Slack token", /xox[baprs]-[A-Za-z0-9-]{10,}/],
  ["bearer literal", /Bearer\s+[A-Za-z0-9._-]{24,}/],
];
// The web renderer never talks to a model provider: the harness does (abuse 11). The
// WASM SDK *is* a harness, so it names the providers it defaults to, but reaches them only
// through the page's `fetch` (its scenario tests prove it) and holds no key; it is scanned
// for key patterns only.
const PROVIDER_HOSTS = ["api.openai.com", "api.anthropic.com", "openrouter.ai/api"];

function files(dir: string, out: string[] = []): string[] {
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) files(path, out);
    else if (/\.(js|mjs|cjs|css|html|wasm)$/.test(name)) out.push(path);
  }
  return out;
}

describe("the bundles", () => {
  it("abuse_npm_bundle_contains_no_provider_key_pattern", () => {
    const web = files(WEB_DIST);
    const scanned = [...web, ...files(WASM_DIST)];
    expect(scanned.length, "build the web renderer first (bun run build)").toBeGreaterThan(0);
    const findings: string[] = [];
    for (const file of scanned) {
      const text = readFileSync(file, "latin1");
      for (const [name, pattern] of KEY_PATTERNS) {
        const m = pattern.exec(text);
        if (m) findings.push(`${file}: ${name} (${m[0].slice(0, 12)}…)`);
      }
      if (!web.includes(file)) continue;
      for (const host of PROVIDER_HOSTS) {
        if (text.includes(host)) findings.push(`${file}: provider host ${host}`);
      }
    }
    expect(findings).toEqual([]);
  });
});
