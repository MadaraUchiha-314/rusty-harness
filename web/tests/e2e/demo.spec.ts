// Feature: The definition-of-done demo, the browser leg (T4, R32.1–2)
// Requirement: docs/specs/issue-3/requirements.md#R32
//
// Scenario: The demo runs end to end in the web renderer
//     Given the demo harness running (`cargo run -p rusty_harness --features cli --
//       --config tests/fixtures/demo/config.toml --no-tui`) and RH_DEMO_URL naming it
//     When the web renderer drafts release notes over three turns
//     Then the A2UI ask renders through the official React renderer, its answer resumes
//     the task, and the task stays open
//
// Skipped without RH_DEMO_URL. `PW_VIDEO=1` records it; RH_E2E_EVIDENCE=<dir> keeps the
// stills of each state.
import { expect, test, type Page } from "@playwright/test";

const url = process.env.RH_DEMO_URL;
const evidence = process.env.RH_E2E_EVIDENCE;

async function still(page: Page, name: string) {
  if (evidence) await page.screenshot({ path: `${evidence}/web-${name}.png` });
}

test.skip(!url, "RH_DEMO_URL names no running demo harness");

test("the demo runs end to end in the web renderer", async ({ page }) => {
  test.setTimeout(600_000);
  await page.goto(`/?url=${encodeURIComponent(url ?? "")}`);
  const box = page.getByPlaceholder(/Message demo-agent/);
  await box.waitFor();
  await box.fill(
    "Draft the release notes for rusty-harness 0.2: the npm and PyPI packages, the Responses API, voice. Keep the task open.",
  );
  await box.press("Enter");

  const form = page.getByTestId("a2ui-block");
  await expect(form).toBeVisible({ timeout: 240_000 });
  await expect(form).toContainText("Which audience are these notes for?");
  await expect(page.getByTestId("state-badge")).toHaveText(/input-required/);
  await still(page, "1-ask");

  await form.getByText("Send answer").click();
  await expect(page.getByText("→ answer")).toBeVisible();
  await expect(page.getByText(/Waiting for your next message/)).toBeVisible({ timeout: 240_000 });
  await still(page, "2-draft");

  // Turn 3 ends with one more agent turn, nothing streaming and the task waiting again.
  const agentTurns = page.getByTestId("turn-agent");
  const before = await agentTurns.count();
  await box.fill("Shorter, please: three bullets at most.");
  await box.press("Enter");
  await expect(page.getByText("Shorter, please: three bullets at most.")).toBeVisible();
  await expect(agentTurns).toHaveCount(before + 1, { timeout: 240_000 });
  await expect(page.getByText(/· streaming/)).toHaveCount(0, { timeout: 240_000 });
  await expect(page.getByTestId("state-badge")).toHaveText(/input-required/, { timeout: 240_000 });
  await still(page, "3-revised");

  const html = await page.content();
  expect(html).not.toMatch(/sk-[A-Za-z0-9]{8,}/);
});
