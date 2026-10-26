// Every string a person reads comes from the crate's catalog (R33.7, R25.6): the same
// `strings/en.json` the TUI and the CLI use, formatted with `intl-messageformat`.
import { IntlMessageFormat } from "intl-messageformat";
import en from "../../crates/rusty-harness/strings/en.json";

/** The catalog's keys. */
export type StringKey = keyof typeof en;

const cache = new Map<string, IntlMessageFormat>();
let locale = "en";

/** The locale used for formatting; the catalog itself is `en` until more ship. */
export function setLocale(tag: string): void {
  locale = tag;
  cache.clear();
}

/** The string for `key`, formatted with `args`. */
export function t(key: StringKey, args: Record<string, string | number | boolean> = {}): string {
  let fmt = cache.get(key);
  if (!fmt) {
    // `ignoreTag`: `<alias>` in a pattern is text, as it is for the crate's parser.
    fmt = new IntlMessageFormat(en[key], locale, undefined, { ignoreTag: true });
    cache.set(key, fmt);
  }
  const out = fmt.format(args);
  return typeof out === "string" ? out : String(out);
}

/** Whether `name` is a catalog key. */
export function isKey(name: string): name is StringKey {
  return Object.hasOwn(en, name);
}

/** The catalog key `name`, or `fallback` when the catalog has no such key. */
export function catalogKey(name: string, fallback: StringKey): StringKey {
  return isKey(name) ? name : fallback;
}

/** The raw catalog, for the strings test. */
export const catalog: Record<string, string> = en;
