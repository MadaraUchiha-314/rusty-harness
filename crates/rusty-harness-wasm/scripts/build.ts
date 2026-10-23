// Builds the npm package: one wasm-pack build per target under dist/, with the
// package version synced from the workspace Cargo.toml (the single source of truth).
import { readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { $ } from "bun";

const crateDir = join(import.meta.dir, "..");
const targets = ["bundler", "web", "nodejs"] as const;

rmSync(join(crateDir, "dist"), { recursive: true, force: true });
for (const target of targets) {
  const outDir = join(crateDir, "dist", target);
  await $`wasm-pack build ${crateDir} --release --target ${target} --out-dir ${outDir} --out-name rusty_harness`;
  // wasm-pack writes a catch-all .gitignore, which would also hide the files from npm.
  rmSync(join(outDir, ".gitignore"), { force: true });
}

const cargoToml = readFileSync(join(crateDir, "..", "..", "Cargo.toml"), "utf8");
const version = /^\[workspace\.package\][^[]*?^version\s*=\s*"([^"]+)"/m.exec(cargoToml)?.[1];
if (version === undefined) {
  throw new Error("workspace.package.version not found in Cargo.toml");
}
const manifestPath = join(crateDir, "package.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as { version: string };
manifest.version = version;
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`rusty-harness@${version} built for ${targets.join(", ")}`);
