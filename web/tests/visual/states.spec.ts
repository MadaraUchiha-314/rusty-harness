// Feature: Web renderer — the visual contract (T5)
// Requirement: docs/specs/issue-3/requirements.md#R26 (R26.7)
//
// One still per prototype state of design/web-renderer.html, light and dark; the
// baselines under __screenshots__ are the locked rendering, re-captured on purpose with
// `bun run test:visual:update`.
import { expect, test } from "@playwright/test";
import { list, rec } from "../support/json";
import { dark, openWith, send, settle } from "../support/page";

const USER = "Draft release notes for Q4. Keep the task open, I will iterate with you.";

async function shoot(page: import("@playwright/test").Page, name: string) {
  await settle(page);
  await expect(page).toHaveScreenshot(`web-${name}.png`, { fullPage: false });
  await dark(page);
  await expect(page).toHaveScreenshot(`web-${name}-dark.png`, { fullPage: false });
  await page.emulateMedia({ colorScheme: "light" });
}

test("streaming: the reply is still arriving", async ({ page }) => {
  const mock = await openWith(page, "streaming");
  await send(page, USER);
  await expect(page.getByText(/streaming/)).toBeVisible();
  await expect(page.getByText("per-tenant rate limits")).toBeVisible();
  await shoot(page, "streaming");
  await mock.close();
});

test("idle: input-required, the artifact attached, your turn", async ({ page }) => {
  const mock = await openWith(page, "idle");
  await send(page, USER);
  await expect(page.getByText(/Waiting for your next message/)).toBeVisible();
  await expect(page.getByTestId("artifact")).toContainText("release-notes-q4.md");
  await expect(page.getByTestId("state-badge")).toHaveText(/input-required/);
  await shoot(page, "idle");
  await mock.close();
});

test("a2ui: the ask renders as a form on the channel", async ({ page }) => {
  const mock = await openWith(page, "a2ui");
  await send(page, USER);
  const form = page.getByTestId("a2ui-block");
  await expect(form).toBeVisible();
  await expect(form).toContainText("Which audience are these notes for?");
  await expect(page.getByText(/asked @you on channel ch-7f3a/)).toBeVisible();
  await shoot(page, "a2ui");
  // The form's button answers on the channel as an A2UI client action.
  await form.getByText("Send answer").click();
  await expect
    .poll(() => mock.requests.filter((r) => r.method === "SendStreamingMessage").length)
    .toBe(2);
  const reply = rec(rec(mock.requests.at(-1)?.params).message);
  expect(list(reply.parts)[0]?.data).toMatchObject([
    { version: "v0.9.1", action: { name: "answer" } },
  ]);
  await mock.close();
});

test("command: the palette opens on / and the token is highlighted", async ({ page }) => {
  const mock = await openWith(page, "idle");
  await page.getByPlaceholder(/Message demo-agent/).fill("/mo");
  await expect(page.getByTestId("palette")).toContainText("/model");
  await shoot(page, "command");
  await page.getByPlaceholder(/Message demo-agent/).fill("/model gpt-6.1");
  await expect(page.getByTestId("composer-command")).toHaveText("/model");
  await mock.close();
});

test("unknown: an unknown command is answered without the model", async ({ page }) => {
  const mock = await openWith(page, "unknown");
  await send(page, "/modle gpt-6.1");
  await expect(page.getByText(/Unknown command \/modle/)).toBeVisible();
  await expect(page.getByTestId("turn-user").locator(".cmd")).toHaveCount(0);
  await shoot(page, "unknown");
  await mock.close();
});

test("done: complete is confirmed, then the controls close", async ({ page }) => {
  const mock = await openWith(page, "done");
  await send(page, USER);
  await expect(page.getByTestId("state-badge")).toHaveText(/input-required/);
  await page.getByTestId("complete").click();
  const dialog = page.getByTestId("confirm-complete");
  await expect(dialog).toContainText("1 running sub-task will be canceled");
  await dialog.getByRole("button", { name: "Complete" }).click();
  await expect(page.getByTestId("state-badge")).toHaveText(/completed/);
  await expect(page.getByTestId("complete")).toBeDisabled();
  await expect(page.getByTestId("cancel")).toBeDisabled();
  await shoot(page, "done");
  const transition = rec(rec(rec(mock.requests.at(-1)?.params).message).metadata);
  expect(Object.values(transition).some((m) => rec(m).transition === "complete")).toBe(true);
  await mock.close();
});

test("tool and denied: the tool card states", async ({ page }) => {
  // Tool activity is not on the wire yet; the `Tool` composition is shown from the
  // fixtures page the test build adds (design/web-renderer.html, states tool and denied).
  await page.goto("/fixtures/?state=tool");
  await expect(page.getByTestId("tool-card")).toHaveCount(2);
  await shoot(page, "tool");
  await page.goto("/fixtures/?state=denied");
  await expect(page.getByTestId("tool-card")).toHaveCount(1);
  await expect(page.getByTestId("fixture-denied")).toContainText("denied by pre-hook");
  await shoot(page, "denied");
});
