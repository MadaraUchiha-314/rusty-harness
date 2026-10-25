import {
  Conversation,
  ConversationContent,
  ConversationEmptyState,
  ConversationScrollButton,
} from "@/components/ai-elements/conversation";
import { Message, MessageContent, MessageResponse } from "@/components/ai-elements/message";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { catalogKey, t } from "@/i18n";
import {
  isInputRequired,
  isTerminal,
  isWorking,
  stateName,
  type ArtifactView,
  type TaskView,
  type Turn,
} from "@/store/task";
import { A2uiBlock, type A2uiAction } from "./a2ui-block";
import { Highlight } from "./highlight";

function clock(at: string | undefined): string {
  if (!at) return "";
  const d = new Date(at);
  return Number.isNaN(d.getTime()) ? "" : d.toISOString().slice(11, 16);
}

function Avatar({ who }: { who: "user" | "agent" }) {
  const initials = who === "user" ? "yo" : "rh";
  return (
    <div
      aria-hidden="true"
      className={
        who === "user"
          ? "grid size-7 place-items-center rounded-lg bg-primary text-xs font-semibold text-primary-foreground"
          : "grid size-7 place-items-center rounded-lg bg-agent-soft text-xs font-semibold text-agent"
      }
    >
      {initials}
    </div>
  );
}

function ArtifactRow({ artifact }: { artifact: ArtifactView }) {
  const image = artifact.parts.find((p) => p.mediaType.startsWith("image/"));
  let src: string | undefined;
  if (image?.content?.$case === "url") src = image.content.value;
  if (image?.content?.$case === "raw") {
    // The SDK hands raw bytes over as a Buffer; `isView` works across realms where
    // `instanceof Uint8Array` does not.
    const raw: unknown = image.content.value;
    if (ArrayBuffer.isView(raw)) {
      const bytes = new Uint8Array(raw.buffer, raw.byteOffset, raw.byteLength);
      let binary = "";
      for (const b of bytes) binary += String.fromCharCode(b);
      src = `data:${image.mediaType};base64,${btoa(binary)}`;
    }
  }
  const text = artifact.parts
    .map((p) => (p.content?.$case === "text" ? p.content.value : undefined))
    .filter((v): v is string => v !== undefined)
    .join("\n");
  return (
    <div className="rounded-lg border bg-card p-3 text-sm" data-testid="artifact">
      <div className="flex items-center gap-2">
        <span className="font-medium">{artifact.name}</span>
        <span className="text-muted-foreground text-xs">
          {t("web.artifact", { name: artifact.description || artifact.id })}
        </span>
        {src && (
          <Button asChild size="sm" variant="outline" className="ml-auto">
            <a href={src} target="_blank" rel="noreferrer">
              {t("web.open")}
            </a>
          </Button>
        )}
      </div>
      {src && (
        <img
          alt={artifact.description || artifact.name}
          className="mt-2 max-h-72 rounded-md"
          src={src}
        />
      )}
      {text && <pre className="mt-2 overflow-auto rounded-md bg-muted p-2 text-xs">{text}</pre>}
    </div>
  );
}

