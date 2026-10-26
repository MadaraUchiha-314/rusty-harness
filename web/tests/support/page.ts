// Shared steps for the browser specs: open the renderer against a mock, send a message,
// wait for the state the scenario ends in.
import type { Page } from "@playwright/test";
import { startMock, scenarios, type MockHarness, type Scenario } from "./mock-harness";

export async function openWith(page: Page, scenario: Scenario | string): Promise<MockHarness> {
  const fn = typeof scenario === "string" ? scenarios[scenario] : scenario;
  if (!fn) throw new Error(`no scenario ${String(scenario)}`);
  const mock = await startMock(fn);
  await page.goto(`/?url=${encodeURIComponent(mock.url)}`);
  await page.getByPlaceholder(/Message demo-agent/).waitFor();
  return mock;
}

export async function send(page: Page, text: string): Promise<void> {
  const box = page.getByPlaceholder(/Message demo-agent/);
  await box.fill(text);
  await box.press("Enter");
}

export async function settle(page: Page): Promise<void> {
  // Fonts, finite transitions (a theme switch fades colours) and the stick-to-bottom scroll.
  await page.evaluate(async () => {
    await document.fonts.ready;
    const finite = document
      .getAnimations()
      .filter((a) => a.effect?.getTiming().iterations !== Infinity);
    await Promise.all(finite.map((a) => a.finished.catch(() => undefined)));
  });
  await page.waitForTimeout(250);
}

export async function dark(page: Page): Promise<void> {
  await page.emulateMedia({ colorScheme: "dark" });
  await settle(page);
}

export async function light(page: Page): Promise<void> {
  await page.emulateMedia({ colorScheme: "light" });
  await settle(page);
}
