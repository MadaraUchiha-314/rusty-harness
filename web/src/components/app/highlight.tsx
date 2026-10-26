import { commandTokens } from "@/store/task";

/** `text` with every registered command token wrapped in a `.cmd` span (R26.5). */
export function Highlight({ text, commands }: { text: string; commands: readonly string[] }) {
  const tokens = commandTokens(text, commands);
  if (tokens.length === 0) return <>{text}</>;
  const out: React.ReactNode[] = [];
  let pos = 0;
  for (const t of tokens) {
    if (t.start > pos) out.push(text.slice(pos, t.start));
    out.push(
      <span className="cmd" key={`${t.start}-${t.name}`} data-command={t.name}>
        {text.slice(t.start, t.end)}
      </span>,
    );
    pos = t.end;
  }
  if (pos < text.length) out.push(text.slice(pos));
  return <>{out}</>;
}
