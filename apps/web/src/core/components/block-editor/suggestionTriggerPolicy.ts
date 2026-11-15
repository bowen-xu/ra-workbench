import type { Transaction } from "@tiptap/pm/state";
import {isFixedBlock} from "./blockEditingConstraints";

/** Evaluated before insertion. Dollar formulas may adjoin text or inline atoms;
 * slash/reference menus still require a whitespace or block-start boundary. */
export function canOpenSuggestionMenu(transaction: Transaction, trigger = "/"): boolean {
  const { selection, storedMarks } = transaction;
  if (!selection.empty) return false;
  const { $from } = selection;
  if (isFixedBlock({type: $from.parent.type.name})) return false;
  if (!$from.parent.isTextblock) return false;
  for (let depth = $from.depth; depth > 0; depth--) {
    if ($from.node(depth).type.spec.code) return false;
  }
  if ((storedMarks ?? $from.marks()).some(mark => mark.type.spec.code)) return false;
  if (trigger === "$") return true;
  return $from.parentOffset === 0
    || /^\s$/u.test($from.parent.textBetween($from.parentOffset - 1, $from.parentOffset, "\n", "\uFFFC"));
}

export function isSuggestionQueryAllowed(trigger: string, query: string): boolean {
  if (trigger === "$") return query === "" || query === "$";
  if (trigger === "/") return query.split(" ").length <= 2;
  return true;
}
