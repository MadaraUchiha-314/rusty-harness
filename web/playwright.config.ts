import { defineConfig, devices } from "@playwright/test";

// The browser legs (testing-plan T5, T9): the production bundle served by `vite preview`
// against the scripted mock harness in tests/support. Screenshot baselines are pinned to
// the Playwright container image CI runs in (`bun run test:visual:update`).
export default defineConfig({
  testDir: "tests",
  testMatch: /(visual|a11y)\/.*\.spec\.ts/,
  fullyParallel: false,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  snapshotPathTemplate: "{testDir}/visual/__screenshots__/{arg}{ext}",
  expect: {
    // The baselines are captured in the Playwright image (CI); a host with other font
    // hinting lands within a few percent of them.
    toHaveScreenshot: { maxDiffPixelRatio: 0.04, animations: "disabled", caret: "hide" },
  },
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "retain-on-failure",
    video: process.env.PW_VIDEO ? "on" : "off",
  },
  webServer: {
    command: "VITE_FIXTURES=1 vite build && vite preview --host 127.0.0.1 --port 4173 --strictPort",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
  projects: [
    {
      name: "visual",
      testMatch: /visual\/states\.spec\.ts/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } },
    },
    {
      name: "phone",
      testMatch: /visual\/phone\.spec\.ts/,
      use: { ...devices["Pixel 7"], viewport: { width: 420, height: 860 } },
    },
    {
      name: "a11y",
      testMatch: /a11y\/.*\.spec\.ts/,
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } },
    },
  ],
});
