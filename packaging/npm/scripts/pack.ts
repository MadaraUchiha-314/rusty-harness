// Release packaging for `@rusty-harness/cli` (Q8), run by the release workflow:
//
//   bun packaging/npm/scripts/pack.ts platform <platform> <binary> <out-dir>
//     writes the platform package `@rusty-harness/cli-<platform>` around <binary>;
//   bun packaging/npm/scripts/pack.ts main <out-dir>
//     writes the main package with one optional dependency per platform.
//
// Every version comes from the workspace Cargo.toml, the single source of truth.
import { chmodSync, copyFileSync, cpSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { PLATFORMS, executable, packageName } from "../lib/resolve.js";

const npmDir = join(import.meta.dirname, "..");
const root = join(npmDir, "..", "..");

export function workspaceVersion(): string {
  const cargoToml = readFileSync(join(root, "Cargo.toml"), "utf8");
  const version = /^\[workspace\.package\][^[]*?^version\s*=\s*"([^"]+)"/m.exec(cargoToml)?.[1];
  if (version === undefined) throw new Error("workspace.package.version not found in Cargo.toml");
  return version;
}

const common = {
  homepage: "https://madarauchiha-314.github.io/rusty-harness/",
  license: "Apache-2.0",
  repository: {
    type: "git",
    url: "git+https://github.com/MadaraUchiha-314/rusty-harness.git",
    directory: "packaging/npm",
  },
};

export function platformPackage(platform: string, binary: string, out: string): string {
  if (!PLATFORMS.includes(platform)) throw new Error(`unknown platform ${platform}`);
  const [os = "", cpu = ""] = platform.split("-");
  const name = packageName(os, cpu);
  const target = join(out, executable(os));
  mkdirSync(join(out, "bin"), { recursive: true });
  copyFileSync(binary, target);
  chmodSync(target, 0o755);
  const manifest = {
    name,
    version: workspaceVersion(),
    description: `The rusty-harness binary for ${platform}.`,
    ...common,
    os: [os],
    cpu: [cpu],
    ...(os === "linux" ? { libc: ["glibc"] } : {}),
    files: ["bin"],
  };
  writeFileSync(join(out, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  writeFileSync(join(out, "README.md"), `# ${name}\n\nInstalled by \`@rusty-harness/cli\`.\n`);
  return name;
}

export function mainPackage(out: string): void {
  const version = workspaceVersion();
  const manifest: Record<string, unknown> = JSON.parse(
    readFileSync(join(npmDir, "package.json"), "utf8"),
  );
  delete manifest["scripts"];
  manifest["version"] = version;
  manifest["optionalDependencies"] = Object.fromEntries(
    PLATFORMS.map((p) => {
      const [os = "", cpu = ""] = p.split("-");
      return [packageName(os, cpu), version];
    }),
  );
  mkdirSync(out, { recursive: true });
  for (const dir of ["bin", "lib"]) cpSync(join(npmDir, dir), join(out, dir), { recursive: true });
  copyFileSync(join(npmDir, "README.md"), join(out, "README.md"));
  writeFileSync(join(out, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);
}

if (import.meta.main) {
  const [command, ...args] = process.argv.slice(2);
  if (command === "platform" && args.length === 3) {
    const [platform = "", binary = "", out = ""] = args;
    console.log(platformPackage(platform, binary, out));
  } else if (command === "main" && args.length === 1) {
    mainPackage(args[0] ?? "");
  } else {
    console.error("usage: pack.ts platform <platform> <binary> <out-dir> | main <out-dir>");
    process.exit(2);
  }
}
