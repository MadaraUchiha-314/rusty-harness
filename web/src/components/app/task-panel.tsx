import {
  Plan,
  PlanContent,
  PlanHeader,
  PlanTitle,
  PlanTrigger,
} from "@/components/ai-elements/plan";
import { Task, TaskContent, TaskItem, TaskTrigger } from "@/components/ai-elements/task";
import { t } from "@/i18n";
import { stateName, type TaskView } from "@/store/task";

function Row({ k, v }: { k: string; v: string }) {
  return (
    <>
      <dt className="text-muted-foreground">{k}</dt>
      <dd className="m-0 truncate font-mono text-xs" title={v}>
        {v}
      </dd>
    </>
  );
}

const STEP_GLYPH: Record<string, string> = {
  done: "■",
  running: "▣",
  pending: "□",
  skipped: "◌",
  failed: "✗",
};

/** The task panel: the record, the participants, the plan (the `Plan` element) with
 * sub-tasks (the `Task` element), and the activity timeline. */
export function TaskPanel({ task, durable }: { task: TaskView | undefined; durable: string }) {
  if (!task) {
    return (
      <aside className="hidden border-l bg-card p-4 text-muted-foreground text-sm lg:block">
        {t("web.no_task")}
      </aside>
    );
  }
  const subTasks = task.meta?.sub_tasks ?? [];
  return (
    <aside
      className="hidden min-h-0 flex-col gap-4 overflow-auto border-l bg-card p-4 text-sm lg:flex"
      data-testid="task-panel"
    >
      <section>
        <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {t("web.task.heading")}
        </h2>
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
          <Row k={t("web.task.id")} v={task.id} />
          <Row k={t("web.task.context")} v={task.contextId} />
          <Row k={t("web.task.state")} v={stateName(task.state)} />
          <Row k={t("web.task.sub_state")} v={task.subState?.kind ?? "—"} />
          <Row k={t("web.task.mode")} v={task.meta?.mode ?? "—"} />
          <Row k={t("web.task.durable")} v={durable} />
        </dl>
      </section>
      <section>
        <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {t("web.participants")}
        </h2>
        <ul className="m-0 flex list-none flex-col gap-1 p-0">
          {task.participants.map((p) => (
            <li key={p.id} className="flex items-center gap-2">
              <span className="font-medium">{p.display || p.id}</span>
              <span className="text-muted-foreground text-xs">{p.roles.join(" · ")}</span>
            </li>
          ))}
        </ul>
      </section>
      <Plan defaultOpen>
        <PlanHeader>
          <PlanTitle>{t("web.plan")}</PlanTitle>
          <PlanTrigger />
        </PlanHeader>
        <PlanContent>
          <ol className="m-0 flex list-none flex-col gap-1 p-0">
            {(task.plan?.steps ?? []).map((s) => (
              <li key={s.id} className="flex items-center gap-2" data-status={s.status}>
                <span aria-hidden="true">{STEP_GLYPH[s.status] ?? "□"}</span>
                <span>{s.name}</span>
                <span className="sr-only">{s.status}</span>
              </li>
            ))}
            {!task.plan && <li className="text-muted-foreground">{t("tui.plan.none")}</li>}
          </ol>
          {subTasks.length > 0 && (
            <Task defaultOpen>
              <TaskTrigger title={t("web.task.sub_tasks", { n: subTasks.length })} />
              <TaskContent>
                {subTasks.map((s) => (
                  <TaskItem key={s.task}>{s.task}</TaskItem>
                ))}
              </TaskContent>
            </Task>
          )}
        </PlanContent>
      </Plan>
      <section>
        <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {t("web.activity")}
        </h2>
        <ul className="m-0 flex list-none flex-col gap-1 p-0 text-xs">
          {task.activity.map((a, i) => (
            <li key={`${a.kind}-${i}`}>
              {a.at ? <b className="mr-1 font-mono">{a.at.slice(11, 16)}</b> : null}
              {a.kind} · {a.detail}
            </li>
          ))}
        </ul>
      </section>
    </aside>
  );
}
