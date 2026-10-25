import {
  PromptInput,
  PromptInputBody,
  PromptInputFooter,
  PromptInputProvider,
  PromptInputSubmit,
  PromptInputTextarea,
  PromptInputTools,
  usePromptInputController,
} from "@/components/ai-elements/prompt-input";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { t } from "@/i18n";
import { commandTokens } from "@/store/task";
import { useState } from "react";

export interface CommandInfo {
  name: string;
  description: string;
}

function Palette({
  commands,
  query,
  onPick,
}: {
  commands: readonly CommandInfo[];
  query: string;
  onPick: (name: string) => void;
}) {
  const matching = commands.filter((c) => c.name.startsWith(query));
  return (
    <Command className="rounded-lg border shadow-md" data-testid="palette" aria-label="Commands">
      <CommandList>
        <CommandEmpty>{t("command.unknown_no_suggestion", { name: `/${query}` })}</CommandEmpty>
        <CommandGroup>
          {matching.map((c) => (
            <CommandItem key={c.name} value={c.name} onSelect={() => onPick(c.name)}>
              <span className="cmd">/{c.name}</span>
              <span className="text-muted-foreground">{c.description}</span>
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </Command>
  );
}

function Inner({
  agent,
  commands,
  busy,
  disabled,
}: {
  agent: string;
  commands: readonly CommandInfo[];
  busy: boolean;
  disabled: boolean;
}) {
  const controller = usePromptInputController();
  const value = controller.textInput.value;
  const [open, setOpen] = useState(true);
  const query = value.startsWith("/") ? (value.slice(1).split(/\s/)[0] ?? "") : undefined;
  const names = commands.map((c) => c.name);
  const token = commandTokens(value, names)[0];
  const showPalette = open && query !== undefined && !value.includes(" ");
  return (
    <>
      {showPalette && (
        <Palette
          commands={commands}
          query={query}
          onPick={(name) => {
            controller.textInput.setInput(`/${name} `);
            setOpen(false);
          }}
        />
      )}
      <PromptInputBody
        onKeyDownCapture={(e) => {
          // The element owns `onKeyDown` (Enter sends, Shift+Enter breaks the line);
          // Escape closes the palette from the capture phase.
          if (e.key === "Escape") setOpen(false);
        }}
      >
        <PromptInputTextarea
          aria-label={t("web.composer.placeholder", { agent })}
          placeholder={t("web.composer.placeholder", { agent })}
          disabled={disabled}
          onChange={() => setOpen(true)}
        />
      </PromptInputBody>
      <PromptInputFooter>
        <PromptInputTools>
          {token && (
            <span className="cmd" data-testid="composer-command">
              /{token.name}
            </span>
          )}
          <span className="text-muted-foreground text-xs">{t("web.composer.hint")}</span>
        </PromptInputTools>
        <PromptInputSubmit
          status={busy ? "streaming" : undefined}
          disabled={disabled || value.trim().length === 0}
          aria-label={t("tui.hint.send")}
        />
      </PromptInputFooter>
    </>
  );
}

/** The composer: the `Prompt Input` element with the command palette on `/` (R26.1,
 * R26.5). `Enter` sends, `Shift+Enter` breaks the line, `Esc` closes the palette. */
export function Composer({
  agent,
  commands,
  busy,
  disabled,
  onSend,
}: {
  agent: string;
  commands: readonly CommandInfo[];
  busy: boolean;
  disabled: boolean;
  onSend: (text: string) => void;
}) {
  return (
    <PromptInputProvider>
      <PromptInput
        className="mx-auto w-full max-w-3xl"
        onSubmit={({ text }) => {
          if (text.trim()) onSend(text.trim());
        }}
      >
        <Inner agent={agent} commands={commands} busy={busy} disabled={disabled} />
      </PromptInput>
    </PromptInputProvider>
  );
}
