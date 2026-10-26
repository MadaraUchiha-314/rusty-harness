import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { t } from "@/i18n";

/** The complete confirmation (R21.7): what the transition does, and how many running
 * sub-tasks the default policy cancels. */
export function CompleteConfirmation({
  open,
  subTasks,
  onConfirm,
  onClose,
}: {
  open: boolean;
  subTasks: number;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent data-testid="confirm-complete">
        <DialogHeader>
          <DialogTitle>{t("web.confirm.title")}</DialogTitle>
          <DialogDescription>{t("tui.confirm.complete", { n: subTasks })}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {t("web.confirm.no")}
          </Button>
          <Button onClick={onConfirm}>{t("web.confirm.yes")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
