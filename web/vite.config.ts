import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const src = fileURLToPath(new URL("./src", import.meta.url));

// The fixtures page (tests/visual) is built only when asked, never in the shipped bundle.
const inputs: Record<string, string> = {
  main: fileURLToPath(new URL("./index.html", import.meta.url)),
  ...(process.env.VITE_FIXTURES
    ? { fixtures: fileURLToPath(new URL("./fixtures/index.html", import.meta.url)) }
    : {}),
};

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": src } },
  build: { rollupOptions: { input: inputs } },
  // The string catalog is read straight from the crate (R33.7): one `en.json`.
  server: { fs: { allow: [fileURLToPath(new URL("..", import.meta.url))] } },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test/setup.ts"],
    css: false,
  },
});
