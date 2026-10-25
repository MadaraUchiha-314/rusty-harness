// Feature: Web renderer — the conversation composed from the AI Elements (T1)
// Requirement: docs/specs/issue-3/requirements.md#R26
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { TaskView, Turn } from "@/store/task";
import { A2UI_MEDIA_TYPE } from "@/store/task";
import { ConversationView } from "./conversation";

function turn(id: string, role: Turn["role"], text: string, extra: Partial<Turn> = {}): Turn {
  return { id, role, text, parts: [], attempt: 0, a2ui: [], ...extra };
}

function view(turns: Turn[], state = 2): TaskView {
  return {
    id: "task_01",
    contextId: "ctx",
    state,
    participants: [],
    turns,
    artifacts: [],
    activity: [],
    retracted: [],
  };
}

describe("the conversation", () => {
  it("renders a user turn with its command highlighted and the agent's markdown reply", () => {
    const task = view([
      turn("u1", "user", "/model gpt-6.1 please"),
      turn("a1", "agent", "Three areas changed:\n\n- **API**: rate limits\n- Auth"),
    ]);
    render(
      <ConversationView
        task={task}
        agent="demo-agent"
        commands={["model"]}
        busy={false}
        onA2uiAction={vi.fn<() => void>()}
      />,
    );
    expect(screen.getByTestId("turn-user")).toHaveTextContent("/model gpt-6.1 please");
    expect(screen.getByText("/model")).toHaveClass("cmd");
    expect(screen.getByTestId("turn-agent")).toHaveTextContent("Three areas changed");
    expect(screen.getByText(/turn 1/)).toBeInTheDocument();
  });

  it("marks the last agent turn as streaming while a turn is in flight", () => {
    const task = view([turn("u1", "user", "go"), turn("a1", "agent", "Working on it")]);
    render(
      <ConversationView
        task={task}
        agent="demo-agent"
        commands={[]}
        busy={true}
        onA2uiAction={vi.fn<() => void>()}
      />,
    );
    expect(screen.getByText(/streaming/)).toBeInTheDocument();
  });

  it("renders an A2UI ask through the official renderer and says who was asked", () => {
    const a2ui = [
      {
        version: "v0.9.1",
        createSurface: {
          surfaceId: "ask-1",
          catalogId: "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json",
        },
      },
      {
        version: "v0.9.1",
        updateComponents: {
          surfaceId: "ask-1",
          components: [
            { id: "root", component: "Column", children: ["q"] },
            { id: "q", component: "Text", text: "Which audience are these notes for?" },
          ],
        },
      },
    ];
    const ask = turn("ask", "agent", "", {
      a2ui,
      channel: { channel: "ch-7f3a", ask: "ch-7f3a", question_kind: "a2ui" },
      parts: [
        {
          content: { $case: "data", value: a2ui },
          metadata: { mimeType: A2UI_MEDIA_TYPE },
          filename: "",
          mediaType: A2UI_MEDIA_TYPE,
        },
      ],
    });
    const task = { ...view([ask], 6), subState: { kind: "awaiting-participant", ask: "ch-7f3a" } };
    render(
      <ConversationView
        task={task}
        agent="demo-agent"
        commands={[]}
        busy={false}
        onA2uiAction={vi.fn<() => void>()}
      />,
    );
    expect(screen.getByText(/asked @you on channel ch-7f3a/)).toBeInTheDocument();
    expect(screen.getByTestId("a2ui-block")).toHaveAttribute("role", "form");
    expect(screen.getByText(/awaiting participant/)).toBeInTheDocument();
  });

  it("notes a withdrawn attempt and a finished task", () => {
    const task = {
      ...view([turn("a1", "agent", "done")], 3),
      retracted: [1],
      updatedAt: "2026-10-24T12:41:00Z",
    };
    render(
      <ConversationView
        task={task}
        agent="demo-agent"
        commands={[]}
        busy={false}
        onA2uiAction={vi.fn<() => void>()}
      />,
    );
    expect(screen.getByTestId("retracted")).toBeInTheDocument();
    expect(screen.getByTestId("terminal-note")).toHaveTextContent("completed · 12:41");
  });

  it("renders an image artifact as an image with its alt text (R28.2)", () => {
    const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    const task = {
      ...view([turn("a1", "agent", "Here is the chart.")], 6),
      artifacts: [
        {
          id: "chart-1",
          name: "chart.png",
          description: "revenue by quarter",
          attempt: 0,
          parts: [
            {
              content: { $case: "raw" as const, value: Buffer.from(png) },
              metadata: undefined,
              filename: "chart.png",
              mediaType: "image/png",
            },
          ],
        },
      ],
    };
    render(
      <ConversationView
        task={task}
        agent="demo-agent"
        commands={[]}
        busy={false}
        onA2uiAction={vi.fn<() => void>()}
      />,
    );
    const img = screen.getByAltText("revenue by quarter");
    expect(img.getAttribute("src")).toMatch(/^data:image\/png;base64,iVBORw0KGgo=$/);
    expect(screen.getByTestId("artifact")).toHaveTextContent("chart.png");
  });
});
