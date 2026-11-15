import { StateEffect, StateField, type EditorState } from "@codemirror/state";
import { SearchQuery } from "@codemirror/search";

export interface SearchRange { readonly from: number; readonly to: number }
export const setSearchScope = StateEffect.define<readonly SearchRange[] | null>();
export const editorSearchScope = StateField.define<readonly SearchRange[] | null>({
  create: () => null,
  update(value, transaction) {
    for (const effect of transaction.effects) if (effect.is(setSearchScope)) return effect.value;
    return value?.map(range => ({ from: transaction.changes.mapPos(range.from, -1), to: transaction.changes.mapPos(range.to, 1) })) ?? null;
  },
});
export function inSearchScope(_match: string, state: EditorState, from: number, to: number): boolean {
  const ranges = state.field(editorSearchScope, false);
  return !ranges || ranges.some(range => from >= range.from && to <= range.to);
}
export function preserveReplacementCase(value: string, original: string): string {
  if (!original) return value;
  for (const separator of ["-", "_"]) {
    const before = original.split(separator), after = value.split(separator);
    if (before.length > 1 && before.length === after.length && !original.includes(separator === "-" ? "_" : "-")) {
      return after.map((part, index) => preserveReplacementCase(part, before[index]!)).join(separator);
    }
  }
  if (original.toUpperCase() === original && original.toLowerCase() !== original) return value.toUpperCase();
  if (original.toLowerCase() === original) return value.toLowerCase();
  const first = Array.from(value)[0] ?? "", originalFirst = Array.from(original)[0] ?? "";
  const leading = originalFirst.toUpperCase() === originalFirst ? first.toUpperCase() : first.toLowerCase();
  return leading + value.slice(first.length);
}
/** Expand captures with the native RegExp engine; matching remains CodeMirror-owned. */
export function replacementFor(query: SearchQuery, doc: string, range: SearchRange): string {
  if (!query.regexp) return query.replace;
  const regexp = new RegExp(query.search, query.caseSensitive ? "gmu" : "gimu");
  regexp.lastIndex = range.from;
  const match = regexp.exec(doc);
  if (!match || match.index !== range.from || match[0].length !== range.to - range.from) throw new Error("匹配已变化，请重新查找。");
  return query.replace.replace(/\$([$&]|\d+)/g, (literal, key: string) => {
    if (key === "$") return "$";
    if (key === "&") return match[0];
    for (let length = key.length; length > 0; length--) {
      const index = Number(key.slice(0, length));
      if (index > 0 && index < match.length) return (match[index] ?? "") + key.slice(length);
    }
    return literal;
  });
}
