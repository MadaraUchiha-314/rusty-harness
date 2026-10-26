import { Button } from "@/components/ui/button";
import { t } from "@/i18n";
import { stateName, type TaskView } from "@/store/task";

/** The task list: one row per task this session saw, the current one marked. */
export function Sidebar({
  agent,
  tasks,
  current,
  cardUrl,
  onSelect,
  onNew,
}: {
  agent: string;
  tasks: readonly TaskView[];
  current: string | undefined;
  cardUrl: string;
  onSelect: (id: string) => void;
  onNew: () => void;
}) {
  return (
    <nav
      className="hidden min-h-0 flex-col gap-2.5 border-r bg-card p-3 md:flex"
      aria-label={t("web.tasks")}
    >
      <div className="flex items-center gap-2 px-1.5 pb-2 font-semibold">
        <span aria-hidden="true" className="size-2.5 rounded-sm bg-agent" />
        rusty-harness{" "}
        <small className="font-mono font-normal text-muted-foreground text-[11px]">{agent}</small>
      </div>
      <Button variant="secondary" size="sm" onClick={onNew} data-testid="new-task">
        ＋ {t("web.new_task")}
      </Button>
      <ul className="m-0 flex list-none flex-col gap-0.5 overflow-auto p-0">
        {tasks.map((task) => {
          const name =
            task.meta?.name || task.turns.find((x) => x.role === "user")?.text || task.id;
          const turns = task.turns.filter((x) => x.role === "user").length;
          return (
            <li key={task.id}>
              <button
                type="button"
                aria-current={task.id === current ? "true" : undefined}
                onClick={() => onSelect(task.id)}
                className="grid w-full grid-cols-[8px_1fr] gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-muted aria-[current=true]:bg-muted"
              >
                <span
                  aria-hidden="true"
                  className="mt-1.5 size-2 rounded-full bg-muted-foreground data-[s=working]:bg-info data-[s=input-required]:bg-warn data-[s=completed]:bg-ok"
                  data-s={stateName(task.state)}
                />
                <span className="min-w-0">
                  <span className="block truncate font-medium">{name}</span>
                  <span className="block text-muted-foreground text-xs">
                    {task.meta?.mode ?? stateName(task.state)} · {t("web.turns", { n: turns })}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      <div className="mt-auto flex justify-between px-1.5 text-muted-foreground text-xs">
        <a href={cardUrl} target="_blank" rel="noreferrer">
          {t("web.agent_card")}
        </a>
        <span>a2a 1.0</span>
      </div>
    </nav>
  );
}
