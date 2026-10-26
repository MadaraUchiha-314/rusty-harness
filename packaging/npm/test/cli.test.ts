// Feature: CLI distribution on npm (Q8, R27.1)
//
// Scenario: The npm shim runs the platform's binary
//     Given the main package and this platform's package laid out as npm installs them
//     When `rusty-harness` runs with arguments
//     Then the binary gets them and its exit status is the command's
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, test } from "bun:test";
import { PLATFORMS, packageName, resolveBinary } from "../lib/resolve.js";
import { mainPackage, platformPackage, workspaceVersion } from "../scripts/pack";

const here = `${process.platform}-${process.arch}`;

describe("@rusty-harness/cli", () => {
  test("names one package per platform", () => {
    expect(packageName("linux", "x64")).toBe("@rusty-harness/cli-linux-x64");
    expect(PLATFORMS).toHaveLength(5);
  });

  test("explains an unsupported platform instead of failing obscurely", () => {
    const found = resolveBinary({ platform: "aix", arch: "ppc64" });
    expect(found.ok).toBe(false);
    if (!found.ok) expect(found.message).toContain("cargo install rusty_harness");
  });

  test("says musl has no prebuilt binary instead of advising a reinstall", () => {
    const found = resolveBinary({ platform: "linux", arch: "x64", libc: "musl" });
    expect(found.ok).toBe(false);
    if (!found.ok) {
      expect(found.message).toContain("musl");
      expect(found.message).toContain("cargo install rusty_harness");
      expect(found.message).not.toContain("reinstall");
    }
  });

  test("the main package depends on every platform at the workspace version", () => {
    const out = mkdtempSync(join(tmpdir(), "rh-npm-"));
    mainPackage(out);
    const manifest = JSON.parse(readFileSync(join(out, "package.json"), "utf8"));
    expect(manifest.version).toBe(workspaceVersion());
    expect(Object.keys(manifest.optionalDependencies)).toHaveLength(PLATFORMS.length);
    expect(manifest.scripts).toBeUndefined();
  });

  test.skipIf(process.platform === "win32" || !PLATFORMS.includes(here))(
    "the shim runs the platform binary with its arguments and exit status",
    () => {
      const root = mkdtempSync(join(tmpdir(), "rh-npm-"));
      const fake = join(root, "fake");
      writeFileSync(fake, '#!/bin/sh\necho "binary got: $*"\nexit 3\n');
      const modules = join(root, "node_modules", "@rusty-harness");
      platformPackage(here, fake, join(modules, `cli-${here}`));
      mainPackage(join(modules, "cli"));
      const run = spawnSync(
        "node",
        [join(modules, "cli", "bin", "rusty-harness.js"), "serve", "--x"],
        {
          encoding: "utf8",
        },
      );
      expect(run.stdout.trim()).toBe("binary got: serve --x");
      expect(run.status).toBe(3);
    },
  );
});
