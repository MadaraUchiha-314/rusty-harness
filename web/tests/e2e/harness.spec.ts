// Feature: Web renderer — one harness, the A2UI ask round trip (T2, T4 rehearsal)
// Requirement: docs/specs/issue-3/requirements.md#R32 (R32.1), R25.4, R26
//
// Scenario: An A2UI ask renders in the web renderer and the TUI (the web leg; the TUI
// leg is `tests/tui.rs::on_one_harness` in the core crate)
//     Given the rusty-harness binary with a model that asks an A2UI question, then answers
//     When the web renderer sends a message
//     Then the ask renders through the official React renderer and the form's answer
//     resumes the task, which waits for the next turn
import { expect, test } from "@playwright/test";
import { startHarness, type RunningHarness } from "../support/harness-binary";
import { a2uiMessages } from "../support/mock-harness";
import { startMockOpenAi, type MockOpenAi } from "../support/mock-openai";

let model: MockOpenAi;
let harness: RunningHarness;

test.beforeAll(async () => {
  model = await startMockOpenAi([
    {
      tool: {
        id: "call_ask",
        name: "participant.ask",
        arguments: { to: ["operator"], kind: "a2ui", question: a2uiMessages() },
      },
    },
    { text: "Drafting the notes for customers." },
  ]);
  harness = await startHarness(model.port);
});

test.afterAll(async () => {
  harness?.stop();
  await model?.close();
});

test("the A2UI ask round-trips through the real harness", async ({ page }) => {
  await page.goto(`/?url=${encodeURIComponent(harness.url)}`);
  const box = page.getByPlaceholder(/Message demo-agent/);
  await box.waitFor();
  await box.fill("Draft release notes for Q4. Keep the task open, I will iterate with you.");
  await box.press("Enter");

  const form = page.getByTestId("a2ui-block");
  await expect(form).toBeVisible({ timeout: 30_000 });
  await expect(form).toContainText("Which audience are these notes for?");
  await expect(page.getByText(/asked @you on channel/)).toBeVisible();
  await expect(page.getByTestId("state-badge")).toHaveText(/input-required/);

  await form.getByText("Send answer").click();
  await expect(page.getByText("Drafting the notes for customers.")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.getByText(/Waiting for your next message/)).toBeVisible();
  expect(model.calls).toBe(2);
  // The canary model key never reaches the page.
  const html = await page.content();
  expect(html).not.toContain("sk-e2e-canary");
});
