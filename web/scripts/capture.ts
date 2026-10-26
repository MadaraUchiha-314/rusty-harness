// Captures the multi-turn flow as a video and turns it into the evidence GIF
// (testing-plan T5: `ui/multi-turn.gif`), plus a still per state, from the same mock
// harness the browser tests use. Run `bun run capture [out-dir]` with the preview server
// up (`bun run build && bun run preview --port 4173`).
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, renameSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "@playwright/test";
import { scenarios, startMock } from "../tests/support/mock-harness";

const out = process.argv[2] ?? join(import.meta.dirname, "..", "capture");
mkdirSync(out, { recursive: true });
const base = process.env.BASE_URL ?? "http://127.0.0.1:4173";

const browser = await chromium.launch();
const context = await browser.newContext({
  viewport: { width: 1280, height: 800 },
  recordVideo: { dir: out, size: { width: 1280, height: 800 } },
});
const page = await context.newPage();
const mock = await startMock((request, index) =>
  index === 0 ? scenarios.a2ui(request, index) : scenarios.done(request, index + 1),
);
await page.goto(`${base}/?url=${encodeURIComponent(mock.url)}`);
const box = page.getByPlaceholder(/Message demo-agent/);
await box.waitFor();
await page.waitForTimeout(600);
await box.pressSequentially(
  "Draft release notes for Q4. Keep the task open, I will iterate with you.",
  { delay: 20 },
);
await box.press("Enter");
await page.getByTestId("a2ui-block").waitFor();
await page.waitForTimeout(1200);
await page.screenshot({ path: join(out, "web-a2ui.png") });
await page.getByTestId("a2ui-block").getByText("Send answer").click();
await page.waitForTimeout(1200);
await page.getByTestId("complete").click();
await page.waitForTimeout(800);
await page.getByTestId("confirm-complete").getByRole("button", { name: "Complete" }).click();
await page.waitForTimeout(1200);
await page.screenshot({ path: join(out, "web-done.png") });
await context.close();
await browser.close();
await mock.close();

const webm = readdirSync(out).find((f) => f.endsWith(".webm"));
if (webm) {
  const ffmpeg = process.env.FFMPEG ?? "ffmpeg";
  const gif = join(out, "multi-turn.gif");
  execFileSync(ffmpeg, [
    "-y",
    "-i",
    join(out, webm),
    "-vf",
    "fps=8,scale=960:-1:flags=lanczos",
    gif,
  ]);
  renameSync(join(out, webm), join(out, "multi-turn.webm"));
  console.log(`wrote ${gif}`);
}
