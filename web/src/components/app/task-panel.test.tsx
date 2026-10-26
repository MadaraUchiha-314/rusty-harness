// Feature: Web renderer — the task panel and header (T1)
// Requirement: docs/specs/issue-3/requirements.md#R26
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { TaskView } from "@/store/task";
import { CompleteConfirmation } from "./confirm";
import { Header } from "./header";
import { TaskPanel } from "./task-panel";

const task: TaskView = {
  id: "task_01HZX4V3K9N2",
  contextId: "ctx_9a1f",
  state: 6,
  subState: { kind: "awaiting-next-turn" },
  meta: {
    name: "Plan the Q4 release notes",
    mode: "long-running",
    sub_tasks: [{ task: "classify-25-prs" }],
  },
  participants: [
    { id: "you", display: "you", kind: "human", roles: ["reporter", "admin"] },
    { id: "demo-agent", display: "demo-agent", kind: "system", roles: ["assignee"] },
  ],
  plan: {
    id: "p1",
    task: "task_01HZX4V3K9N2",
    version: 3,
    steps: [
      { id: "collect", name: "Collect merged PRs", description: "", status: "done" },
      { id: "group", name: "Group by area", description: "", status: "running" },
      { id: "draft", name: "Draft notes", description: "", status: "pending" },
    ],
  },
  turns: [],
  artifacts: [],
  activity: [{ at: "2026-10-24T12:30:00Z", kind: "created", detail: "submitted" }],
  retracted: [],
};

describe("the task panel", () => {
  it("shows the record, participants, plan steps, sub-tasks and activity", () => {
    render(<TaskPanel task={task} durable="remote" />);
    const panel = screen.getByTestId("task-panel");
    expect(panel).toHaveTextContent("task_01HZX4V3K9N2");
    expect(panel).toHaveTextContent("input-required");
    expect(panel).toHaveTextContent("awaiting-next-turn");
    expect(panel).toHaveTextContent("reporter · admin");
    expect(panel.querySelectorAll("[data-status]")).toHaveLength(3);
    expect(panel.querySelector('[data-status="running"]')).toHaveTextContent("Group by area");
    expect(panel).toHaveTextContent("1 sub-task");
    expect(panel).toHaveTextContent("classify-25-prs");
    expect(panel).toHaveTextContent("12:30");
  });
});

describe("the header", () => {
  it("offers complete and cancel while the task is open and not after", async () => {
    const onComplete = vi.fn<() => void>();
    const onCancel = vi.fn<() => void>();
    const { rerender } = render(
      <Header
        task={task}
        title="Plan the Q4 release notes"
        onComplete={onComplete}
        onCancel={onCancel}
      />,
    );
    expect(screen.getByTestId("state-badge")).toHaveTextContent("input-required");
    expect(screen.getByText("awaiting next turn")).toBeInTheDocument();
    await userEvent.click(screen.getByTestId("complete"));
    await userEvent.click(screen.getByTestId("cancel"));
    expect(onComplete).toHaveBeenCalledOnce();
    expect(onCancel).toHaveBeenCalledOnce();
    rerender(
      <Header task={{ ...task, state: 3 }} title="x" onComplete={onComplete} onCancel={onCancel} />,
    );
    expect(screen.getByTestId("complete")).toBeDisabled();
    expect(screen.getByTestId("cancel")).toBeDisabled();
    expect(screen.getByTestId("state-badge")).toHaveTextContent("completed");
  });

  it("confirms completion and says how many sub-tasks the policy cancels", async () => {
    const onConfirm = vi.fn<() => void>();
    render(
      <CompleteConfirmation
        open
        subTasks={1}
        onConfirm={onConfirm}
        onClose={vi.fn<() => void>()}
      />,
    );
    expect(screen.getByTestId("confirm-complete")).toHaveTextContent(
      "1 running sub-task will be canceled",
    );
    await userEvent.click(screen.getByRole("button", { name: "Complete" }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
