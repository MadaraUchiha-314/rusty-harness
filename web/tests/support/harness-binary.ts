// Boots the `rusty-harness` binary against a mock model, as the conformance suite does.
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { portOf } from "./json";

const REPO = join(import.meta.dirname, "..", "..", "..");

async function freePort(): Promise<number> {
  const s = createServer();
  await new Promise<void>((r) => s.listen(0, "127.0.0.1", () => r()));
  const port = portOf(s.address());
  await new Promise<void>((r) => s.close(() => r()));
  return port;
}

async function waitFor(url: string, ms: number): Promise<void> {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    try {
      // oxlint-disable-next-line no-await-in-loop -- polling until the server answers
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      /* not up yet */
    }
    // oxlint-disable-next-line no-await-in-loop -- the back-off between polls
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`${url} never answered`);
}

export interface RunningHarness {
  url: string;
  child: ChildProcess;
  stop(): void;
}

/** Starts the binary (the debug build when present, `cargo run` otherwise) with a model
 * at `mockOpenAiPort`; the model key is a canary that must never surface. */
export async function startHarness(mockOpenAiPort: number): Promise<RunningHarness> {
  const port = await freePort();
  const dir = mkdtempSync(join(tmpdir(), "rh-e2e-"));
  writeFileSync(
    join(dir, "config.toml"),
    [
      `[agent]`,
      `name = "demo-agent"`,
      `description = "The web renderer's end-to-end harness."`,
      `[models.default]`,
      `provider = "openai"`,
      `model = "mock"`,
      `api_key = "env:RH_E2E_KEY"`,
      `base_url = "http://127.0.0.1:${mockOpenAiPort}/v1"`,
      `[server]`,
      `bind = "127.0.0.1:${port}"`,
      ``,
    ].join("\n"),
  );
  const built = join(REPO, "target", "debug", "rusty-harness");
  const [command, args] = existsSync(built)
    ? [built, ["--no-tui", "--config", join(dir, "config.toml")]]
    : [
        "cargo",
        [
          "run",
          "-q",
          "-p",
          "rusty_harness",
          "--features",
          "cli",
          "--bin",
          "rusty-harness",
          "--",
          "--no-tui",
          "--config",
          join(dir, "config.toml"),
        ],
      ];
  const child = spawn(command, args, {
    cwd: REPO,
    env: { ...process.env, RH_E2E_KEY: "sk-e2e-canary", RUST_LOG: "warn" },
    stdio: ["ignore", "inherit", "inherit"],
  });
  const url = `http://127.0.0.1:${port}/`;
  await waitFor(`${url}.well-known/agent-card.json`, 600_000);
  return {
    url,
    child,
    stop: () => {
      child.kill("SIGTERM");
    },
  };
}
