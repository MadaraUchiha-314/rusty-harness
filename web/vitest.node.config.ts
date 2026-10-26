import { defineConfig } from "vitest/config";

// The Node legs: the conformance suite (boots the server binary and drives it with the
// official SDK) and the bundle scan; the component tests run in jsdom via vite.config.ts.
export default defineConfig({
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
  },
});
