// Feature: The definition-of-done demo, the recording (R32.6)
// Requirement: docs/specs/issue-3/requirements.md#R32
//
// Scenario: Both renderers on one harness, in one recording
//     Given the demo harness (RH_DEMO_URL) and the TUI attached to it in a browser
//       terminal (RH_DEMO_TTYD_URL: `ttyd -W -t rendererType=dom rusty-harness --url …`)
//     When the web renderer and the TUI each work a task through the A2UI ask
//     Then one video shows both renderers drawing the ask and the drafted notes
//
// Skipped unless both URLs are set; run with PW_VIDEO=1.
import { expect, test } from "@playwright/test";

const harness = process.env.RH_DEMO_URL;
const tui = process.env.RH_DEMO_TTYD_URL;
const ASK =
  "Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep the task open.";

test.skip(!harness || !tui, "RH_DEMO_URL and RH_DEMO_TTYD_URL name no running demo");
test.use({ viewport: { width: 1920, height: 1000 } });

test("both renderers on one harness, recorded", async ({ page, baseURL }) => {
  test.setTimeout(900_000);
  const web = `${baseURL ?? ""}/?url=${encodeURIComponent(harness ?? "")}`;
  // The wrapper must be a secure context too, or the framed renderer is not one and has
  // no `crypto.randomUUID` to name its messages: write it over a page of the preview's.
  await page.goto(`${baseURL ?? ""}/`);
  await page.setContent(`<!doctype html>
    <style>body{margin:0;display:flex;height:100vh;font:14px sans-serif}
      figure{margin:0;flex:1;display:flex;flex-direction:column;border-right:2px solid #ccc}
      figcaption{padding:6px 10px;background:#222;color:#eee}
      iframe{flex:1;border:0;width:100%}</style>
    <figure><figcaption>web renderer</figcaption><iframe id="web" src="${web}"></iframe></figure>
    <figure><figcaption>TUI (rusty-harness --url)</figcaption><iframe id="tui" src="${tui ?? ""}"></iframe></figure>`);

  // The web renderer: ask, answer the form, see the draft.
  const w = page.frameLocator("#web");
  const box = w.getByPlaceholder(/Message demo-agent/);
  await box.waitFor({ timeout: 60_000 });
  await box.fill(ASK);
  await box.press("Enter");
  const form = w.getByTestId("a2ui-block");
  await expect(form).toContainText("Which audience are these notes for?", { timeout: 240_000 });
  await page.waitForTimeout(1500);
  await form.getByText("Send answer").click();
  await expect(w.getByText(/Waiting for your next message/)).toBeVisible({ timeout: 240_000 });

  // The TUI on the same harness: the same journey, drawn in the terminal.
  const t = page.frameLocator("#tui");
  const rows = t.locator(".xterm-rows");
  await expect(rows).toContainText("demo-agent", { timeout: 60_000 });
  const keys = t.locator("textarea.xterm-helper-textarea");
  await keys.pressSequentially(ASK, { delay: 5 });
  await keys.press("Enter");
  await expect(rows).toContainText("Which audience are these notes for?", { timeout: 240_000 });
  await page.waitForTimeout(1500);
  await keys.press("Tab");
  await keys.press("Tab");
  await keys.press("Enter");
  await expect(rows).toContainText("awaiting next turn", { timeout: 240_000 });
  await page.waitForTimeout(3000);
});
