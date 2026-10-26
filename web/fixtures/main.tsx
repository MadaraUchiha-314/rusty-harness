// The states the wire cannot produce yet, mounted for the visual contract (T5): the
// `Tool` composition running, completed and denied by a hook. Built only when
// `VITE_FIXTURES=1` (the browser tests), never in the shipped bundle. `?state=tool` shows
// the running and completed cards, `?state=denied` the vetoed one; neither shows both.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { ToolCard } from "@/components/app/tool-card";
// oxlint-disable-next-line import/no-unassigned-import -- the stylesheet is a side effect
import "@/styles.css";

const input = { since: "v0.3.0", repo: "acme/platform" };
const output = [
  { number: 401, title: "feat(api): rate limits per tenant" },
  { number: 407, title: "fix(auth): refresh token rotation" },
  "… 23 more",
];

const state = new URLSearchParams(window.location.search).get("state") ?? "";
const showTool = state !== "denied";
const showDenied = state !== "tool";

const root = document.getElementById("root");
if (!root) throw new Error("no #root");
createRoot(root).render(
  <StrictMode>
    <main className="mx-auto flex max-w-3xl flex-col gap-6 p-6">
      {showTool && (
        <section data-testid="fixture-tool" className="flex flex-col gap-3">
          <ToolCard call={{ id: "t1", name: "changelog.list_merged", input, status: "running" }} />
          <ToolCard
            call={{
              id: "t2",
              name: "changelog.list_merged",
              input,
              output,
              status: "completed",
              seconds: 1.8,
            }}
          />
        </section>
      )}
      {showDenied && (
        <section data-testid="fixture-denied">
          <ToolCard
            call={{
              id: "t3",
              name: "shell.exec",
              input: { cmd: "rm -rf /" },
              status: "denied",
              error:
                'denied by pre-hook policy.tools: tool "shell.exec" is not allowed for participant role "reporter"; returned to the model as a tool error',
            }}
          />
        </section>
      )}
    </main>
  </StrictMode>,
);
