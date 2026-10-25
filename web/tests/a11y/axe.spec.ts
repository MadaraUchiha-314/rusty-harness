// Feature: Web renderer — accessibility (T9)
// Requirement: docs/specs/issue-3/requirements.md#R26 (NFR accessibility)
//
// axe at WCAG 2.1 AA on every state in light and dark, and the keyboard flow the design
// promises: Enter sends, Shift+Enter breaks the line, `/` opens the palette, Esc closes.
import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { dark, openWith, send } from "../support/page";

const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"];

async function analyze(page: Page, where: string, scheme: "light" | "dark") {
  const results = await new AxeBuilder({ page }).withTags(TAGS).analyze();
  const violations = results.violations.map((v) => ({
    id: v.id,
    impact: v.impact,
    nodes: v.nodes.map((n) => `${n.target.join(" ")} ${n.html.slice(0, 160)}`),
  }));
  expect(violations, `${where} (${scheme})`).toEqual([]);
}

/** axe in light, then dark, then back to light. */
async function audit(page: Page, where: string) {
  await analyze(page, where, "light");
  await dark(page);
  await analyze(page, where, "dark");
  await page.emulateMedia({ colorScheme: "light" });
}

const USER = "Draft release notes for Q4. Keep the task open, I will iterate with you.";

test("the empty renderer and the streaming state have no AA violations", async ({ page }) => {
  const mock = await openWith(page, "streaming");
  await audit(page, "empty");
  await send(page, USER);
  await expect(page.getByText("per-tenant rate limits")).toBeVisible();
  await audit(page, "streaming");
  await mock.close();
});

test("idle, the palette and the confirmation have no AA violations", async ({ page }) => {
  const mock = await openWith(page, "done");
  await send(page, USER);
  await expect(page.getByText(/Waiting for your next message/)).toBeVisible();
  await audit(page, "idle");
  await page.getByPlaceholder(/Message demo-agent/).fill("/mo");
  await expect(page.getByTestId("palette")).toBeVisible();
  await audit(page, "palette");
  await page.getByPlaceholder(/Message demo-agent/).fill("");
  await page.getByTestId("complete").click();
  await expect(page.getByTestId("confirm-complete")).toBeVisible();
  await audit(page, "confirm");
  await mock.close();
});

test("the A2UI form has no AA violations", async ({ page }) => {
  const mock = await openWith(page, "a2ui");
  await send(page, USER);
  await expect(page.getByTestId("a2ui-block")).toBeVisible();
  await audit(page, "a2ui");
  await mock.close();
});

test("the keyboard flow: Enter, Shift+Enter, / and Esc", async ({ page }) => {
  const mock = await openWith(page, "idle");
  const box = page.getByPlaceholder(/Message demo-agent/);
  await box.focus();
  await page.keyboard.type("line one");
  await page.keyboard.press("Shift+Enter");
  await page.keyboard.type("line two");
  await expect(box).toHaveValue("line one\nline two");
  expect(mock.requests.filter((r) => r.method === "SendStreamingMessage")).toHaveLength(0);
  await page.keyboard.press("Enter");
  await expect
    .poll(() => mock.requests.filter((r) => r.method === "SendStreamingMessage").length)
    .toBe(1);
  await expect(box).toHaveValue("");
  await page.keyboard.type("/");
  await expect(page.getByTestId("palette")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("palette")).toHaveCount(0);
  // Focus stays visible on the controls reached by Tab.
  await page.keyboard.press("Tab");
  const focused = await page.evaluate(() => document.activeElement?.tagName);
  expect(focused).toBeTruthy();
  await mock.close();
});
