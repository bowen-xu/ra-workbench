import type { ComponentType } from "react";
import { CheckSquare, Code2, Heading1, Heading2, Heading3, Heading4, List, ListCollapse, ListOrdered, Minus, Pilcrow, Quote, Table2 } from "lucide-react";
import { getDefaultSlashMenuItems, insertOrUpdateBlockForSlashMenu, type DefaultSuggestionItem } from "@blocknote/core/extensions";
import type { DefaultReactSuggestionItem } from "@blocknote/react";
import type { baseBlockEditorSchema } from "./baseSchema";
import { buildEquationSuggestionItems } from "./equationSuggestionItems";

export const SAFE_DEFAULT_SLASH_KEYS = [
  "heading", "heading_2", "heading_3", "heading_4", "quote", "numbered_list", "bullet_list",
  "check_list", "toggle_list", "toggle_heading", "toggle_heading_2", "toggle_heading_3", "paragraph", "code_block", "divider", "table",
] as const;
const safeDefaultKeys = new Set<string>(SAFE_DEFAULT_SLASH_KEYS);
const inputBadges: Readonly<Partial<Record<DefaultSuggestionItem["key"], string>>> = {
  heading: "#", heading_2: "##", heading_3: "###", heading_4: "####", quote: '"',
  numbered_list: "1.", bullet_list: "-", check_list: "[]", code_block: "```", divider: "---",
};
const defaultIcons: Partial<Record<DefaultSuggestionItem["key"], ComponentType<{ readonly size?: number }>>> = {
  heading: Heading1, heading_2: Heading2, heading_3: Heading3, heading_4: Heading4, quote: Quote,
  numbered_list: ListOrdered, bullet_list: List, check_list: CheckSquare,
  toggle_list: ListCollapse, toggle_heading: Heading1, toggle_heading_2: Heading2, toggle_heading_3: Heading3,
  paragraph: Pilcrow, code_block: Code2, divider: Minus, table: Table2,
};

export function filterLosslessDefaultSlashItems(items: readonly DefaultSuggestionItem[]): DefaultReactSuggestionItem[] {
  return items.filter(item => safeDefaultKeys.has(item.key)).map(item => {
    const Icon = defaultIcons[item.key] ?? Pilcrow;
    const { badge: _keyboardBadge, ...rest } = item;
    const badge = inputBadges[item.key];
    return { ...rest, ...(badge === undefined ? {} : { badge }), icon: <Icon size={16} /> };
  });
}

type NoteCommands = typeof baseBlockEditorSchema.BlockNoteEditor;
/** A note preset requires the common formula schema; extended host schemas fit
 * structurally without widening their document or persistence types. */
export interface NoteEditingSource {
  readonly schema: {
    readonly blockSchema: { readonly formulaBlock: unknown };
    readonly inlineContentSchema: { readonly inlineFormula: unknown };
  };
}
// Consume only the shared base schema. Confine the pinned upstream's
// incompatible optional-heading generic projection to this library edge.
const defaultItems = getDefaultSlashMenuItems as unknown as (editor: NoteEditingSource) => DefaultSuggestionItem[];
const insert = insertOrUpdateBlockForSlashMenu as unknown as
  (editor: NoteCommands, block: Parameters<NoteCommands["updateBlock"]>[1]) => unknown;

export function createEquationItems(instance: NoteEditingSource): ReturnType<typeof buildEquationSuggestionItems> {
  const editor = instance as NoteCommands;
  return buildEquationSuggestionItems({
    // Inline atoms support a native caret boundary; do not persist spacer text.
    insertInline: () => editor.insertInlineContent([{ type: "inlineFormula", props: { source: "" } }], { updateSelection: true }),
    insertBlock: () => { insert(editor, { type: "formulaBlock", props: { source: "" } }); },
  });
}

/** Portable blocks and formulas. Hosts append domain commands without copying
 * the common registry or importing another host's schema or storage. */
export function buildNoteEditingItems(editor: NoteEditingSource): DefaultReactSuggestionItem[] {
  return [...filterLosslessDefaultSlashItems(defaultItems(editor)), ...createEquationItems(editor)];
}
