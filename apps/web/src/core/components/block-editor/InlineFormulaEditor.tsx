import { Popover } from "../overlay/OverlayPopup";
import { PopoverDisclosure, PopoverProvider, usePopoverStore, useStoreState } from "@ariakit/react";
import { CornerDownLeft } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { EditorView } from "@tiptap/pm/view";
import { useEditorReadOnly } from "./EditorReadOnly";
import { EquationSourceInput, type EquationSourceInputHandle } from "./EquationSourceInput";
import "./InlineFormulaEditor.css";

/** Atomic inline math stays in the text flow; source editing uses a portal. */
export function InlineFormulaEditor({ source, getView, onChange, onDelete, onFinish, onMoveOut, emptyPreview, children }: {
  readonly source: string;
  readonly getView: () => EditorView;
  readonly onChange: (source: string) => void;
  readonly onDelete: (element: HTMLButtonElement) => void;
  readonly onFinish: () => void;
  readonly onMoveOut: (element: HTMLButtonElement, side: "left" | "right") => void;
  readonly emptyPreview: ReactNode;
  readonly children: (source: string) => ReactNode;
}): React.JSX.Element {
  const readOnly = useEditorReadOnly();
  const trigger = useRef<HTMLButtonElement>(null);
  const input = useRef<EquationSourceInputHandle>(null);
  const selectSourceOnOpen = useRef(false);
  const [draft, setDraft] = useState(source);
  const isEmpty = source.trim() === "";
  const discarded = useRef(false);
  const discardEmpty = () => {
    const element = trigger.current;
    if (readOnly || discarded.current || !isEmpty || element === null || !element.isConnected) return;
    discarded.current = true;
    onDelete(element);
  };
  const popover = usePopoverStore({ placement: "bottom-start", defaultOpen: isEmpty && !readOnly,
    setOpen: open => {
      if (open) { setDraft(source); discarded.current = false; }
      else discardEmpty();
    },
  });
  const open = useStoreState(popover, "open");
  const previewSource = open ? draft : source;
  const previewEmpty = previewSource.trim() === "";
  useEffect(() => { if (readOnly) popover.hide(); }, [popover, readOnly]);
  useEffect(() => {
    const host = trigger.current?.closest(".block-editor");
    if (!host || readOnly) return;
    const enter = (event: Event) => {
      if (!(event instanceof KeyboardEvent) || (event.key !== "ArrowLeft" && event.key !== "ArrowRight")
        || event.isComposing || event.keyCode === 229 || event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return;
      const view = getView();
      if (!view.editable || view.composing || !(event.target instanceof Element) || !view.dom.contains(event.target)
        || event.target.closest('input, textarea, button, select, [contenteditable="false"]')) return;
      // Native arrow movement can precede ProseMirror's selectionchange update.
      // Read the live DOM caret so rapid consecutive keys do not skip an atom.
      const domSelection = view.dom.ownerDocument.getSelection();
      let caret = view.state.selection.from;
      if (domSelection?.anchorNode && view.dom.contains(domSelection.anchorNode)) {
        if (!domSelection.isCollapsed) return;
        caret = view.posAtDOM(domSelection.anchorNode, domSelection.anchorOffset);
      } else if (!view.state.selection.empty) return;
      const resolved = view.state.doc.resolve(caret);
      const node = event.key === "ArrowLeft" ? resolved.nodeBefore : resolved.nodeAfter;
      if (node?.type.name !== "inlineFormula") return;
      const wrapper = trigger.current?.closest(".bn-inline-content-section");
      if (!wrapper || !view.dom.contains(wrapper)) return;
      const position = event.key === "ArrowLeft" ? caret - node.nodeSize : caret;
      if (view.posAtDOM(wrapper, 0) !== position) return;
      event.preventDefault(); event.stopImmediatePropagation();
      selectSourceOnOpen.current = true;
      popover.show();
    };
    host.addEventListener("keydown", enter, true);
    return () => host.removeEventListener("keydown", enter, true);
  }, [getView, popover, readOnly]);
  const complete = () => {
    if (readOnly) return;
    onChange(draft);
    if (draft.trim() === "" && trigger.current) {
      discarded.current = true;
      onDelete(trigger.current);
    }
    popover.hide();
    onFinish();
  };
  const dismiss = () => {
    popover.hide();
    if (!readOnly && isEmpty) onFinish();
    else trigger.current?.focus({ preventScroll: true });
  };

  return <PopoverProvider store={popover}>
    <PopoverDisclosure ref={trigger} store={popover} type="button" contentEditable={false}
      className={`inline-equation-preview${previewEmpty ? " empty" : ""}`} disabled={readOnly}
      aria-label={`${isEmpty ? "空行内公式" : `行内公式 ${source}`}，${readOnly ? "只读" : "点击编辑"}`}>
      {previewEmpty ? <><span className="inline-equation-placeholder-symbol" aria-hidden="true">{emptyPreview}</span><span>新公式</span></> : children(previewSource)}
    </PopoverDisclosure>
    {!readOnly && <Popover store={popover} portal fixed modal={false} unmountOnHide fitViewport
      gutter={6} overflowPadding={12} autoFocusOnHide={false}
      autoFocusOnShow={() => {
        input.current?.focus();
        if (selectSourceOnOpen.current) { input.current?.select(); selectSourceOnOpen.current = false; }
        return false;
      }}
      hideOnEscape={event => {
        const native = "nativeEvent" in event ? event.nativeEvent : event;
        if (input.current?.ownsTarget(native.target)) return false;
        if (!native.isComposing && native.keyCode !== 229) dismiss();
        return false;
      }}
      hideOnInteractOutside={event => !input.current?.ownsTarget(event.target)}
      aria-label="编辑行内公式" className="inline-equation-editor" contentEditable={false}>
      <EquationSourceInput inputRef={input} aria-label="行内公式 LaTeX 源码"
        placeholder="E = mc^2" value={draft}
        onChange={source => { if (!readOnly) setDraft(source); }} onComplete={complete} onCancel={dismiss}
        onMoveOut={side => {
          if (!trigger.current) return;
          // Map the document boundary before deleting an unconfirmed empty atom.
          onMoveOut(trigger.current, side);
          popover.hide(); onFinish();
        }} />
      <button type="button" className="inline-equation-done" onClick={complete}>完成<CornerDownLeft aria-hidden="true" /></button>
    </Popover>}
  </PopoverProvider>;
}
