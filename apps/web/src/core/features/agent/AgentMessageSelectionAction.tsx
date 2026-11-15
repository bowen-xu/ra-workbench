import { useEffect, useRef, useState, type RefObject } from "react";
import { usePopoverStore } from "@ariakit/react";
import { Popover } from "../../components/overlay/OverlayPopup";
import { AnnotationCommentEditor } from "./AnnotationCommentEditor";
import "./AgentMessageSelectionAction.css";

interface CapturedSelection {
  readonly text: string;
  readonly rect: DOMRect;
  readonly range?: Range;
  readonly messageId: string;
  readonly scope: string;
}
interface PendingComment { readonly text: string; readonly comment: string; readonly messageId: string; }
// Hosts unmount their chat panel on conversation switches. Keep unfinished
// comments in memory by actual conversation identity, never in another draft.
const pendingComments = new Map<string, PendingComment>();
function hasStableScope(scope: string): boolean {
  try {
    const identity: unknown = JSON.parse(scope);
    return Array.isArray(identity) && typeof identity[0] === "string" && identity[0].trim().length > 0;
  } catch {return false;}
}

/** Select text in an actual message, including optimistic message bubbles. */
export function AgentMessageSelectionAction({ root, enabled, scope, onAdd }: {
  readonly root: RefObject<HTMLElement | null>;
  readonly enabled: boolean;
  readonly scope: string;
  readonly onAdd: (text: string, comment: string, options: {readonly focusComposer: boolean}) => void | Promise<void>;
}): React.JSX.Element | null {
  const [selection, setSelection] = useState<CapturedSelection | null>(null);
  const [editing, setEditing] = useState(false);
  const currentOwner = useRef({scope, enabled});
  currentOwner.current = {scope, enabled};
  const localComments = useRef(new Map<string, PendingComment>());
  const drafts = hasStableScope(scope) ? pendingComments : localComments.current;
  const editingRef = useRef(false);
  const selecting = useRef(false);
  const keyboardSelecting = useRef(false);
  const popover = usePopoverStore({ open: enabled && selection !== null && selection.scope === scope,
    setOpen: open => { if (!open && !editingRef.current) setSelection(null); }, placement: "top" });
  useEffect(() => {
    editingRef.current = false;
    selecting.current = false;
    keyboardSelecting.current = false;
    const saved = enabled ? drafts.get(scope) : undefined;
    setEditing(!!saved);
    setSelection(saved ? {text: saved.text, scope, messageId: saved.messageId, rect: new DOMRect()} : null);
    editingRef.current = !!saved;
    if (!enabled) return;
    const update = () => {
      if (editingRef.current || selecting.current || keyboardSelecting.current) return;
      const selected = window.getSelection();
      if (!selected || selected.isCollapsed || selected.rangeCount !== 1) { setSelection(null); return; }
      const range = selected.getRangeAt(0);
      const start = range.startContainer instanceof Element ? range.startContainer : range.startContainer.parentElement;
      const end = range.endContainer instanceof Element ? range.endContainer : range.endContainer.parentElement;
      const bubble = start?.closest<HTMLElement>("[data-selectable-text]");
      const body = start?.closest<HTMLElement>(".writing-agent-markdown, .agent-user-message-text > p");
      const collapsed = body?.closest<HTMLElement>('.agent-user-message-text[data-collapsed="true"]');
      if (!bubble || bubble !== end?.closest("[data-selectable-text]") || !root.current?.contains(bubble)
        || !body || body !== end?.closest(".writing-agent-markdown, .agent-user-message-text > p")
        || !!collapsed && collapsed.scrollHeight > collapsed.clientHeight + 1
        || !bubble.closest("[data-agent-message-id]") || start?.closest("button, [data-agent-message-actions]")
        || end?.closest("button, [data-agent-message-actions]")
        || Array.from(body.querySelectorAll<HTMLElement>("*")).some(element => {
          if (!range.intersectsNode(element)) return false;
          if (element.matches('button, [data-agent-message-actions], [hidden], [aria-hidden="true"]')) return true;
          const style = window.getComputedStyle(element);
          const opacity = Number.parseFloat(style.opacity);
          return style.display === "none" || style.visibility === "hidden" || Number.isFinite(opacity) && opacity < 1;
        })) {
        setSelection(null); return;
      }
      const messageId = bubble.closest<HTMLElement>("[data-agent-message-id]")?.dataset.agentMessageId;
      const text = selected.toString();
      if (!text.trim() || !messageId) { setSelection(null); return; }
      const rect = range.getBoundingClientRect();
      setSelection({ text, rect, range: range.cloneRange(), messageId, scope });
    };
    const clear = () => { if (!editingRef.current) setSelection(null); };
    const startSelection = (event: PointerEvent) => {
      if (editingRef.current || event.button !== 0) return;
      selecting.current = true;
      setSelection(null);
    };
    const endSelection = () => { if (selecting.current) { selecting.current = false; update(); } };
    const cancelSelection = () => { selecting.current = false; };
    const startKeyboardSelection = (event: KeyboardEvent) => {
      if (editingRef.current || !event.shiftKey || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(event.key)) return;
      keyboardSelecting.current = true;
      setSelection(null);
    };
    const endKeyboardSelection = (event: KeyboardEvent) => {
      if (keyboardSelecting.current && !event.shiftKey) { keyboardSelecting.current = false; update(); }
    };
    const stopSelection = () => {selecting.current = false; keyboardSelecting.current = false; clear();};
    const target = root.current;
    target?.addEventListener("pointerdown", startSelection);
    document.addEventListener("pointerup", endSelection);
    document.addEventListener("pointercancel", cancelSelection);
    document.addEventListener("keydown", startKeyboardSelection);
    document.addEventListener("keyup", endKeyboardSelection);
    document.addEventListener("selectionchange", update);
    window.addEventListener("scroll", clear, true);
    window.addEventListener("resize", clear);
    window.addEventListener("blur", stopSelection);
    return () => {
      target?.removeEventListener("pointerdown", startSelection);
      document.removeEventListener("pointerup", endSelection);
      document.removeEventListener("pointercancel", cancelSelection);
      document.removeEventListener("keydown", startKeyboardSelection);
      document.removeEventListener("keyup", endKeyboardSelection);
      document.removeEventListener("selectionchange", update);
      window.removeEventListener("scroll", clear, true);
      window.removeEventListener("resize", clear);
      window.removeEventListener("blur", stopSelection);
    };
  }, [enabled, root, scope, drafts]);
  if (!enabled || !selection || selection.scope !== scope) return null;
  const finish = async (comment: string, focusComposer: boolean) => {
    if (!currentOwner.current.enabled || currentOwner.current.scope !== selection.scope) return;
    const pending = drafts.get(selection.scope);
    await onAdd(selection.text, comment, {focusComposer});
    if (drafts.get(selection.scope) === pending) drafts.delete(selection.scope);
    if (!currentOwner.current.enabled || currentOwner.current.scope !== selection.scope) return;
    editingRef.current = false;
    setEditing(false);
    setSelection(null);
    window.getSelection()?.removeAllRanges();
  };
  const anchor = () => {
    if (!editing) return selection.rect;
    if (selection.range?.startContainer.isConnected) return selection.range.getBoundingClientRect();
    const message = Array.from(root.current?.querySelectorAll<HTMLElement>("[data-agent-message-id]") ?? [])
      .find(item => item.dataset.agentMessageId === selection.messageId);
    return message?.getBoundingClientRect() ?? root.current?.getBoundingClientRect() ?? selection.rect;
  };
  return <Popover store={popover} getAnchorRect={anchor} gutter={8} fitViewport modal={false}
    autoFocusOnShow={false} autoFocusOnHide={false} hideOnEscape={false} hideOnInteractOutside={false} className={`agent-message-selection-action${editing ? " is-editing" : ""}`}>
    {editing ? <AnnotationCommentEditor mode="create" title="对话选段" initialComment={drafts.get(scope)?.comment ?? ""}
      onCommentChange={comment => drafts.set(selection.scope, {text: selection.text, messageId: selection.messageId, comment})}
      onSave={comment => finish(comment, true)} onDismiss={comment => finish(comment, false)}
      onCancel={() => {drafts.delete(selection.scope); editingRef.current = false; setEditing(false); setSelection(null);}} />
      : <button type="button" onMouseDown={event => event.preventDefault()} onClick={() => {drafts.set(scope, {text: selection.text, messageId: selection.messageId, comment: ""}); editingRef.current = true; setEditing(true);}}>添加批注</button>}
  </Popover>;
}
