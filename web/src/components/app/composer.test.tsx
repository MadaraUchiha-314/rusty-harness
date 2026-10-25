// Feature: Web renderer — the composer on the Prompt Input element (T1)
// Requirement: docs/specs/issue-3/requirements.md#R26
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Composer } from "./composer";

const commands = [
  { name: "model", description: "Pick the model" },
  { name: "effort", description: "Set the effort" },
  { name: "exit", description: "Leave" },
];

describe("the composer", () => {
  it("sends on Enter and keeps Shift+Enter as a newline", async () => {
    const onSend = vi.fn<() => void>();
    render(
      <Composer
        agent="demo-agent"
        commands={commands}
        busy={false}
        disabled={false}
        onSend={onSend}
      />,
    );
    const box = screen.getByPlaceholderText(/Message demo-agent/);
    await userEvent.type(box, "line one{Shift>}{Enter}{/Shift}line two");
    expect(onSend).not.toHaveBeenCalled();
    await userEvent.type(box, "{Enter}");
    await waitFor(() => expect(onSend).toHaveBeenCalledWith("line one\nline two"));
  });

  it("opens the palette on / and highlights a registered command", async () => {
    const onSend = vi.fn<() => void>();
    render(
      <Composer
        agent="demo-agent"
        commands={commands}
        busy={false}
        disabled={false}
        onSend={onSend}
      />,
    );
    const box = screen.getByPlaceholderText(/Message demo-agent/);
    await userEvent.type(box, "/mo");
    const palette = screen.getByTestId("palette");
    expect(palette).toHaveTextContent("/model");
    expect(palette).not.toHaveTextContent("/exit");
    await userEvent.type(box, "del gpt");
    expect(screen.getByTestId("composer-command")).toHaveTextContent("/model");
    expect(screen.queryByTestId("palette")).toBeNull();
    await userEvent.keyboard("{Escape}");
  });

  it("does not highlight an unknown command", async () => {
    render(
      <Composer
        agent="demo-agent"
        commands={commands}
        busy={false}
        disabled={false}
        onSend={vi.fn<() => void>()}
      />,
    );
    const box = screen.getByPlaceholderText(/Message demo-agent/);
    await userEvent.type(box, "/modle x");
    expect(screen.queryByTestId("composer-command")).toBeNull();
  });
});
