// An A2UI payload rendered with the official React renderer (R25.4): the basic catalog's
// web components behind `A2uiSurface`, actions handed back as A2UI client messages.
import { A2uiSurface, type ReactCatalogComponent } from "@a2ui/react/v0_9";
import { MessageProcessor, type ActionPayload, type SurfaceModel } from "@a2ui/web_core/v0_9";
import { basicCatalog, injectBasicCatalogStyles } from "@a2ui/web_core/v0_9/basic_catalog";
import { useEffect, useRef, useState } from "react";
import { t } from "@/i18n";

let stylesInjected = false;

export type A2uiAction = ActionPayload;

export function A2uiBlock({
  messages,
  onAction,
  label,
}: {
  messages: unknown[];
  onAction: (action: A2uiAction) => void;
  label?: string;
}) {
  const handler = useRef(onAction);
  handler.current = onAction;
  const [processor] = useState(
    () => new MessageProcessor([basicCatalog], (action) => handler.current(action)),
  );
  const [surfaces, setSurfaces] = useState(() => [...processor.model.surfacesMap.values()]);
  const applied = useRef(0);

  useEffect(() => {
    if (!stylesInjected && typeof document !== "undefined") {
      injectBasicCatalogStyles(document);
      stylesInjected = true;
    }
    const sync = () => setSurfaces([...processor.model.surfacesMap.values()]);
    const created = processor.onSurfaceCreated(sync);
    const deleted = processor.onSurfaceDeleted(sync);
    return () => {
      created.unsubscribe();
      deleted.unsubscribe();
    };
  }, [processor]);

  useEffect(() => {
    // Messages only grow for one ask; feed the processor the ones it has not seen.
    if (messages.length > applied.current) {
      // The payload is the agent's JSON; the processor validates it against the catalog.
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- untyped wire data into the processor's input type
      const batch = messages.slice(applied.current) as Parameters<
        typeof processor.processMessages
      >[0];
      processor.processMessages(batch, { isUserActivated: false });
      applied.current = messages.length;
      setSurfaces([...processor.model.surfacesMap.values()]);
    }
  }, [messages, processor]);

  return (
    <div
      className="a2ui rounded-lg border bg-card p-4 shadow-xs"
      role="form"
      aria-label={label ?? t("tui.a2ui.title")}
      data-testid="a2ui-block"
    >
      {surfaces.map((surface) => (
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the basic catalog's web components are hosted by the React surface
        <A2uiSurface key={surface.id} surface={surface as SurfaceModel<ReactCatalogComponent>} />
      ))}
    </div>
  );
}
