// Feature: Web renderer — the phone width folds the sidebar and the panel (T5)
// Requirement: docs/specs/issue-3/requirements.md#R26 (R26.7, NFR accessibility)
import { expect, test } from "@playwright/test";
import { openWith, send, settle } from "../support/page";

test("streaming at phone width scrolls vertically only", async ({ page }) => {
  const mock = await openWith(page, "streaming");
  await send(page, "Draft release notes for Q4.");
  await expect(page.getByText("per-tenant rate limits")).toBeVisible();
  await settle(page);
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
  );
  expect(overflow).toBe(false);
  await expect(page).toHaveScreenshot("web-streaming-phone.png");
  await mock.close();
});
