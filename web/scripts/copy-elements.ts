// Copies the shadcn/ui primitives and the AI Elements the renderer composes from their
// registries into src/components (design.md § Web renderer, R26.1): the same files the
// shadcn CLI would write, fetched without its prompts so the copy is reproducible.
//
//   bun run copy:elements
//
// Re-running overwrites the copies; local edits belong in src/components/app, not here.

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const ROOT = join(import.meta.dirname, "..");
const SHADCN = "https://ui.shadcn.com/r/styles/new-york-v4";
const ELEMENTS = "https://registry.ai-sdk.dev";

const primitives = [
  "button",
  "button-group",
  "tooltip",
  "command",
  "dropdown-menu",
  "hover-card",
  "input-group",
  "select",
  "badge",
  "collapsible",
  "card",
  "scroll-area",
  "alert",
  "dialog",
  "separator",
  "textarea",
  "input",
];
const elements = [
  "conversation",
  "message",
  "prompt-input",
  "tool",
  "reasoning",
  "plan",
  "task",
  "suggestion",
  "confirmation",
  "model-selector",
  "loader",
  "shimmer",
  "code-block",
];

type RegistryFile = { path: string; content: string; target?: string };
type RegistryItem = { name: string; files?: RegistryFile[] };

function isItem(v: unknown): v is RegistryItem {
  return typeof v === "object" && v !== null && "name" in v;
}

async function item(url: string): Promise<RegistryItem> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url}: ${res.status}`);
  const data: unknown = await res.json();
  if (!isItem(data)) throw new Error(`${url}: not a registry item`);
  return data;
}

// What the shadcn CLI rewrites on install (import aliases), plus two compatibility
// touches for the versions pinned here: the AI Elements target the AI SDK's v6 types
// behind `@ts-expect-error` lines that v7 makes unused, and `Reasoning` spreads
// collapsible props onto `Streamdown` 2, whose `dir` type is narrower.
function adapt(content: string): string {
  return content
    .replaceAll('from "cn"', 'from "@/lib/utils"')
    .replaceAll("@/registry/new-york-v4/ui/", "@/components/ui/")
    .replaceAll("@/registry/new-york-v4/lib/utils", "@/lib/utils")
    .replace(/^\s*\/\/ @ts-expect-error state only available in AI SDK v6\n/gm, "")
    .replace(
      "<Streamdown {...props}>{children}</Streamdown>",
      "<Streamdown>{children}</Streamdown>",
    );
}

function write(target: string, content: string): void {
  const path = join(ROOT, "src", target);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, adapt(content));
  console.log(`wrote ${target}`);
}

await Promise.all([
  ...primitives.map(async (name) => {
    const it = await item(`${SHADCN}/${name}.json`);
    for (const f of it.files ?? []) write(`components/ui/${f.path.split("/").pop()}`, f.content);
  }),
  ...elements.map(async (name) => {
    const it = await item(`${ELEMENTS}/${name}.json`);
    for (const f of it.files ?? []) {
      write(`components/ai-elements/${f.path.split("/").pop()}`, f.content);
    }
  }),
]);
