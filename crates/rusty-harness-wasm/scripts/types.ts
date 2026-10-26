// Generates ts/config.d.ts, the `Config` interface, from the configuration's JSON schema
// (Q9), the same schema the Python `TypedDict`s come from. `--check` fails when stale.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { compile } from "json-schema-to-typescript";

const crateDir = join(import.meta.dirname, "..");
const schemaPath = join(crateDir, "..", "rusty-harness", "rusty-harness.schema.json");
const outPath = join(crateDir, "ts", "config.d.ts");

export async function configDeclarations(): Promise<string> {
  const schema: Parameters<typeof compile>[0] = JSON.parse(readFileSync(schemaPath, "utf8"));
  const ts = await compile(schema, "Config", {
    bannerComment:
      "\n// Generated from crates/rusty-harness/rusty-harness.schema.json by scripts/types.ts.\n// Do not edit: run `bun run types`.\n",
    additionalProperties: false,
    format: false,
    strictIndexSignatures: true,
  });
  return ts;
}

if (import.meta.main) {
  const ts = await configDeclarations();
  if (process.argv.includes("--check")) {
    if (readFileSync(outPath, "utf8") !== ts) {
      console.error("ts/config.d.ts is stale: run `bun run types`");
      process.exit(1);
    }
  } else {
    writeFileSync(outPath, ts);
  }
}
