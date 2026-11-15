import { editorSearchScope } from "./editorSearchBehavior";
import { Compartment, EditorState, Text } from "@codemirror/state";
import { defaultKeymap, history, historyField, historyKeymap } from "@codemirror/commands";
import { defaultHighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { getOriginalDoc, unifiedMergeView } from "@codemirror/merge";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { drawSelection, EditorView, keymap, lineNumbers, type ViewUpdate } from "@codemirror/view";
import { chunkActionDecorations } from "../editor-diff/codeMirrorChunkActions";
import { resolveTextChunk } from "../editor-diff/textDiffCommands";
import { createEditorSearchPanel } from "./editorSearchPanel";
import { hostReviewHistory, textReviewBaseline } from "../editor-diff/textReviewBaseline";
import { editorActionHistory } from "./textEditorHistory";
import { allowsReadOnlyHistoryTransaction } from "./readOnlyEditorHistory";

export function createTextEditorState(options: {
  readonly content: string;
  readonly baseline?: string;
  readonly readOnly?: boolean;
  readonly readOnlyHistory?: boolean;
  readonly presentation?: "document" | "prose";
  readonly path: string;
  readonly onChange: (content: string, baseline: string) => void;
  readonly isReviewDisabled?: () => boolean;
  readonly reviewDisabled?: boolean;
  readonly reviewCompartment?: Compartment;
  readonly onResolveReview?: (view: EditorView, position: number, action: "keep" | "undo") => void;
  readonly onUpdate?: (update: ViewUpdate) => void;
  readonly onHistoryRestore?: (update: ViewUpdate) => boolean;
  readonly historyEnabled?: boolean;
  /** Retain native history/selection only; all listeners and view callbacks are
   * rebuilt for the newly mounted production host. */
  readonly retainedState?: EditorState;
}): EditorState {
  const historyOnly=options.readOnly === true && options.readOnlyHistory === true && options.historyEnabled === true;
  const retained=options.retainedState;
  return EditorState.create({ doc: retained?.doc??Text.of(options.content.split("\n")),
    ...(retained?{selection:retained.selection}:{}), extensions: [
    EditorState.readOnly.of(options.readOnly === true && !historyOnly), EditorView.editable.of(!options.readOnly),
    ...(historyOnly ? [EditorState.transactionFilter.of(transaction=>allowsReadOnlyHistoryTransaction(transaction) ? transaction : [])] : []),
    ...(options.presentation === "prose" ? [] : [lineNumbers()]), history(), drawSelection(), EditorView.lineWrapping,
    ...(retained?[historyField.init(()=>retained.field(historyField))]:[]),
    EditorState.phrases.of({ Find: "查找", Replace: "替换为", next: "下一处", previous: "上一处", all: "全选匹配", "match case": "区分大小写", regexp: "正则表达式", "by word": "全字匹配", replace: "替换", "replace all": "全部替换", close: "关闭查找" }),
    editorSearchScope,
    search({ createPanel: createEditorSearchPanel, literal: true }),
    highlightSelectionMatches(), keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap]),
    syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
    textReviewBaseline, ...(options.historyEnabled ? [editorActionHistory, hostReviewHistory.of(true)] : []),
    unifiedMergeView({ original: retained?getOriginalDoc(retained):Text.of((options.baseline ?? options.content).split("\n")), mergeControls: false,
      gutter: options.presentation !== "prose",
      syntaxHighlightDeletions: false, allowInlineDiffs: false, diffConfig: { scanLimit: 1000, timeout: 50 } }),
    (options.reviewCompartment ?? new Compartment()).of(chunkActionDecorations((view, position, action) => {
      if (options.isReviewDisabled?.()) return;
      if (options.onResolveReview) options.onResolveReview(view, position, action);
      else resolveTextChunk(view, position, action);
    }, options.reviewDisabled === true || options.readOnly === true)),
    EditorView.updateListener.of(update => {
      const restored = options.onHistoryRestore?.(update);
      if (!restored && (update.docChanged || !getOriginalDoc(update.startState).eq(getOriginalDoc(update.state)))) {
        options.onChange(update.state.doc.toString(), getOriginalDoc(update.state).toString());
      }
      options.onUpdate?.(update);
    }),
    EditorView.contentAttributes.of({ "aria-label": options.path, ...(options.readOnly ? {"aria-readonly":"true"} : {}), spellcheck: "false" }),
    EditorView.theme({ "&": { height: options.presentation === "prose" ? "auto" : "100%" }, ".cm-scroller": { overflow: "auto", fontFamily: options.presentation === "prose" ? "inherit" : '"SFMono-Regular", Consolas, monospace', fontSize: options.presentation === "prose" ? "inherit" : "var(--ui-font-body)", lineHeight: options.presentation === "prose" ? "inherit" : "1.65" },
      ".cm-content": { padding: options.presentation === "prose" ? "0" : "18px 0" }, ".cm-gutters": { background: "#f1f0ea", color: "#7c897e", borderRight: "1px solid #dce1d7" },
      "&.cm-focused": { outline: "none" }, ".cm-deletedChunk:empty": { display: "none" } }),
  ] });
}
