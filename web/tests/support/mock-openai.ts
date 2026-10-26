// A scripted OpenAI chat-completions endpoint: each call streams the next scripted turn
// — text, or a tool call — in the shape the harness's OpenAI client reads.
import { createServer, type Server } from "node:http";
import { portOf } from "./json";

export type ScriptedTurn =
  | { text: string }
  | { tool: { id: string; name: string; arguments: Record<string, unknown> } };

function sse(chunks: unknown[]): string {
  return chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`).join("") + "data: [DONE]\n\n";
}

export interface MockOpenAi {
  port: number;
  calls: number;
  close(): Promise<void>;
}

export async function startMockOpenAi(turns: ScriptedTurn[]): Promise<MockOpenAi> {
  const state = { calls: 0 };
  const server: Server = createServer((_req, res) => {
    const turn = turns[Math.min(state.calls, turns.length - 1)];
    state.calls += 1;
    res.writeHead(200, { "content-type": "text/event-stream" });
    if (!turn) {
      res.end(sse([{ choices: [{ delta: {}, finish_reason: "stop" }] }]));
      return;
    }
    const chunks: unknown[] =
      "text" in turn
        ? [
            { choices: [{ delta: { content: turn.text } }] },
            {
              choices: [{ delta: {}, finish_reason: "stop" }],
              usage: { prompt_tokens: 12, completion_tokens: 4 },
            },
          ]
        : [
            {
              choices: [
                {
                  delta: {
                    tool_calls: [
                      {
                        index: 0,
                        id: turn.tool.id,
                        function: {
                          name: turn.tool.name,
                          arguments: JSON.stringify(turn.tool.arguments),
                        },
                      },
                    ],
                  },
                },
              ],
            },
            {
              choices: [{ delta: {}, finish_reason: "tool_calls" }],
              usage: { prompt_tokens: 12, completion_tokens: 8 },
            },
          ];
    res.end(sse(chunks));
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", () => r()));
  return {
    port: portOf(server.address()),
    get calls() {
      return state.calls;
    },
    close: () => new Promise<void>((r) => server.close(() => r())),
  };
}
