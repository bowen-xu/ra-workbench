import { Popover } from "../overlay/OverlayPopup";
import { PopoverDisclosure, PopoverProvider, usePopoverStore, useStoreState } from "@ariakit/react";
import { CornerDownLeft } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useEditorReadOnly } from "./EditorReadOnly";
import { EquationSourceInput, type EquationSourceInputHandle } from "./EquationSourceInput";
import { useBlockHighlight } from "./BlockHighlight";

export function FormulaBlockEditor({ blockId, source, onChange, children }: {
  readonly blockId: string;
  readonly source: string;
  readonly onChange: (source: string) => void;
  readonly children: (source: string) => ReactNode;
}): React.JSX.Element {
  const readOnly = useEditorReadOnly();
  const highlight = useBlockHighlight();
  const trigger = useRef<HTMLButtonElement>(null);
  const input = useRef<EquationSourceInputHandle>(null);
  const [draft, setDraft] = useState(source);
  const popover = usePopoverStore({ placement: "bottom-start",
    setOpen: next => { if (next) setDraft(source); },
  });
  const open = useStoreState(popover, "open");
  const wasOpen = useRef(false);
  const previewSource = open ? draft : source;
  const empty = previewSource.trim().length === 0;
  useEffect(() => {
    // Closing through an outside interaction preserves the block selection,
    // independently of which control receives keyboard focus.
    if (wasOpen.current && !open && !readOnly) highlight.select([blockId]);
    wasOpen.current = open;
  }, [blockId, highlight, open, readOnly]);
  const complete = () => {
    if (readOnly) return;
    onChange(draft);
    highlight.select([blockId]);
    popover.hide();
    trigger.current?.focus({ preventScroll: true });
  };
  useEffect(() => { if (readOnly) popover.hide(); }, [popover, readOnly]);
  useEffect(() => {
    const host = trigger.current?.closest(".block-editor");
    if (host === undefined || host === null || readOnly) return;
    const activate = (event: Event) => {
      if (!(event instanceof KeyboardEvent) || event.key !== "Enter" || event.isComposing || event.keyCode === 229
        || event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return;
      if (event.target instanceof Element && event.target.closest('input, textarea, button, select, [role="dialog"]')) return;
      const selected = highlight.selectedIds();
      if (selected.length !== 1 || selected[0] !== blockId) return;
      event.preventDefault(); event.stopPropagation();
      popover.show();
    };
    host.addEventListener("keydown", activate, true);
    return () => host.removeEventListener("keydown", activate, true);
  }, [blockId, highlight, popover, readOnly]);

  return <PopoverProvider store={popover}>
    <div className="note-formula-block" contentEditable={false}>
      <PopoverDisclosure ref={trigger} store={popover} className="note-formula-disclosure" type="button"
        data-empty={empty}
        // An inner preview SPAN bypasses Tiptap's direct-target input guard;
        // mouseup can focus ProseMirror before Ariakit toggles this popover.
        // Restore the real button first, or Ariakit adopts the whole editor as
        // its disclosure and ignores clicks on other blocks. Avoid scrolling.
        // Regression details: docs/testing/note-formula-popover-regression.md.
        onClickCapture={event => event.currentTarget.focus({ preventScroll: true })}
        disabled={readOnly} aria-label={`${empty ? "空公式" : `公式 ${source}`}，${readOnly ? "只读" : "点击编辑"}`}>
        {empty ? <>
          <span className="note-formula-tex" aria-hidden="true">T<span>E</span>X</span>
          <span>添加 TeX 公式</span>
        </> : children(previewSource)}
      </PopoverDisclosure>
    </div>
    {!readOnly && <Popover store={popover} portal fixed modal={false} unmountOnHide
      gutter={10} fitViewport overflowPadding={12}
      autoFocusOnShow={() => { input.current?.focus(); return false; }}
      // Ariakit restores disclosure focus with browser scrolling by default.
      // Keep its outside-focus ownership, but leave the document viewport still.
      autoFocusOnHide={element => {
        element?.focus({ preventScroll: true });
        return false;
      }}
      hideOnEscape={event => {
        const native = "nativeEvent" in event ? event.nativeEvent : event;
        if (input.current?.ownsTarget(native.target)) return false;
        return !native.isComposing && native.keyCode !== 229;
      }}
      hideOnInteractOutside={event => !input.current?.ownsTarget(event.target)}
      className="note-formula-edit-panel" aria-label="编辑公式" contentEditable={false}>
      <EquationSourceInput inputRef={input} aria-label="公式 LaTeX 源码"
        placeholder="输入 LaTeX 公式，例如 E = mc^2" value={draft}
        onChange={source => { if (!readOnly) setDraft(source); }} onComplete={complete} onCancel={() => popover.hide()} />
      <button className="note-formula-done" type="button" onClick={complete}>
        完成<CornerDownLeft aria-hidden="true" />
      </button>
    </Popover>}
  </PopoverProvider>;
}
