// A tool call as the `Tool` element shows it: input, output, and the hook veto rendered
// in the card (design.md § Web renderer). The loop does not put tool activity on the
// wire yet; this composition is what will render it when it does.
import {
  Tool,
  ToolContent,
  ToolHeader,
  ToolInput,
  ToolOutput,
} from "@/components/ai-elements/tool";
import type { ToolUIPart } from "ai";

export interface ToolCall {
  id: string;
  name: string;
  input: unknown;
  output?: unknown;
  error?: string;
  /** `denied` when a pre hook vetoed it (R5): the model was told, the loop went on. */
  status: "running" | "completed" | "error" | "denied";
  seconds?: number;
}

function stateOf(call: ToolCall): ToolUIPart["state"] {
  switch (call.status) {
    case "running":
      return "input-available";
    case "completed":
      return "output-available";
    case "error":
      return "output-error";
    case "denied":
      return "output-denied";
    default:
      return "input-available";
  }
}

export function ToolCard({ call }: { call: ToolCall }) {
  const state = stateOf(call);
  return (
    <Tool defaultOpen={call.status !== "completed"} data-testid="tool-card">
      <ToolHeader type={`tool-${call.name}`} state={state} title={call.name} />
      <ToolContent>
        <ToolInput input={call.input} />
        {call.status !== "running" && <ToolOutput output={call.output} errorText={call.error} />}
      </ToolContent>
    </Tool>
  );
}
