import {
  ModelSelector,
  ModelSelectorContent,
  ModelSelectorEmpty,
  ModelSelectorInput,
  ModelSelectorItem,
  ModelSelectorList,
} from "@/components/ai-elements/model-selector";
import { t } from "@/i18n";

/** `/model` as the `Model Selector` element: the aliases the harness advertises. */
export function ModelPicker({
  open,
  models,
  onPick,
  onClose,
}: {
  open: boolean;
  models: readonly string[];
  onPick: (alias: string) => void;
  onClose: () => void;
}) {
  return (
    <ModelSelector open={open} onOpenChange={(o) => !o && onClose()}>
      <ModelSelectorContent title={t("web.model.title")}>
        <ModelSelectorInput placeholder={t("web.model.pick")} />
        <ModelSelectorList>
          <ModelSelectorEmpty>{t("web.model.pick")}</ModelSelectorEmpty>
          {models.map((alias) => (
            <ModelSelectorItem key={alias} value={alias} onSelect={() => onPick(alias)}>
              {alias}
            </ModelSelectorItem>
          ))}
        </ModelSelectorList>
      </ModelSelectorContent>
    </ModelSelector>
  );
}
