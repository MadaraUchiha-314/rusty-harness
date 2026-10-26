// Feature: Strings as assets — the web reads the crate's catalog (T14, `test:strings`)
// Requirement: docs/specs/issue-3/requirements.md#R33
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { IntlMessageFormat } from "intl-messageformat";
import { describe, expect, it } from "vitest";
import { catalog, t } from "./i18n";

function sources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      if (name !== "ui" && name !== "ai-elements") sources(path, out);
    } else if (
      /\.(ts|tsx)$/.test(name) &&
      !name.endsWith(".test.ts") &&
      !name.endsWith(".test.tsx")
    ) {
      out.push(path);
    }
  }
  return out;
}

describe("strings", () => {
  it("every key the renderer references exists in en.json and every pattern compiles", () => {
    const referenced = new Set<string>();
    for (const file of sources(join(import.meta.dirname))) {
      const text = readFileSync(file, "utf8");
      for (const m of text.matchAll(/\bt\(\s*"([a-z0-9_.]+)"/g)) referenced.add(m[1]);
      for (const m of text.matchAll(/"((?:tui|web|command)\.[a-z0-9_.]+)"/g)) referenced.add(m[1]);
    }
    expect(referenced.size).toBeGreaterThan(20);
    const missing = [...referenced].filter((k) => !(k in catalog));
    expect(missing).toEqual([]);
    const broken: string[] = [];
    for (const [key, pattern] of Object.entries(catalog)) {
      try {
        const compiled = new IntlMessageFormat(pattern, "en", undefined, { ignoreTag: true });
        expect(compiled.getAst().length).toBeGreaterThan(0);
      } catch (e) {
        broken.push(`${key}: ${String(e)}`);
      }
    }
    expect(broken).toEqual([]);
  });

  it("formats plurals and arguments like the crate's parser", () => {
    expect(t("web.turns", { n: 1 })).toBe("1 turn");
    expect(t("web.turns", { n: 3 })).toBe("3 turns");
    expect(t("tui.confirm.complete", { n: 0 })).toContain("no running sub-task is affected");
    expect(t("web.composer.placeholder", { agent: "demo-agent" })).toBe(
      "Message demo-agent… ( / for commands )",
    );
  });
});
