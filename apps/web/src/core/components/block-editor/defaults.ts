import { zh } from "@blocknote/core/locales";
import type {BlockNoteEditorOptions, BlockSchema, InlineContentSchema, StyleSchema} from "@blocknote/core";
import {isImageClipboardHTML} from "./ImageClipboard";
import {markdownFormulaHTML, normalizeFormulaHTML} from "../rich-text/formulaClipboard";

type NativePasteContext = Parameters<NonNullable<BlockNoteEditorOptions<BlockSchema, InlineContentSchema, StyleSchema>["pasteHandler"]>>[0];
type PasteContext = Pick<NativePasteContext, "defaultPasteHandler" | "event"> & {
  readonly editor: Pick<NativePasteContext["editor"], "pasteHTML"> & {
    readonly getTextCursorPosition?: () => {readonly block: {readonly type: string}};
  };
};

/** Shared editor defaults; deliberately no schema, document or upload API. */
export const blockEditorDefaults = {
  tables: {headers: true},
  // ProseMirror calls its link handler on mouseup, before the native click.
  // Keep the anchor click for the application resource/browser router instead
  // of letting the editor pre-open a second, unmanaged window.
  links: { onClick: () => false, HTMLAttributes: {draggable: "false"} },
  dictionary: {
    ...zh,
    side_menu: { ...zh.side_menu, drag_handle_label: "拖动块；点击打开块菜单" },
  },
  dropCursor: { color: "#3f6d50", width: 3 },
  // Browser HTML is authoritative when both representations are available.
  pasteHandler: ({defaultPasteHandler, editor, event}: PasteContext) => {
    const html = event.clipboardData?.getData("text/html") ?? "";
    if (isImageClipboardHTML(html)) {
      // Standard HTML is all the system clipboard may retain. Preserve the
      // block schema and resource reference, but let native paste hooks reset
      // copied block identities before the first transaction reaches HBS.
      // Do not call pasteHTML from a capture listener or synthesize a paste.
      editor.pasteHTML(html, true);
      return true;
    }
    const data = event.clipboardData, types = data?.types ?? [];
    const code = editor.getTextCursorPosition?.().block.type === "codeBlock";
    // Native BlockNote fragments, files and IDE code keep their existing paste
    // semantics. Literal Cmd/Ctrl+Shift+V bypasses this handler altogether.
    if (!code && !types.includes("blocknote/html") && !types.includes("vscode-editor-data") && !types.includes("Files")) {
      const converted = html ? normalizeFormulaHTML(html, document)
        : markdownFormulaHTML(data?.getData("text/markdown") || data?.getData("text/plain") || "");
      if (converted !== null) {editor.pasteHTML(converted, true); return true;}
    }
    return defaultPasteHandler({prioritizeMarkdownOverHTML: false});
  },
};
