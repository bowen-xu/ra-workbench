import { diff } from "@codemirror/merge";

import type { HbsTextPolicy, TextDiff } from "@research-assistant/core/hbs";
import { ChangeSet, Text } from "@codemirror/state";

/** Default strategy: directly compare texts with the existing Myers matcher.
 * Numeric tokens stay atomic so competing numeric edits remain conflicts. */
export const directTextDiff: TextDiff = (before, after) => {
  const tokens = (text: string) => [...text.matchAll(/\d+(?:\.\d+)?%?|[\s\S]/gu)].map(match => ({ value: match[0], from: match.index }));
  const left = tokens(before), right = tokens(after), dictionary = new Map<string, string>();
  const encode = (values: typeof left) => values.map(token => {
    let code = dictionary.get(token.value);
    if (!code) { code = String.fromCharCode(dictionary.size + 1); dictionary.set(token.value, code); }
    return code;
  }).join("");
  return diff(encode(left), encode(right), { scanLimit: 2_000 }).map(change => ({
    fromA: left[change.fromA]?.from ?? before.length, toA: left[change.toA]?.from ?? before.length,
    fromB: right[change.fromB]?.from ?? after.length, toB: right[change.toB]?.from ?? after.length,
  }));
};

/** Default adapter uses the same pinned diff/position engine as the frozen reference. */
export function createCodeMirrorHbsPolicy(calculate: TextDiff = directTextDiff): HbsTextPolicy {
  return { calculate, map: (before, after, changes) => ChangeSet.of(changes.map(change => ({
    from: change.fromA, to: change.toA, insert: Text.of(after.slice(change.fromB, change.toB).split("\n")),
  })), before.length) };
}
