import { defineConfig } from "vitest/config";

// The conformance suite boots the server binary and drives it with the official SDK;
// it runs in Node, apart from the browser component tests in `vite.config.ts`.
export default defineConfig({
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
  },
});
