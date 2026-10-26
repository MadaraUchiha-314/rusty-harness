import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { catalogKey, t } from "@/i18n";
import { isTerminal, stateName, type TaskView } from "@/store/task";

function statusKey(name: string) {
  return catalogKey(`tui.status.${name.replace("-", "_")}`, "tui.status.submitted");
}

/** The state vocabulary as a badge: working pulses, waiting is amber, done is green. */
export function StateBadge({ task }: { task: TaskView | undefined }) {
  if (!task) {
    return <Badge variant="outline">{t("web.no_task")}</Badge>;
  }
  const name = stateName(task.state);
  const tone =
    name === "working" || name === "submitted"
      ? "bg-info-soft text-info"
      : name === "input-required" || name === "auth-required"
        ? "bg-warn-soft text-warn"
        : name === "completed"
          ? "bg-ok-soft text-ok"
          : name === "canceled"
            ? "bg-muted text-muted-foreground"
            : "bg-danger-soft text-danger";
  return (
    <Badge className={`gap-1.5 border-transparent ${tone}`} data-testid="state-badge">
      {name === "working" && (
        <span aria-hidden="true" className="size-1.5 animate-pulse rounded-full bg-current" />
      )}
      {t(statusKey(name))}
    </Badge>
  );
}

export function Header({
  task,
  title,
  onComplete,
  onCancel,
}: {
  task: TaskView | undefined;
  title: string;
  onComplete: () => void;
  onCancel: () => void;
}) {
  const open = task !== undefined && !isTerminal(task.state);
  const sub = task?.subState
    ? t(
        catalogKey(
          `tui.sub_state.${task.subState.kind.replace(/-/g, "_")}`,
          "tui.sub_state.paused",
        ),
      )
    : undefined;
  return (
    <header className="flex flex-wrap items-center gap-2.5 border-b bg-card px-4 py-2.5">
      <h1 className="m-0 text-[15px] font-semibold">{title}</h1>
      <StateBadge task={task} />
      {sub && <span className="text-muted-foreground text-xs">{sub}</span>}
      <span className="flex-1" />
      <Button
        size="sm"
        variant="outline"
        disabled={!open}
        title={t("web.complete_title")}
        onClick={onComplete}
        data-testid="complete"
      >
        ✓ {t("web.complete")}
      </Button>
      <Button
        size="sm"
        variant="outline"
        className="text-danger"
        disabled={!open}
        title={t("web.cancel_title")}
        onClick={onCancel}
        data-testid="cancel"
      >
        {t("web.cancel")}
      </Button>
    </header>
  );
}
