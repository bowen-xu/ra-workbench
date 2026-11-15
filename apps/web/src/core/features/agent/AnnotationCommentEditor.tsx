import { useNotificationState } from "../../services/useNotificationState";
import { useEffect, useRef, useState } from "react";
import { ArrowUp, Check, Trash2 } from "lucide-react";
import { ControlHint } from "../../components/ControlHint";
import { PaperAiReadingSpeechInput } from "../speech/PaperAiReadingSpeechInput";
import type { SpeechInputState } from "../speech/speechRecognition";
import "./AgentEvidenceReferences.css";
import { useAnnotationCommentLayout } from "./useAnnotationCommentLayout";

/** Comment text stays local until its owner acknowledges Add, Send or Save. */
export function AnnotationCommentEditor({title, initialComment = "", onSave, onSend, onCancel, onDismiss, onRemove, onCommentChange, disabled = false, mode = "edit"}: {
  readonly title: string; readonly initialComment?: string; readonly disabled?: boolean;
  readonly mode?: "create" | "edit";
  readonly onSave: (comment: string) => void | Promise<void>; readonly onCancel: () => void;
  readonly onSend?: (comment: string) => void | Promise<void>;
  readonly onDismiss?: (comment: string) => void | Promise<void>;
  readonly onRemove?: () => void | Promise<void>;
  readonly onCommentChange?: (comment: string) => void;
}) {
  const [comment, setComment] = useState(initialComment), [busy, setBusy] = useState(false), [, setError] = useNotificationState("error");
  const [sendModifier, setSendModifier] = useState(false), [speech, setSpeech] = useState<SpeechInputState>("idle");
  const input = useRef<HTMLTextAreaElement>(null), committing = useRef(false);
  const editor = useRef<HTMLDivElement>(null), internalPointer = useRef(false), dismissed = useRef(false);
  const creating = mode === "create", hasComment = !!comment.trim(), speaking = speech !== "idle";
  const multiline = useAnnotationCommentLayout(input, creating, comment);
  useEffect(() => {
    const modifier = (event: KeyboardEvent) => setSendModifier(event.metaKey || event.ctrlKey);
    const clear = () => setSendModifier(false);
    window.addEventListener("keydown", modifier); window.addEventListener("keyup", modifier); window.addEventListener("blur", clear);
    return () => {window.removeEventListener("keydown", modifier); window.removeEventListener("keyup", modifier); window.removeEventListener("blur", clear);};
  }, []);
  function change(value: string) {setComment(value); onCommentChange?.(value);}
  async function commit(action: () => void | Promise<void>) {
    if (committing.current || disabled || speaking) return false;
    committing.current = true; setBusy(true); setError("");
    try {await action(); return true;} catch (reason) {setError(reason instanceof Error ? reason.message : "批注暂未保存，请重试。"); return false;}
    finally {committing.current = false; setBusy(false);}
  }
  async function dismiss() {
    if (!creating || !onDismiss || dismissed.current) return;
    dismissed.current = true;
    if (!await commit(() => onDismiss(comment))) dismissed.current = false;
  }
  useEffect(() => {
    if (!creating || !onDismiss) return;
    const outside = (event: PointerEvent) => {
      if (event.button !== 0 || event.target instanceof Node && editor.current?.contains(event.target)) return;
      internalPointer.current = false; void dismiss();
    };
    document.addEventListener("pointerdown", outside, true);
    return () => document.removeEventListener("pointerdown", outside, true);
  }, [creating, onDismiss, comment, disabled, speaking]);
  const voice = <PaperAiReadingSpeechInput tooltipClassName="agent-evidence-hint" value={comment} onChange={change} onStateChange={setSpeech} disabled={busy || disabled}/>;
  const send = sendModifier && onSend && hasComment;
  const hint = <div className="annotation-comment-hint"><span>添加批注 <kbd>↵</kbd></span>{onSend && <span>发送 <kbd>⌘ ↵</kbd></span>}</div>;
  return <div ref={editor} className={`annotation-comment-editor${creating ? " is-create" : ""}${multiline ? " is-multiline" : ""}`}
    onKeyDown={event => {
      if (event.key !== "Escape" || event.nativeEvent.isComposing || event.keyCode === 229) return;
      event.preventDefault(); event.stopPropagation();
      if (!busy) {if (creating && onDismiss) void dismiss(); else onCancel();}
    }}
    onPointerDownCapture={() => {internalPointer.current = true;}}
    onPointerUp={() => {internalPointer.current = false;}} onPointerCancel={() => {internalPointer.current = false;}}
    onBlur={event => {if (!internalPointer.current && !event.currentTarget.contains(event.relatedTarget)) void dismiss();}}>
    <div className="annotation-comment-input-row">
      <textarea ref={input} rows={creating ? 1 : 3} aria-label={`批注 ${title}`} placeholder="添加可选评论…" value={comment} disabled={busy || disabled} onChange={event => change(event.target.value)} autoFocus
        onKeyDown={event => {
          if (event.nativeEvent.isComposing || event.keyCode === 229) return;
          if ((event.metaKey || event.ctrlKey) && ["z", "y"].includes(event.key.toLowerCase())) {event.stopPropagation(); return;}
          if (event.key !== "Enter" || event.shiftKey) return;
          event.preventDefault(); event.stopPropagation();
          const sending = creating && (event.metaKey || event.ctrlKey) && onSend;
          if (sending && !hasComment) return;
          void commit(() => sending ? onSend!(comment) : onSave(comment));
        }}/>
      {creating && <div className="annotation-comment-primary">
        <span hidden={hasComment && !speaking && !multiline}>{voice}</span>
        {hasComment && !speaking && <ControlHint tooltipClassName="agent-evidence-hint" content={hint}><button type="button" className="annotation-comment-commit" aria-label={send ? "发送批注问题" : "添加批注"} disabled={busy || disabled}
          onClick={() => void commit(() => send ? onSend!(comment) : onSave(comment))}>{send ? <ArrowUp aria-hidden="true"/> : <Check aria-hidden="true"/>}</button></ControlHint>}
      </div>}
    </div>

    {!creating && <footer className="agent-evidence-edit-actions">
      {onRemove && <ControlHint tooltipClassName="agent-evidence-hint" content="删除批注"><button type="button" className="agent-evidence-icon-action" aria-label={`移除批注 ${title}`} disabled={busy || disabled || speaking} onClick={() => void commit(onRemove)}><Trash2 aria-hidden="true"/></button></ControlHint>}
      <div>{voice}<button type="button" className="agent-evidence-cancel" disabled={busy || disabled} onClick={onCancel}>取消</button>
        <button type="button" className="agent-evidence-save" disabled={busy || disabled || speaking} onClick={() => void commit(() => onSave(comment))}>保存</button></div>
    </footer>}
  </div>;
}
