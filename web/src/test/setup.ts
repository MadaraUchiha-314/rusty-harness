// oxlint-disable-next-line import/no-unassigned-import -- registers the DOM matchers
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// Without vitest globals, Testing Library does not unmount between tests on its own.
afterEach(() => {
  cleanup();
});

// jsdom has no ResizeObserver or scrollIntoView; the AI Elements call both.
class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}
if (!("ResizeObserver" in globalThis)) {
  Object.assign(globalThis, { ResizeObserver: ResizeObserverStub });
}
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}
