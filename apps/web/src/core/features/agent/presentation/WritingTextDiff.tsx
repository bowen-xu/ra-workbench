import { useEffect, useRef, useState } from "react";
import { EditorState, Prec, Text } from "@codemirror/state";
import { defaultKeymap } from "@codemirror/commands";
import { EditorView, keymap, lineNumbers, highlightSpecialChars } from "@codemirror/view";
import { searchKeymap } from "@codemirror/search";
import { unifiedMergeView, getChunks } from "@codemirror/merge";
import { EditorDiffHighlight, type EditorDiffHighlightAppearance } from "../../../public/editorReview";

interface Props {
  readonly appearance?: EditorDiffHighlightAppearance;
  readonly before: string;
  readonly after: string;
  readonly showNavigation?: boolean;
  readonly showSummary?: boolean;
}

/** Frozen historical comparison only. Editing and review commands belong to LatexEditor. */
export function WritingTextDiff({ before, after, showNavigation = true, showSummary = true, appearance }: Props): React.JSX.Element {
  const host = useRef<HTMLDivElement>(null);
  const editor = useRef<EditorView | null>(null);
  const [summary, setSummary] = useState({ count: 0, precise: true });
  useEffect(() => {
    if (!host.current) return;
    const view = new EditorView({ parent: host.current, state: EditorState.create({
      doc: Text.of(after.split("\n")),
      extensions: [EditorState.readOnly.of(true), EditorView.editable.of(false),
        EditorState.lineSeparator.of("\n"), EditorView.contentAttributes.of({ "aria-label": "逐行差异（只读）", tabindex: "0" }),
        Prec.high(EditorView.domEventHandlers({ copy: (event, currentView) => copyDeletedSelection(event, currentView) })),
        lineNumbers(), highlightSpecialChars(), EditorView.lineWrapping,
        keymap.of([...defaultKeymap, ...searchKeymap]),
        unifiedMergeView({ original: Text.of(before.split("\n")), mergeControls: false,
          syntaxHighlightDeletions: false, allowInlineDiffs: false, diffConfig: { scanLimit: 1000, timeout: 50 } }),
        EditorView.updateListener.of(update => {
          const chunks = getChunks(update.state)?.chunks ?? [];
          setSummary({ count: chunks.length, precise: chunks.every(chunk => chunk.precise) });
        }),
      ],
    }) });
    editor.current = view;
    const chunks = getChunks(view.state)?.chunks ?? [];
    setSummary({ count: chunks.length, precise: chunks.every(chunk => chunk.precise) });
    return () => { editor.current = null; view.destroy(); };
  }, [before]);
  useEffect(() => {
    const view = editor.current;
    if (!view || view.state.doc.toString() === after) return;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: Text.of(after.split("\n")) } });
  }, [after]);
  function move(direction: "next" | "previous"): void {
    const view = editor.current;
    if (!view) return;
    const chunks = getChunks(view.state)?.chunks ?? [];
    if (!chunks.length) return;
    const cursor = view.state.selection.main.head;
    const currentIndex = chunks.findIndex(chunk => chunk.fromB <= cursor && cursor <= chunk.endB);
    const targetIndex = direction === "next"
      ? currentIndex >= 0 ? currentIndex + 1 : chunks.findIndex(chunk => chunk.fromB > cursor)
      : currentIndex >= 0 ? currentIndex - 1 : chunks.reduce((last, chunk, index) => chunk.endB < cursor ? index : last, -1);
    if (targetIndex < 0 || targetIndex >= chunks.length) return;
    const target = chunks[targetIndex];
    if (!target) return;
    view.dispatch({ selection: { anchor: target.fromB }, scrollIntoView: true });
    view.focus();
  }
  return <EditorDiffHighlight appearance={appearance}><section className="writing-text-diff" aria-label="逐行修改比较">
    {showSummary && <p><span className="writing-diff-removed">− 删除内容</span> · <span className="writing-diff-added">+ 新增内容</span> · {summary.count} 处变更</p>}
    {!summary.precise && <p role="status">差异较复杂，当前按较大范围分组。完整内容未省略。</p>}
    {before === after && <p>文本内容相同。</p>}
    {before.endsWith("\n") !== after.endsWith("\n") && <p>文件末尾换行：{before.endsWith("\n") ? "有" : "无"} → {after.endsWith("\n") ? "有" : "无"}</p>}
    {showNavigation && <div><button type="button" disabled={!summary.count} onClick={() => move("previous")}>上一处变更</button>
      <button type="button" disabled={!summary.count} onClick={() => move("next")}>下一处变更</button></div>}
    <div ref={host} />
  </section></EditorDiffHighlight>;
}

function copyDeletedSelection(event: Event, view: EditorView): boolean {
  const selection = view.dom.ownerDocument.getSelection();
  const anchor = selection?.anchorNode;
  const element = anchor instanceof Element ? anchor : anchor?.parentElement;
  if (!selection || selection.isCollapsed || !element?.closest(".cm-deletedChunk")) return false;
  const text = selection.toString();
  if (!text) return false;
  const clipboard = (event as ClipboardEvent).clipboardData;
  if (!clipboard) return false;
  clipboard.setData("text/plain", text);
  event.preventDefault();
  return true;
}
