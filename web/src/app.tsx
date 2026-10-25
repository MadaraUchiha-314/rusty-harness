// The web renderer: one harness over the official SDK, one store per task, the AI
// Elements for the chrome (design.md § Web renderer; R26).
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { StreamResponse } from "@a2a-js/sdk";
import {
  a2uiAction,
  cancel,
  complete,
  connect,
  resubscribe,
  send,
  type Harness,
  type Outgoing,
} from "@/a2a/client";
import { isKey, t } from "@/i18n";
import { isTerminal, reduce, type TaskView, type Turn } from "@/store/task";
import type { A2uiAction } from "@/components/app/a2ui-block";
import { Composer, type CommandInfo } from "@/components/app/composer";
import { CompleteConfirmation } from "@/components/app/confirm";
import { ConversationView } from "@/components/app/conversation";
import { Header } from "@/components/app/header";
import { ModelPicker } from "@/components/app/model-picker";
import { Sidebar } from "@/components/app/sidebar";
import { TaskPanel } from "@/components/app/task-panel";

/** Where the harness is: `?url=`, else the page's own origin (the binary serves both). */
export function harnessUrl(location: { search: string; origin: string }): string {
  const given = new URLSearchParams(location.search).get("url");
  return given ?? `${location.origin}/`;
}

const COMMAND_DESCRIPTIONS: Record<string, string> = {
  model: "command.model.description",
  effort: "command.effort.description",
  exit: "command.exit.description",
};

export function App() {
  const url = useMemo(() => harnessUrl(window.location), []);
  const [harness, setHarness] = useState<Harness>();
  const [error, setError] = useState<string>();
  const [tasks, setTasks] = useState<Map<string, TaskView>>(new Map());
  const [current, setCurrent] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [picker, setPicker] = useState(false);
  const [notice, setNotice] = useState<string>();
  const lastFinished = useRef<{ id: string; contextId: string } | undefined>(undefined);

  useEffect(() => {
    connect(url).then(setHarness, (e: unknown) =>
      setError(t("web.connect.failed", { url, reason: String(e) })),
    );
  }, [url]);

  const apply = useCallback((response: StreamResponse) => {
    setTasks((prev) => {
      const payload = response.payload;
      if (!payload) return prev;
      const id =
        payload.$case === "task"
          ? payload.value.id
          : payload.$case === "message"
            ? payload.value.taskId
            : payload.value.taskId;
      const next = new Map(prev);
      const view = reduce(prev.get(id), response, new Date().toISOString());
      if (view) next.set(id, view);
      setCurrent((c) => c ?? id);
      return next;
    });
  }, []);

  const drive = useCallback(
    async (stream: AsyncGenerator<StreamResponse>) => {
      setBusy(true);
      try {
        for await (const event of stream) apply(event);
      } catch (e) {
        setNotice(t("web.send_failed", { reason: String(e) }));
      } finally {
        setBusy(false);
      }
    },
    [apply],
  );

  const task = current ? tasks.get(current) : undefined;
  const agent = harness?.card.name ?? "rusty-harness";
  const commands: CommandInfo[] = (harness?.commands ?? []).map((name) => {
    const descriptionKey = COMMAND_DESCRIPTIONS[name];
    return {
      name,
      description: descriptionKey && isKey(descriptionKey) ? t(descriptionKey) : "",
    };
  });

  const outgoingFor = useCallback(
    (base: Outgoing): Outgoing => {
      if (task && !isTerminal(task.state)) {
        return { ...base, taskId: task.id, contextId: task.contextId };
      }
      const finished = task && isTerminal(task.state) ? task : undefined;
      if (finished) {
        lastFinished.current = { id: finished.id, contextId: finished.contextId };
        setCurrent(undefined);
        return { ...base, contextId: finished.contextId, referenceTaskIds: [finished.id] };
      }
      return base;
    },
    [task],
  );

  const onSend = useCallback(
    (text: string) => {
      if (!harness) return;
      if (text === "/model" && harness.models.length > 0) {
        setPicker(true);
        return;
      }
      void drive(send(harness.client, outgoingFor({ text })));
    },
    [drive, harness, outgoingFor],
  );

  const onA2uiAction = useCallback(
    (turn: Turn, action: A2uiAction) => {
      if (!harness || !task) return;
      const payload = {
        name: action.name,
        surfaceId: action.surfaceId,
        sourceComponentId: action.sourceComponentId,
        timestamp: action.timestamp,
        context: action.context,
      };
      void drive(
        send(harness.client, a2uiAction(payload, turn.channel?.channel, task.id, task.contextId)),
      );
    },
    [drive, harness, task],
  );

  const onComplete = useCallback(() => {
    setConfirm(false);
    if (!harness || !task) return;
    void drive(complete(harness.client, task.id, task.contextId));
  }, [drive, harness, task]);

  const onCancel = useCallback(() => {
    if (!harness || !task) return;
    cancel(harness.client, task.id).then(
      (canceled) => apply({ payload: { $case: "task", value: canceled } }),
      (e: unknown) => setNotice(t("web.send_failed", { reason: String(e) })),
    );
  }, [apply, harness, task]);

  const onSelect = useCallback(
    (id: string) => {
      setCurrent(id);
      const view = tasks.get(id);
      if (harness && view && !isTerminal(view.state)) {
        void drive(resubscribe(harness.client, id));
      }
    },
    [drive, harness, tasks],
  );

  const title = task?.meta?.name || task?.turns.find((x) => x.role === "user")?.text || agent;
  const durableParam: unknown = harness?.card.capabilities?.extensions.find((e) =>
    e.uri.endsWith("/task/v1"),
  )?.params?.durable;
  const durable = typeof durableParam === "string" ? durableParam : "—";

  if (error) {
    return (
      <main className="grid h-full place-items-center p-6 text-center">
        <p role="alert">{error}</p>
      </main>
    );
  }
  if (!harness) {
    return (
      <main className="grid h-full place-items-center p-6 text-muted-foreground">
        <p>{t("web.connect.connecting", { url })}</p>
      </main>
    );
  }
  return (
    <div className="grid h-full grid-cols-1 md:grid-cols-[240px_minmax(0,1fr)] lg:grid-cols-[240px_minmax(0,1fr)_300px]">
      <Sidebar
        agent={agent}
        tasks={[...tasks.values()]}
        current={current}
        cardUrl={`${url}.well-known/agent-card.json`}
        onSelect={onSelect}
        onNew={() => setCurrent(undefined)}
      />
      <main className="flex min-h-0 min-w-0 flex-col">
        <Header task={task} title={title} onComplete={() => setConfirm(true)} onCancel={onCancel} />
        <ConversationView
          task={task}
          agent={agent}
          commands={harness.commands}
          busy={busy}
          onA2uiAction={onA2uiAction}
        />
        {notice && (
          <p className="mx-auto w-full max-w-3xl px-4 text-danger text-xs" role="status">
            {notice}
          </p>
        )}
        <div className="border-t bg-card p-3">
          <Composer
            agent={agent}
            commands={commands}
            busy={busy}
            disabled={false}
            onSend={onSend}
          />
        </div>
      </main>
      <TaskPanel task={task} durable={durable} />
      <CompleteConfirmation
        open={confirm}
        subTasks={task?.meta?.sub_tasks?.length ?? 0}
        onConfirm={onComplete}
        onClose={() => setConfirm(false)}
      />
      <ModelPicker
        open={picker}
        models={harness.models}
        onPick={(alias) => {
          setPicker(false);
          onSend(`/model ${alias}`);
        }}
        onClose={() => setPicker(false)}
      />
    </div>
  );
}
