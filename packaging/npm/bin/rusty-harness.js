#!/usr/bin/env node
// The `rusty-harness` command: runs the platform's prebuilt binary with this process's
// arguments, standard streams and exit status.
import { spawnSync } from "node:child_process";
import { resolveBinary } from "../lib/resolve.js";

const found = resolveBinary();
if (!found.ok) {
  console.error(found.message);
  process.exit(1);
}
const run = spawnSync(found.path, process.argv.slice(2), { stdio: "inherit" });
if (run.error) {
  console.error(`rusty-harness: ${run.error.message}`);
  process.exit(1);
}
if (run.signal) {
  process.kill(process.pid, run.signal);
}
process.exit(run.status ?? 1);
