import { describe, expect, it } from "vitest";
import { hello } from "rusty-harness";

describe("hello", () => {
  it("resolves a Promise<string> from the Rust core", async () => {
    const greeting: Promise<string> = hello();
    await expect(greeting).resolves.toBe("Hello from rusty-harness!");
  });
});
