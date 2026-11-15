import { useLayoutEffect, useRef, type MouseEvent, type ReactNode } from "react";
import "./StructuredFieldTextEditor.css";

/** Capture the browser's character hit test before replacing the paragraph. */
export function structuredFieldCaretOffset(event: MouseEvent<HTMLElement>): number | undefined {
  const host = event.currentTarget, owner = host.ownerDocument;
  const position = owner.caretPositionFromPoint?.(event.clientX, event.clientY);
  const hit = position ? undefined : owner.caretRangeFromPoint?.(event.clientX, event.clientY);
  const node = position?.offsetNode ?? hit?.startContainer;
  const offset = position?.offset ?? hit?.startOffset;
  if (!node || offset === undefined || !host.contains(node)) return undefined;
  const range = owner.createRange();
  range.selectNodeContents(host); range.setEnd(node, offset);
  return range.toString().length;
}

/** The paragraph owns wrapping and height; the native input shares its exact
 * geometry without textarea chrome or a second scroll/resize surface. */
export function StructuredFieldTextEditor({ label, value, disabled, initialCaret, inlineDiff=false, children, onChange, onCommit, onHistory }: {
  readonly label: string;
  readonly value: string;
  readonly disabled: boolean;
  readonly initialCaret?: number;
  readonly inlineDiff?: boolean;
  readonly children?: ReactNode;
  readonly onChange: (value: string) => void;
  readonly onCommit: (relatedTarget?: EventTarget | null) => void | Promise<void>;
  readonly onHistory?: (direction: "undo" | "redo") => void;
}): React.JSX.Element {
  const input = useRef<HTMLTextAreaElement>(null);
  const finishing = useRef(false);
  const finish = async () => {
    if (finishing.current) return;
    finishing.current = true;
    try { await onCommit(); }
    finally { finishing.current = false; }
  };
  useLayoutEffect(() => {
    const editor = input.current;
    editor?.focus({ preventScroll: true });
    if (editor && initialCaret !== undefined) {
      const offset = Math.min(editor.value.length, Math.max(0, initialCaret));
      editor.setSelectionRange(offset, offset);
    }
  }, []);
  const content=<>{children??value}{value.endsWith("\n") ? "\u200b" : ""}</>;
  return <div className="structured-field-inline-editor" data-inline-diff={inlineDiff||undefined}>
    {inlineDiff?<div aria-hidden="true" className="structured-field-inline-mirror">{content}</div>
      :<p aria-hidden="true" className="structured-field-inline-mirror">{content}</p>}
    <textarea ref={input} aria-label={label} value={value} disabled={disabled} rows={1}
      onChange={event => onChange(event.target.value)}
      onBlur={event => { if (!finishing.current) onCommit(event.relatedTarget); }}
      onKeyDown={event => {
        if (event.nativeEvent.isComposing) return;
        const key=event.key.toLowerCase();
        if (onHistory && (event.metaKey || event.ctrlKey) && !event.altKey && (key === "z" || event.ctrlKey && key === "y")) {
          event.preventDefault(); event.stopPropagation(); onHistory(key === "y" || event.shiftKey ? "redo" : "undo");
        } else if (event.key === "Escape") {
          event.preventDefault(); event.stopPropagation(); void finish();
        } else if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
          event.preventDefault(); event.stopPropagation(); void finish();
        }
      }} />
  </div>;
}