function TurnView({
  turn,
  agent,
  commands,
  streaming,
  index,
  onA2uiAction,
}: {
  turn: Turn;
  agent: string;
  commands: readonly string[];
  streaming: boolean;
  index: number;
  onA2uiAction: (turn: Turn, action: A2uiAction) => void;
}) {
  const who = turn.role === "user" ? t("web.you") : agent;
  let note: string | undefined;
  if (turn.role === "agent" && turn.channel) {
    note = t("web.asked_on_channel", { channel: turn.channel.channel });
  } else if (turn.role === "agent" && streaming) {
    note = t("web.streaming");
  } else if (turn.role === "agent") {
    note = t("web.turn", { n: index });
  }
  return (
    <Message from={turn.role === "user" ? "user" : "assistant"} data-testid={`turn-${turn.role}`}>
      <div className="flex items-start gap-2.5">
        {turn.role === "agent" && <Avatar who="agent" />}
        <div className="min-w-0 flex-1">
          <div className="mb-1 flex items-center gap-2 text-muted-foreground text-xs">
            <span>{who}</span>
            {clock(turn.at) && <span>{clock(turn.at)}</span>}
            {note && <span>· {note}</span>}
          </div>
          <MessageContent>
            {turn.role === "user" ? (
              <p className="whitespace-pre-wrap">
                <Highlight text={turn.text} commands={commands} />
              </p>
            ) : turn.a2ui.length > 0 ? (
              <A2uiBlock messages={turn.a2ui} onAction={(action) => onA2uiAction(turn, action)} />
            ) : (
              <MessageResponse>{turn.text}</MessageResponse>
            )}
            {turn.role === "agent" && turn.a2ui.length > 0 && turn.text && (
              <p className="text-muted-foreground text-xs">{turn.text}</p>
            )}
          </MessageContent>
          {streaming && turn.role === "agent" && (
            <span
              aria-hidden="true"
              className="ml-0.5 inline-block h-4 w-2 animate-pulse rounded-sm bg-agent align-text-bottom"
            />
          )}
        </div>
      </div>
    </Message>
  );
}

export function ConversationView({
  task,
  agent,
  commands,
  busy,
  onA2uiAction,
}: {
  task: TaskView | undefined;
  agent: string;
  commands: readonly string[];
  busy: boolean;
  onA2uiAction: (turn: Turn, action: A2uiAction) => void;
}) {
  const turns = task?.turns ?? [];
  let lastAgent: Turn | undefined;
  for (const x of turns) if (x.role === "agent") lastAgent = x;
  let agentIndex = 0;
  return (
    <Conversation className="min-h-0 flex-1" aria-live="polite">
      <ConversationContent className="mx-auto w-full max-w-3xl gap-5">
        {turns.length === 0 && (
          <ConversationEmptyState
            title={t("tui.prompt.first_message")}
            description={t("tui.prompt.long_running")}
          />
        )}
        {turns.map((turn) => {
          if (turn.role === "agent") agentIndex += 1;
          return (
            <TurnView
              key={turn.id}
              turn={turn}
              agent={agent}
              commands={commands}
              index={agentIndex}
              streaming={busy && turn === lastAgent && task !== undefined && isWorking(task.state)}
              onA2uiAction={onA2uiAction}
            />
          );
        })}
        {task?.artifacts.map((a) => (
          <ArtifactRow key={a.id} artifact={a} />
        ))}
        {task?.retracted.length ? (
          <p className="text-muted-foreground text-xs" data-testid="retracted">
            {t("web.retracted")}
          </p>
        ) : null}
        {task?.meta?.error && (
          <Badge variant="destructive" className="w-fit">
            {t("web.error", { kind: task.meta.error.kind, message: task.meta.error.message })}
          </Badge>
        )}
        {task && isInputRequired(task.state) && task.subState?.kind === "awaiting-participant" && (
          <p className="text-muted-foreground text-xs">⏳ {t("web.a2ui.waiting")}</p>
        )}
        {task && isInputRequired(task.state) && task.subState?.kind !== "awaiting-participant" && (
          <p className="text-muted-foreground text-xs">{t("web.waiting_next")}</p>
        )}
        {task && isTerminal(task.state) && (
          <p className="text-muted-foreground text-xs" data-testid="terminal-note">
            ✓{" "}
            {t(
              catalogKey(
                `tui.status.${stateName(task.state).replace("-", "_")}`,
                "tui.status.completed",
              ),
            )}
            {task.updatedAt ? ` · ${clock(task.updatedAt)}` : ""}
          </p>
        )}
      </ConversationContent>
      <ConversationScrollButton />
    </Conversation>
  );
}
